pub mod makefile;
pub mod rig;

use crate::config::{load_config, ConfigError, ResolvedTask};

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
/// and discarded after. tmux stays the only held state.
pub fn providers(
    config_path: Option<&str>,
) -> Result<Vec<Box<dyn TaskProvider + Send + Sync>>, ConfigError> {
    let (config, _dir) = load_config(config_path)?;
    // TODO(make-provider): define multi-provider resolution
    Ok(vec![Box::new(RigProvider::new(config))])
}
