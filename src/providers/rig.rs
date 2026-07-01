use std::collections::HashMap;
use std::process::{Command, Stdio};

use crate::config::{get_all_tasks, resolve_task, Config, ConfigError, ResolvedTask};
use crate::output::{log_error, log_verbose};

use super::TaskProvider;

/// The rig provider: tasks defined in the loaded rig `Config`.
pub struct RigProvider {
    config: Config,
}

impl RigProvider {
    pub fn new(config: Config) -> Self {
        RigProvider { config }
    }
}

impl TaskProvider for RigProvider {
    fn name(&self) -> &str {
        "rig"
    }

    fn discover(&self) -> Vec<ResolvedTask> {
        get_all_tasks(&self.config)
    }

    fn resolve(&self, path: &str) -> Result<ResolvedTask, ConfigError> {
        resolve_task(path, &self.config)
    }

    fn run(&self, task: &ResolvedTask, args: &[String]) -> i32 {
        run_task(task, args)
    }
}

/// Quote an argument for safe interpolation into a `sh -c` command line.
/// Shared with `MakeProvider`.
pub(crate) fn shell_escape(arg: &str) -> String {
    if arg
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
    use crate::config::{GroupDef, TaskDef};
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn task_def(command: &str) -> TaskDef {
        TaskDef {
            command: command.to_string(),
            working_dir: None,
            environment: None,
            env_file: None,
            description: None,
        }
    }

    /// A config with two group-level tasks under `alpha` and one under `beta`,
    /// with `deploy` intentionally duplicated across groups for ambiguity.
    fn sample_config() -> Config {
        let mut alpha_tasks = HashMap::new();
        alpha_tasks.insert("build".to_string(), task_def("echo build"));
        alpha_tasks.insert("deploy".to_string(), task_def("echo alpha deploy"));

        let mut beta_tasks = HashMap::new();
        beta_tasks.insert("deploy".to_string(), task_def("echo beta deploy"));

        let mut groups = HashMap::new();
        groups.insert(
            "alpha".to_string(),
            GroupDef {
                working_dir: None,
                services: None,
                tasks: Some(alpha_tasks),
            },
        );
        groups.insert(
            "beta".to_string(),
            GroupDef {
                working_dir: None,
                services: None,
                tasks: Some(beta_tasks),
            },
        );

        Config { groups }
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
        };

        let provider = RigProvider::new(sample_config());
        assert_eq!(provider.run(&resolved, &[]), 0);
        let written = std::fs::read_to_string(&out_path).unwrap();
        assert_eq!(written.trim(), "hello");
    }
}
