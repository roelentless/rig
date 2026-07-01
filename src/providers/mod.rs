pub mod makefile;
pub mod rig;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::config::{try_load_config, ConfigError, ResolvedTask};

use makefile::MakeProvider;
pub use rig::RigProvider;

/// A request-time source of runnable tasks.
///
/// Providers own discovery, resolution, and execution; `rig` is a thin
/// dispatcher over them. The trait is object-safe (no async) so providers can
/// be boxed and executed synchronously.
pub trait TaskProvider {
    /// Stable identifier for this provider (e.g. `"rig"`).
    fn name(&self) -> &str;
    /// Every task this provider offers.
    fn discover(&self) -> Vec<ResolvedTask>;
    /// Resolve a task path to a handle, owning the error text.
    fn resolve(&self, path: &str) -> Result<ResolvedTask, ConfigError>;
    /// Execute a resolved task synchronously, returning its exit code.
    fn run(&self, task: &ResolvedTask, args: &[String]) -> i32;
}

/// Standard makefile names make itself finds without an explicit `-f`, in
/// priority order (used to pick one per directory).
const STANDARD_MAKEFILE_NAMES: [&str; 3] = ["Makefile", "makefile", "GNUmakefile"];

/// Build the providers for a single command invocation.
///
/// Stateless: a pure function of the filesystem, constructed fresh per command
/// and discarded after. tmux stays the only held state. RigProvider is placed
/// first so rig tasks take precedence over make targets during resolution.
pub fn providers(
    config_path: Option<&str>,
) -> Result<Vec<Box<dyn TaskProvider + Send + Sync>>, ConfigError> {
    let mut ps: Vec<Box<dyn TaskProvider + Send + Sync>> = Vec::new();

    // Optional now: a missing rig config is not an error, but a malformed one
    // still fails loudly (try_load_config only maps not-found to None).
    if let Some((config, _dir)) = try_load_config(config_path)? {
        ps.push(Box::new(RigProvider::new(config)));
    }

    // Downward, gitignore-aware walk: one MakeProvider per discovered Makefile,
    // namespaced by its folder path relative to CWD. Sorted by namespace for
    // stable `rig tasks` output.
    let cwd = std::env::current_dir().map_err(|e| ConfigError::generic(e.to_string()))?;
    let mut discovered: Vec<(String, PathBuf)> = scan_for_makefiles(&cwd)?
        .into_iter()
        .map(|mf| {
            let dir = mf.parent().unwrap_or(&cwd);
            (namespace_from_relative_dir(&cwd, dir), mf.clone())
        })
        .collect();
    discovered.sort_by(|a, b| a.0.cmp(&b.0));
    for (ns, mf) in discovered {
        ps.push(Box::new(MakeProvider::new(mf, ns)));
    }

    if ps.is_empty() {
        return Err(ConfigError::generic("No rig.yaml or Makefile found"));
    }

    Ok(ps)
}

/// Discover every standard-named Makefile under `cwd`, gitignore-aware, one per
/// directory (highest priority name wins). Reuses the shared `ignore`-crate
/// walker so vendored/gitignored Makefiles are excluded exactly as rig files are.
fn scan_for_makefiles(cwd: &Path) -> Result<Vec<PathBuf>, ConfigError> {
    let files = crate::commands::walk_ignored_files(&cwd.to_string_lossy())
        .map_err(ConfigError::generic)?;

    // dir → best (lowest-index) standard name seen in that dir.
    let mut best: BTreeMap<PathBuf, usize> = BTreeMap::new();
    for path in files {
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_string()) else {
            continue;
        };
        let Some(prio) = STANDARD_MAKEFILE_NAMES.iter().position(|n| *n == name) else {
            continue;
        };
        let dir = path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| cwd.to_path_buf());
        best.entry(dir)
            .and_modify(|p| {
                if prio < *p {
                    *p = prio;
                }
            })
            .or_insert(prio);
    }

    Ok(best
        .into_iter()
        .map(|(dir, prio)| dir.join(STANDARD_MAKEFILE_NAMES[prio]))
        .collect())
}

/// Namespace for a Makefile's directory: its path relative to CWD with
/// separators mapped to `.`. CWD itself → `""` (bare target names).
fn namespace_from_relative_dir(cwd: &Path, dir: &Path) -> String {
    let rel = dir.strip_prefix(cwd).unwrap_or(dir);
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join(".")
}

/// Resolve a task path across providers, disambiguating short names that now
/// collide across folder-namespaced make providers.
///
/// - Gather every candidate: a dotted `path` matches a task's full `path`; a
///   short name matches a task's `name`.
/// - 0 candidates → fall back to `ps[0].resolve(path)`, preserving rig's error
///   text ("Unknown task", "Unknown group").
/// - 1 candidate → that one.
/// - many + dotted → same fully-qualified path across providers; lowest provider
///   index wins (rig index 0 → force precedence over make; the R1 overlap case).
/// - many + short → force precedence: a single rig-owned candidate wins over any
///   number of make targets (the plan's "rig task beats make target"). Otherwise
///   — a short name spread across make folders, or rig itself ambiguous — it is
///   genuinely ambiguous; `Err` listing the distinct FQ paths.
///
/// rig is the provider named `"rig"` (always index 0 when a rig config exists);
/// force precedence keys off that identity, not the raw index, since a make
/// provider occupies index 0 when no rig config is present.
///
/// perf: re-parses per call; fine while stateless.
pub fn resolve_across(
    ps: &[Box<dyn TaskProvider + Send + Sync>],
    path: &str,
) -> Result<(usize, ResolvedTask), ConfigError> {
    let dotted = path.contains('.');

    let mut candidates: Vec<(usize, ResolvedTask)> = ps
        .iter()
        .enumerate()
        .flat_map(|(i, p)| p.discover().into_iter().map(move |t| (i, t)))
        .filter(|(_, t)| {
            if dotted {
                t.path == path
            } else {
                t.name == path
            }
        })
        .collect();

    match candidates.len() {
        0 => ps[0].resolve(path).map(|t| (0, t)),
        1 => Ok(candidates.remove(0)),
        _ if dotted => {
            // Same FQ path owned by multiple providers → rig (lowest index) wins.
            Ok(candidates
                .into_iter()
                .min_by_key(|(i, _)| *i)
                .expect("len > 1"))
        }
        _ => {
            // Short name owned by several tasks. A lone rig task wins outright
            // over make targets (force precedence); anything else is ambiguous.
            let rig_indices: Vec<usize> = candidates
                .iter()
                .enumerate()
                .filter(|(_, (i, _))| ps[*i].name() == "rig")
                .map(|(pos, _)| pos)
                .collect();
            if rig_indices.len() == 1 {
                return Ok(candidates.remove(rig_indices[0]));
            }
            let mut fqs: Vec<String> = candidates.into_iter().map(|(_, t)| t.path).collect();
            fqs.sort();
            fqs.dedup();
            Err(ConfigError::generic(format!(
                "Ambiguous task '{}'. Matches: {}",
                path,
                fqs.join(", ")
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_cwd_is_bare() {
        let cwd = Path::new("/work/proj");
        assert_eq!(
            namespace_from_relative_dir(cwd, Path::new("/work/proj")),
            ""
        );
    }

    #[test]
    fn namespace_one_level() {
        let cwd = Path::new("/work/proj");
        assert_eq!(
            namespace_from_relative_dir(cwd, Path::new("/work/proj/backend")),
            "backend"
        );
    }

    #[test]
    fn namespace_nested_dot_joined() {
        let cwd = Path::new("/work/proj");
        assert_eq!(
            namespace_from_relative_dir(cwd, Path::new("/work/proj/apps/web")),
            "apps.web"
        );
    }
}
