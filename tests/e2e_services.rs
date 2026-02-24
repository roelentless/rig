mod common;

use common::*;

#[test]
fn start_d_starts_in_background() {
    let ctx = TestContext::new();
    ctx.setup_test_config();
    let result = ctx.rig(&["start", "-d", "echo-svc"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("Started echo-svc"),
        "stdout: {}",
        result.stdout
    );
    assert!(session_exists("echo-svc", TEST_GROUP));
}

#[test]
fn up_d_alias_for_start() {
    let ctx = TestContext::new();
    ctx.setup_test_config();
    let result = ctx.rig(&["up", "-d", "echo-svc"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("Started echo-svc"),
        "stdout: {}",
        result.stdout
    );
    assert!(session_exists("echo-svc", TEST_GROUP));
}

#[test]
fn ps_shows_status() {
    let ctx = TestContext::new();
    ctx.setup_test_config();
    ctx.rig(&["start", "-d", "echo-svc"]);
    let result = ctx.rig(&["ps"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("echo-svc"),
        "stdout: {}",
        result.stdout
    );
    assert!(
        result.stdout.contains("running"),
        "stdout: {}",
        result.stdout
    );
}

#[test]
fn stop_stops_processes() {
    let ctx = TestContext::new();
    ctx.setup_test_config();
    ctx.rig(&["start", "-d", "echo-svc"]);
    assert!(session_exists("echo-svc", TEST_GROUP));

    let result = ctx.rig(&["stop", "echo-svc"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);

    delay_ms(200);
    assert!(!session_exists("echo-svc", TEST_GROUP));
}

#[test]
fn down_alias_for_stop() {
    let ctx = TestContext::new();
    ctx.setup_test_config();
    ctx.rig(&["start", "-d", "echo-svc"]);
    assert!(session_exists("echo-svc", TEST_GROUP));

    let result = ctx.rig(&["down", "echo-svc"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);

    delay_ms(200);
    assert!(!session_exists("echo-svc", TEST_GROUP));
}

#[test]
fn restart_restarts_processes() {
    let ctx = TestContext::new();
    ctx.setup_test_config();
    ctx.rig(&["start", "-d", "echo-svc"]);
    delay_ms(1100);

    let result = ctx.rig(&["restart", "echo-svc"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(session_exists("echo-svc", TEST_GROUP));
}

#[test]
fn logs_captures_output() {
    let ctx = TestContext::new();
    ctx.setup_test_config();
    ctx.rig(&["start", "-d", "echo-svc"]);
    delay_ms(500);

    let result = ctx.rig(&["logs", "echo-svc"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("hello from echo-svc"),
        "stdout: {}",
        result.stdout
    );
}

#[test]
fn start_no_duplicate_running() {
    let ctx = TestContext::new();
    ctx.setup_test_config();
    ctx.rig(&["start", "-d", "echo-svc"]);
    let result = ctx.rig(&["start", "-d", "echo-svc"]);
    assert!(
        result.stdout.contains("already running"),
        "stdout: {}",
        result.stdout
    );
}

#[test]
fn start_multiple_processes() {
    let ctx = TestContext::new();
    ctx.setup_test_config();
    let result = ctx.rig(&["start", "-d", "echo-svc", "counter"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("Started echo-svc"),
        "stdout: {}",
        result.stdout
    );
    assert!(
        result.stdout.contains("Started counter"),
        "stdout: {}",
        result.stdout
    );
    assert!(session_exists("echo-svc", TEST_GROUP));
    assert!(session_exists("counter", TEST_GROUP));
}

#[test]
fn unknown_process_errors() {
    let ctx = TestContext::new();
    ctx.setup_test_config();
    let result = ctx.rig(&["start", "-d", "nonexistent"]);
    assert_eq!(result.code, 1);
    assert!(
        result.stderr.contains("Unknown"),
        "stderr: {}",
        result.stderr
    );
}
