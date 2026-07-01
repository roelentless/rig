mod common;

use common::*;

#[test]
fn schema_catches_unknown_service_keys() {
    let ctx = TestContext::new();
    ctx.write_file(
        "rig.yaml",
        &format!(
            r#"
groups:
  {}:
    services:
      api:
        command: echo hello
        working_dir: /tmp
        env:
          MY_VAR: test
"#,
            TEST_GROUP
        ),
    );

    let result = ctx.rig(&["start", "-d"]);
    assert_eq!(result.code, 1);
    assert!(
        result.stderr.contains("Unknown key 'env'"),
        "stderr: {}",
        result.stderr
    );
    assert!(
        result.stderr.contains("Valid keys:"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn empty_group_is_rejected() {
    // A group carrying only props (no dir/paths/tasks/services/groups) resolves
    // to nothing and must fail fast, naming the group.
    let ctx = TestContext::new();
    ctx.write_file(
        "rig.yaml",
        r#"
groups:
  ghost:
    environment:
      FOO: bar
"#,
    );

    let result = ctx.rig(&["tasks"]);
    assert_eq!(result.code, 1, "stdout: {}", result.stdout);
    assert!(
        result.stderr.contains("Group 'ghost' is empty"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn init_template_loads_cleanly() {
    // `rig init` must scaffold a wrapper-free rig.yaml (top-level services/tasks,
    // no `groups:` wrapper) that round-trips: the emitted template parses and
    // both `rig tasks` and `rig config` succeed against it.
    let ctx = TestContext::new();

    let init = ctx.rig(&["init"]);
    assert_eq!(init.code, 0, "stderr: {}", init.stderr);

    let written = ctx.read_file("rig.yaml");
    assert!(
        !written.contains("groups:"),
        "template should be wrapper-free: {}",
        written
    );

    let tasks = ctx.rig(&["tasks"]);
    assert_eq!(tasks.code, 0, "stderr: {}", tasks.stderr);
    assert!(
        strip_ansi(&tasks.stdout).contains("build"),
        "tasks: {}",
        tasks.stdout
    );

    let config = ctx.rig(&["config"]);
    assert_eq!(config.code, 0, "stderr: {}", config.stderr);
    assert!(
        strip_ansi(&config.stdout).contains("api"),
        "config: {}",
        config.stdout
    );
}

#[test]
fn schema_catches_unknown_requirement_keys() {
    let ctx = TestContext::new();
    ctx.write_file(
        "rig.yaml",
        &format!(
            r#"
groups:
  {}:
    services:
      req-bad:
        command: echo hello
        working_dir: /tmp
        requirements:
          - check: "true"
            command: "true"
            timeout: 30
"#,
            TEST_GROUP
        ),
    );

    let result = ctx.rig(&["start", "-d"]);
    assert_eq!(result.code, 1);
    assert!(
        result.stderr.contains("Unknown key 'timeout'"),
        "stderr: {}",
        result.stderr
    );
    assert!(
        result.stderr.contains("requirement"),
        "stderr: {}",
        result.stderr
    );
}
