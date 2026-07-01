pub mod makefile;
pub mod rig;

use crate::config::{try_load_config, ConfigError, ResolvedTask, TaskSource};

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

/// Build the providers for a single command invocation.
///
/// Stateless: a pure function of the filesystem, constructed fresh per command
/// and discarded after. tmux stays the only held state. Makefile discovery is
/// now folded into the group tree during `try_load_config`, so a single
/// tree-backed `RigProvider` carries both rig-authored and make-sourced tasks.
pub fn providers(
    config_path: Option<&str>,
) -> Result<Vec<Box<dyn TaskProvider + Send + Sync>>, ConfigError> {
    // A missing config (no rig.yaml and no Makefile anywhere) is `None`; a
    // malformed rig config still fails loudly. The tree already contains the
    // discovered Makefile targets.
    match try_load_config(config_path)? {
        Some((root, _dir)) => Ok(vec![Box::new(RigProvider::new(root))]),
        None => Err(ConfigError::generic("No rig.yaml or Makefile found")),
    }
}

/// Resolve a task path, disambiguating short names that collide across the
/// folder-namespaced tree. Make targets and rig tasks now share one tree-backed
/// provider, so precedence keys off each task's `source`, not provider order.
///
/// - Gather every candidate: a dotted `path` matches a task's full `path`; a
///   short name matches a task's `name`.
/// - 0 candidates → fall back to `ps[0].resolve(path)`, preserving rig's error
///   text ("Unknown task", "Unknown group").
/// - 1 candidate → that one.
/// - many + dotted → same fully-qualified path; lowest provider index wins.
/// - many + short → force precedence: a single rig-authored candidate wins over
///   any number of make targets (the plan's "rig task beats make target").
///   Otherwise — a short name spread across folders, or rig itself ambiguous —
///   it is genuinely ambiguous; `Err` listing the distinct FQ paths.
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
            // Short name owned by several tasks. A lone rig-authored task wins
            // outright over make targets (force precedence); anything else is
            // ambiguous. Source is carried on the task, not the provider, since
            // rig and make now live in the same tree-backed provider.
            let rig_indices: Vec<usize> = candidates
                .iter()
                .enumerate()
                .filter(|(_, (_, t))| t.source == TaskSource::Rig)
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
