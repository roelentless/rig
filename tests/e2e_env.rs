mod common;

use common::*;

#[test]
fn env_file_loads_variables() {
    let ctx = TestContext::new();
    ctx.write_file("test.env", "MY_VAR=from_env_file\n");
    ctx.write_file(
        "rig.yaml",
        &format!(
            r#"
groups:
  {}:
    services:
      env-test:
        command: sh -c "echo MY_VAR=$MY_VAR; sleep 30"
        working_dir: /tmp
        env_file: ./test.env
"#,
            TEST_GROUP
        ),
    );

    ctx.rig(&["start", "-d", "env-test"]);
    delay_ms(500);

    let result = ctx.rig(&["logs", "env-test"]);
    assert!(
        result.stdout.contains("MY_VAR=from_env_file"),
        "stdout: {}",
        result.stdout
    );
}

#[test]
fn env_file_required_false_skips_missing() {
    let ctx = TestContext::new();
    ctx.write_file(
        "rig.yaml",
        &format!(
            r#"
groups:
  {}:
    services:
      optional-env:
        command: sh -c "echo 'started'; sleep 30"
        working_dir: /tmp
        env_file:
          - path: ./nonexistent.env
            required: false
"#,
            TEST_GROUP
        ),
    );

    let result = ctx.rig(&["start", "-d", "optional-env"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("Started optional-env"),
        "stdout: {}",
        result.stdout
    );
}

#[test]
fn inline_env_overrides_env_file() {
    let ctx = TestContext::new();
    ctx.write_file("override.env", "MY_VAR=from_file\n");
    ctx.write_file(
        "rig.yaml",
        &format!(
            r#"
groups:
  {}:
    services:
      override-test:
        command: sh -c "echo MY_VAR=$MY_VAR; sleep 30"
        working_dir: /tmp
        env_file: ./override.env
        environment:
          MY_VAR: from_inline
"#,
            TEST_GROUP
        ),
    );

    ctx.rig(&["start", "-d", "override-test"]);
    delay_ms(500);

    let result = ctx.rig(&["logs", "override-test"]);
    assert!(
        result.stdout.contains("MY_VAR=from_inline"),
        "stdout: {}",
        result.stdout
    );
}

#[test]
fn env_file_required_true_errors_on_missing() {
    let ctx = TestContext::new();
    ctx.write_file(
        "rig.yaml",
        &format!(
            r#"
groups:
  {}:
    services:
      required-env:
        command: sh -c "echo 'test'; sleep 30"
        working_dir: /tmp
        env_file: ./definitely-missing.env
"#,
            TEST_GROUP
        ),
    );

    let result = ctx.rig(&["start", "-d"]);
    assert_eq!(result.code, 1);
    assert!(
        result.stderr.contains("Failed to load env file"),
        "stderr: {}",
        result.stderr
    );
}
