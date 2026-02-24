mod common;

use common::*;

#[test]
fn tasks_lists_all() {
    let ctx = TestContext::new();
    ctx.setup_test_config();
    let result = ctx.rig(&["tasks"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    let clean = strip_ansi(&result.stdout);
    assert!(
        clean.contains(&format!("{}.group-cmd", TEST_GROUP)),
        "stdout: {}",
        clean
    );
    assert!(
        clean.contains(&format!("{}.echo-svc.greet", TEST_GROUP)),
        "stdout: {}",
        clean
    );
    assert!(clean.contains("A test group task"), "stdout: {}", clean);
}

#[test]
fn run_group_task() {
    let ctx = TestContext::new();
    ctx.setup_test_config();
    let result = ctx.rig(&["run", &format!("{}.group-cmd", TEST_GROUP)]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("group command output"),
        "stdout: {}",
        result.stdout
    );
}

#[test]
fn run_service_task() {
    let ctx = TestContext::new();
    ctx.setup_test_config();
    let result = ctx.rig(&["run", &format!("{}.echo-svc.greet", TEST_GROUP)]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("hello from greet"),
        "stdout: {}",
        result.stdout
    );
}

#[test]
fn run_exit_code_passthrough() {
    let ctx = TestContext::new();
    ctx.setup_test_config();
    let result = ctx.rig(&["run", &format!("{}.exit-with-code", TEST_GROUP)]);
    assert_eq!(result.code, 7);
}

#[test]
fn run_service_task_inherits_env() {
    let ctx = TestContext::new();
    ctx.setup_test_config();
    let result = ctx.rig(&["run", &format!("{}.counter.check-env", TEST_GROUP)]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("COUNT_VAR=from-service"),
        "stdout: {}",
        result.stdout
    );
    assert!(
        result.stdout.contains("EXTRA=from-task"),
        "stdout: {}",
        result.stdout
    );
}

#[test]
fn run_task_env_overrides() {
    let ctx = TestContext::new();
    ctx.setup_test_config();
    let result = ctx.rig(&["run", &format!("{}.echo-svc.show-env", TEST_GROUP)]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("PORT=3001"),
        "stdout: {}",
        result.stdout
    );
}

#[test]
fn run_with_args() {
    let ctx = TestContext::new();
    ctx.setup_test_config();
    let result = ctx.rig(&[
        "run",
        &format!("{}.echo-args", TEST_GROUP),
        "--",
        "foo",
        "bar",
        "baz",
    ]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("args: foo bar baz"),
        "stdout: {}",
        result.stdout
    );
}

#[test]
fn run_unknown_task_errors() {
    let ctx = TestContext::new();
    ctx.setup_test_config();
    let result = ctx.rig(&["run", &format!("{}.nonexistent", TEST_GROUP)]);
    assert_eq!(result.code, 1);
    assert!(
        result.stderr.contains("Unknown task"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn run_unknown_group_errors() {
    let ctx = TestContext::new();
    ctx.setup_test_config();
    let result = ctx.rig(&["run", "badgroup.cmd"]);
    assert_eq!(result.code, 1);
    assert!(
        result.stderr.contains("Unknown group"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn run_multiple_sequential() {
    let ctx = TestContext::new();
    ctx.setup_test_config();
    let result = ctx.rig(&[
        "run",
        &format!("{}.task-a", TEST_GROUP),
        &format!("{}.task-b", TEST_GROUP),
        &format!("{}.task-c", TEST_GROUP),
    ]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("task-a-output"),
        "stdout: {}",
        result.stdout
    );
    assert!(
        result.stdout.contains("task-b-output"),
        "stdout: {}",
        result.stdout
    );
    assert!(
        result.stdout.contains("task-c-output"),
        "stdout: {}",
        result.stdout
    );
}

#[test]
fn run_multiple_fail_fast() {
    let ctx = TestContext::new();
    ctx.setup_test_config();
    let result = ctx.rig(&[
        "run",
        &format!("{}.task-a", TEST_GROUP),
        &format!("{}.fail-task", TEST_GROUP),
        &format!("{}.task-c", TEST_GROUP),
    ]);
    assert_eq!(result.code, 3);
    assert!(
        result.stdout.contains("task-a-output"),
        "stdout: {}",
        result.stdout
    );
    assert!(
        result.stdout.contains("fail-task-ran"),
        "stdout: {}",
        result.stdout
    );
    // task-c should NOT have run due to fail-fast
    assert!(
        !result.stdout.contains("task-c-output"),
        "task-c should not run, stdout: {}",
        result.stdout
    );
}

#[test]
fn run_multiple_parallel() {
    let ctx = TestContext::new();
    ctx.setup_test_config();
    let result = ctx.rig(&[
        "run",
        "--parallel",
        &format!("{}.task-a", TEST_GROUP),
        &format!("{}.task-b", TEST_GROUP),
        &format!("{}.task-c", TEST_GROUP),
    ]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("task-a-output"),
        "stdout: {}",
        result.stdout
    );
    assert!(
        result.stdout.contains("task-b-output"),
        "stdout: {}",
        result.stdout
    );
    assert!(
        result.stdout.contains("task-c-output"),
        "stdout: {}",
        result.stdout
    );
}

#[test]
fn run_parallel_continues_on_failure() {
    let ctx = TestContext::new();
    ctx.setup_test_config();
    let result = ctx.rig(&[
        "run",
        "-p",
        &format!("{}.task-a", TEST_GROUP),
        &format!("{}.fail-task", TEST_GROUP),
        &format!("{}.task-c", TEST_GROUP),
    ]);
    assert_eq!(result.code, 3);
    assert!(
        result.stdout.contains("task-a-output"),
        "stdout: {}",
        result.stdout
    );
    assert!(
        result.stdout.contains("fail-task-ran"),
        "stdout: {}",
        result.stdout
    );
    // task-c SHOULD have run (parallel continues on failure)
    assert!(
        result.stdout.contains("task-c-output"),
        "task-c should run in parallel, stdout: {}",
        result.stdout
    );
    assert!(
        result.stderr.contains("failed with exit code 3"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn run_multiple_with_args_errors() {
    let ctx = TestContext::new();
    ctx.setup_test_config();
    let result = ctx.rig(&[
        "run",
        &format!("{}.task-a", TEST_GROUP),
        &format!("{}.task-b", TEST_GROUP),
        "--",
        "some",
        "args",
    ]);
    assert_eq!(result.code, 1);
    assert!(
        result
            .stderr
            .contains("Cannot pass arguments when running multiple tasks"),
        "stderr: {}",
        result.stderr
    );
}
