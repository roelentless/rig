use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

pub const TEST_GROUP: &str = "rig-test";

pub const TEST_CONFIG: &str = r#"
groups:
  rig-test:
    services:
      echo-svc:
        command: sh -c "echo 'hello from echo-svc'; sleep 30"
        working_dir: /tmp
        color: cyan
        tasks:
          greet:
            command: echo "hello from greet"
          show-env:
            command: "sh -c 'echo PORT=$PORT'"
            environment:
              PORT: "3001"

      counter:
        command: sh -c "for i in 1 2 3 4 5; do echo count-$i; sleep 1; done; sleep 30"
        working_dir: /tmp
        color: yellow
        environment:
          COUNT_VAR: "from-service"
        tasks:
          check-env:
            command: "sh -c 'echo COUNT_VAR=$COUNT_VAR EXTRA=$EXTRA'"
            environment:
              EXTRA: "from-task"

      quick-exit:
        command: sh -c "echo 'quick exit'; exit 42"
        working_dir: /tmp
        color: red

    tasks:
      group-cmd:
        command: echo "group command output"
        working_dir: /tmp
        description: A test group task
      exit-with-code:
        command: sh -c "exit 7"
        working_dir: /tmp
      echo-args:
        command: "sh -c 'echo args: $*' --"
        working_dir: /tmp
      task-a:
        command: echo "task-a-output"
        working_dir: /tmp
      task-b:
        command: echo "task-b-output"
        working_dir: /tmp
      task-c:
        command: echo "task-c-output"
        working_dir: /tmp
      fail-task:
        command: sh -c "echo 'fail-task-ran'; exit 3"
        working_dir: /tmp
"#;

pub const TEST_CONFIG_WITH_WATCH: &str = r#"
groups:
  rig-test:
    services:
      watched-svc:
        command: sh -c "echo 'started'; sleep 30"
        working_dir: /tmp
        watch:
          paths: ['.']
          extensions: [txt, md]
          patterns: ['**/*.log']
          ignore: ['**/cache/**']
          debounce: 100ms
"#;

// All test group prefixes for cleanup
const ALL_TEST_GROUPS: &[&str] = &[
    "rig-test", "database", "backend", "frontend", "infra", "shared",
    "app", "root", "mygroup", "group-a", "group-b", "a-group", "sub", "new",
];

pub struct RigResult {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

fn rig_binary() -> PathBuf {
    // Use cargo build's output
    let mut path = std::env::current_exe().unwrap();
    path.pop(); // Remove test binary name
    path.pop(); // Remove deps
    path.push("rig");
    path
}

pub struct TestContext {
    pub dir: TempDir,
}

impl TestContext {
    pub fn new() -> Self {
        let dir = TempDir::new().unwrap();
        TestContext { dir }
    }

    pub fn path(&self) -> &std::path::Path {
        self.dir.path()
    }

    pub fn write_file(&self, name: &str, content: &str) {
        let path = self.dir.path().join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, content).unwrap();
    }

    pub fn read_file(&self, name: &str) -> String {
        std::fs::read_to_string(self.dir.path().join(name)).unwrap()
    }

    pub fn rig(&self, args: &[&str]) -> RigResult {
        let output = Command::new(rig_binary())
            .args(args)
            .current_dir(self.dir.path())
            .output()
            .expect("Failed to run rig binary");

        RigResult {
            code: output.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&output.stdout).to_string(),
            stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        }
    }

    pub fn setup_test_config(&self) {
        self.write_file("rig.yaml", TEST_CONFIG);
    }
}

impl Drop for TestContext {
    fn drop(&mut self) {
        cleanup_sessions();
    }
}

pub fn tmux(args: &[&str]) -> (i32, String) {
    let output = Command::new("tmux")
        .args(args)
        .output()
        .expect("Failed to run tmux");
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
    )
}

pub fn session_exists(name: &str, group: &str) -> bool {
    let session = format!("{}-{}", group, name);
    let (code, _) = tmux(&["has-session", "-t", &session]);
    code == 0
}

pub fn cleanup_sessions() {
    let (_, stdout) = tmux(&["list-sessions", "-F", "#{session_name}"]);
    for session in stdout.trim().lines().filter(|l| !l.is_empty()) {
        for prefix in ALL_TEST_GROUPS {
            if session.starts_with(&format!("{}-", prefix)) {
                tmux(&["kill-session", "-t", session]);
                break;
            }
        }
    }
}

pub fn strip_ansi(s: &str) -> String {
    let re = regex::Regex::new(r"\x1b\[[0-9;]*m").unwrap();
    re.replace_all(s, "").to_string()
}

pub fn delay_ms(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}

pub fn watchexec_installed() -> bool {
    Command::new("which")
        .arg("watchexec")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
