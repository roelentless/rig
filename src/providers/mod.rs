pub mod makefile;
pub mod rig;

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

    // CWD-level standard Makefile → root namespace (bare target names). The
    // subfolder walk is M2.
    if let Some(makefile) = standard_makefile() {
        ps.push(Box::new(MakeProvider::new(makefile, String::new())));
    }

    if ps.is_empty() {
        return Err(ConfigError::generic("No rig.yaml or Makefile found"));
    }

    Ok(ps)
}

/// The standard Makefile in the CWD, if any (matching make's own name set).
fn standard_makefile() -> Option<PathBuf> {
    for name in ["Makefile", "makefile", "GNUmakefile"] {
        let path = Path::new(name);
        if path.exists() {
            return Some(path.to_path_buf());
        }
    }
    None
}

/// Resolve a task path across providers in order. First `Ok` wins (rig is index
/// 0 → force precedence). If none resolve, returns the FIRST provider's `Err`,
/// preserving rig's error text ("Unknown task", "Unknown group", "Ambiguous
/// task 'deploy'").
///
/// Deferred edge (documented, NOT solved): if RigProvider returns an "Ambiguous"
/// error for a short name that a MakeProvider also owns, the make match wins
/// here (first Ok wins, make tried after rig). Rare in mixed configs; revisit if
/// it bites.
pub fn resolve_across(
    ps: &[Box<dyn TaskProvider + Send + Sync>],
    path: &str,
) -> Result<(usize, ResolvedTask), ConfigError> {
    let mut first_err: Option<ConfigError> = None;
    for (i, p) in ps.iter().enumerate() {
        match p.resolve(path) {
            Ok(task) => return Ok((i, task)),
            Err(e) => {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
    }
    Err(first_err.expect("providers() guarantees at least one provider"))
}
