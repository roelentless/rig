pub mod makefile;
pub mod rig;

use std::sync::Arc;

use sysinfo::Signal;

use crate::config::{try_load_config, ConfigError, ResolvedTask};

pub use rig::RigProvider;

/// A request-time source of runnable tasks.
///
/// Owns discovery, resolution, and execution; `rig` is a thin dispatcher over
/// it. The trait is object-safe (no async) so it can be boxed and executed
/// synchronously. Make targets are folded into the group tree, so the single
/// `RigProvider` carries both rig-authored and make-sourced tasks.
pub trait TaskProvider {
    /// Stable identifier for this provider (e.g. `"rig"`).
    fn name(&self) -> &str;
    /// Every task this provider offers (env-file-free; listing never loads env).
    fn discover(&self) -> Vec<ResolvedTask>;
    /// Resolve a task path to a handle with its run-time env materialized,
    /// owning the error text.
    fn resolve(&self, path: &str) -> Result<ResolvedTask, ConfigError>;
    /// Execute a resolved task synchronously, returning its exit code.
    fn run(&self, task: &ResolvedTask, args: &[String]) -> i32;
    /// Cancel in-flight task work after rig received `signal`: forward the same
    /// signal to the task process tree (graceful — lets `make` run its
    /// delete-partial-target cleanup), wait for it to drain, then hard-kill
    /// whatever remains.
    fn cancel(&self, signal: Signal);
}

/// Build the task provider for a single command invocation.
///
/// Stateless: a pure function of the filesystem, constructed fresh per command
/// and discarded after. tmux stays the only held state. Makefile discovery is
/// folded into the group tree during `try_load_config`, so one tree-backed
/// `RigProvider` carries both rig-authored and make-sourced tasks.
pub fn provider(
    config_path: Option<&str>,
) -> Result<Arc<dyn TaskProvider + Send + Sync>, ConfigError> {
    // A missing config (no rig.yaml and no Makefile anywhere) is `None`; a
    // malformed rig config still fails loudly. The tree already contains the
    // discovered Makefile targets. Arc (not Box) so `rig run` can share the
    // provider between the running-task closure and the signal path.
    match try_load_config(config_path)? {
        Some((root, _dir)) => Ok(Arc::new(RigProvider::new(root))),
        None => Err(ConfigError::generic("No rig.yaml or Makefile found")),
    }
}
