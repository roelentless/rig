use std::collections::{HashMap, HashSet};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use sysinfo::{Pid, ProcessesToUpdate, Signal, System};

use crate::config::{get_all_tasks, resolve_task, ConfigError, Group, ResolvedTask};
use crate::output::{log_error, log_verbose};

use super::TaskProvider;

/// How long cancellation waits for the signalled task tree to drain before the
/// SIGKILL backstop.
const CANCEL_GRACE: Duration = Duration::from_secs(5);
/// Re-snapshot interval while waiting for the tree to drain.
const CANCEL_POLL: Duration = Duration::from_millis(100);

/// The rig provider: tasks defined in the loaded rig group tree.
pub struct RigProvider {
    root: Group,
}

impl RigProvider {
    pub fn new(root: Group) -> Self {
        RigProvider { root }
    }
}

impl TaskProvider for RigProvider {
    fn name(&self) -> &str {
        "rig"
    }

    fn discover(&self) -> Vec<ResolvedTask> {
        get_all_tasks(&self.root)
    }

    fn resolve(&self, path: &str) -> Result<ResolvedTask, ConfigError> {
        resolve_task(path, &self.root)
    }

    fn run(&self, task: &ResolvedTask, args: &[String]) -> i32 {
        run_task(task, args)
    }

    fn cancel(&self, signal: Signal) {
        cancel_descendants(std::process::id(), signal);
    }
}

/// Cancel every descendant process of `root` (not `root` itself), graceful
/// first: forward `signal` (the one rig received) to the whole subtree —
/// emulating what a terminal foreground group delivers, which is what `make`
/// expects to run its delete-partial-target cleanup. Then poll, re-snapshotting
/// so children forked mid-shutdown are seen (and signalled), until the tree
/// drains or the grace window ends — after which whatever remains is SIGKILLed.
fn cancel_descendants(root: u32, signal: Signal) {
    let mut sys = System::new();
    let mut signalled: HashSet<Pid> = HashSet::new();
    let deadline = Instant::now() + CANCEL_GRACE;

    loop {
        let victims = descendants(&mut sys, root);
        if victims.is_empty() {
            return;
        }
        for &pid in &victims {
            if signalled.insert(pid) {
                if let Some(proc_) = sys.process(pid) {
                    proc_.kill_with(signal);
                }
            }
        }
        if Instant::now() >= deadline {
            break;
        }
        std::thread::sleep(CANCEL_POLL);
    }

    // Backstop: SIGKILL processes that ignored (or never drained after) the
    // graceful signal.
    for pid in descendants(&mut sys, root) {
        if let Some(proc_) = sys.process(pid) {
            proc_.kill_with(Signal::Kill);
        }
    }
}

/// Descendant PIDs of `root` (not `root` itself) from a fresh process snapshot:
/// refresh, build the parent→children map, walk the subtree.
fn descendants(sys: &mut System, root: u32) -> Vec<Pid> {
    sys.refresh_processes(ProcessesToUpdate::All, true);

    let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
    for (pid, proc_) in sys.processes() {
        // On Linux, sysinfo lists threads (/proc/*/task) as processes parented to
        // their own process; killing one SIGKILLs the whole thread group — i.e. us.
        if proc_.thread_kind().is_some() {
            continue;
        }
        if let Some(parent) = proc_.parent() {
            children.entry(parent).or_default().push(*pid);
        }
    }

    let mut stack = vec![Pid::from_u32(root)];
    let mut found = Vec::new();
    while let Some(pid) = stack.pop() {
        if let Some(kids) = children.get(&pid) {
            for &kid in kids {
                found.push(kid);
                stack.push(kid);
            }
        }
    }
    found
}

/// Quote an argument for safe interpolation into a `sh -c` command line.
pub(crate) fn shell_escape(arg: &str) -> String {
    if !arg.is_empty()
        && arg
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '=' | '@' | ':'))
    {
        arg.to_string()
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
}

/// Execute a command string synchronously via `sh -c` in `working_dir`,
/// inheriting stdio and merging `environment`. Returns the process exit code.
/// Shared exec path for all providers.
pub(crate) fn exec_sh(
    command: &str,
    working_dir: &str,
    environment: Option<&HashMap<String, String>>,
) -> i32 {
    let mut cmd = Command::new("sh");
    cmd.args(["-c", command]);
    cmd.current_dir(working_dir);
    cmd.stdin(Stdio::inherit());
    cmd.stdout(Stdio::inherit());
    cmd.stderr(Stdio::inherit());

    if let Some(env) = environment {
        cmd.envs(env);
    }

    match cmd.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            log_error(&format!(
                "Failed to run command '{}' in '{}': {}",
                command, working_dir, e
            ));
            1
        }
    }
}

/// Execute a resolved rig task synchronously, appending escaped args to the
/// task's command. Returns the process exit code.
fn run_task(resolved: &ResolvedTask, args: &[String]) -> i32 {
    let full_command = if args.is_empty() {
        resolved.command.clone()
    } else {
        format!(
            "{} {}",
            resolved.command,
            args.iter()
                .map(|a| shell_escape(a))
                .collect::<Vec<_>>()
                .join(" ")
        )
    };

    log_verbose(&format!("task={}", resolved.path));
    log_verbose(&format!("command={}", full_command));
    log_verbose(&format!("working_dir={}", resolved.working_dir));

    exec_sh(
        &full_command,
        &resolved.working_dir,
        resolved.environment.as_ref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Group, Props, TaskDef, TaskSource};
    use tempfile::TempDir;

    fn task_def(command: &str) -> TaskDef {
        TaskDef {
            command: command.to_string(),
            working_dir: None,
            environment: None,
            env_files: Vec::new(),
            description: None,
            source: TaskSource::Rig,
            default_goal: false,
        }
    }

    fn child_group(name: &str, tasks: Vec<(String, TaskDef)>) -> Group {
        Group {
            name: name.to_string(),
            dir: None,
            paths: None,
            props: Props::default(),
            tasks,
            services: Vec::new(),
            groups: Vec::new(),
        }
    }

    /// A tree with two group-level tasks under `alpha` and one under `beta`,
    /// with `deploy` intentionally duplicated across groups for ambiguity.
    fn sample_config() -> Group {
        Group {
            name: String::new(),
            dir: None,
            paths: None,
            props: Props::default(),
            tasks: Vec::new(),
            services: Vec::new(),
            groups: vec![
                child_group(
                    "alpha",
                    vec![
                        ("build".to_string(), task_def("echo build")),
                        ("deploy".to_string(), task_def("echo alpha deploy")),
                    ],
                ),
                child_group(
                    "beta",
                    vec![("deploy".to_string(), task_def("echo beta deploy"))],
                ),
            ],
        }
    }

    #[test]
    fn discover_matches_get_all_tasks() {
        let config = sample_config();
        let provider = RigProvider::new(config.clone());

        let via_provider: Vec<String> =
            provider.discover().iter().map(|t| t.path.clone()).collect();
        let via_config: Vec<String> = get_all_tasks(&config)
            .iter()
            .map(|t| t.path.clone())
            .collect();

        assert_eq!(via_provider, via_config);
        assert_eq!(
            via_provider,
            vec!["alpha.build", "alpha.deploy", "beta.deploy"]
        );
    }

    #[test]
    fn resolve_ok_for_valid_path() {
        let provider = RigProvider::new(sample_config());
        let task = provider
            .resolve("alpha.build")
            .expect("valid path resolves");
        assert_eq!(task.path, "alpha.build");
        assert_eq!(task.command, "echo build");
    }

    #[test]
    fn resolve_unknown_task_error_text() {
        let provider = RigProvider::new(sample_config());
        let err = provider.resolve("nonexistent").unwrap_err();
        assert_eq!(err.to_string(), "Unknown task 'nonexistent'");
    }

    #[test]
    fn resolve_ambiguous_task_error_text() {
        let provider = RigProvider::new(sample_config());
        let err = provider.resolve("deploy").unwrap_err();
        assert_eq!(
            err.to_string(),
            "Ambiguous task 'deploy'. Matches: alpha.deploy, beta.deploy"
        );
    }

    #[test]
    fn run_returns_process_exit_code() {
        let dir = TempDir::new().unwrap();
        let mut task = task_def("exit 7");
        task.working_dir = Some(dir.path().to_string_lossy().to_string());
        let resolved = ResolvedTask {
            path: "alpha.fail".to_string(),
            group: "alpha".to_string(),
            service: None,
            name: "fail".to_string(),
            command: task.command.clone(),
            working_dir: task.working_dir.clone().unwrap(),
            environment: None,
            description: None,
            source: TaskSource::Rig,
            default_goal: false,
        };

        let provider = RigProvider::new(sample_config());
        assert_eq!(provider.run(&resolved, &[]), 7);
    }

    #[test]
    fn run_inherits_stdout_and_succeeds() {
        // Redirect stdout to a file inside the working dir (via the command) so
        // we can assert the inherited stream carried the output, without capturing
        // the process's real stdout.
        let dir = TempDir::new().unwrap();
        let out_path = dir.path().join("out.txt");
        let resolved = ResolvedTask {
            path: "alpha.write".to_string(),
            group: "alpha".to_string(),
            service: None,
            name: "write".to_string(),
            command: format!("echo hello > {}", out_path.to_string_lossy()),
            working_dir: dir.path().to_string_lossy().to_string(),
            environment: None,
            description: None,
            source: TaskSource::Rig,
            default_goal: false,
        };

        let provider = RigProvider::new(sample_config());
        assert_eq!(provider.run(&resolved, &[]), 0);
        let written = std::fs::read_to_string(&out_path).unwrap();
        assert_eq!(written.trim(), "hello");
    }
}
