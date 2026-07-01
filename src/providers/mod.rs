pub mod makefile;

/// A task discovered by a [`TaskProvider`].
///
/// `command` is the lowered command string (e.g. `"make build"` /
/// `"make -f ci.mk build"`) exactly as produced today.
pub struct DiscoveredTask {
    pub name: String,
    pub command: String,
    pub working_dir: String,
    pub description: Option<String>,
}

/// A source of tasks (e.g. a set of Makefiles).
pub trait TaskProvider {
    fn name(&self) -> &str;
    fn discover(&self) -> Vec<DiscoveredTask>;
}
