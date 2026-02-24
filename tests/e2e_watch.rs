mod common;

use common::*;

#[test]
fn watch_config_parses_correctly() {
    let ctx = TestContext::new();
    ctx.write_file("rig.yaml", TEST_CONFIG_WITH_WATCH);

    let result = ctx.rig(&["config", "--json"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    let clean = strip_ansi(&result.stdout);
    let config: serde_json::Value = serde_json::from_str(&clean).expect("valid JSON");
    let watched = &config["groups"][TEST_GROUP]["services"]["watched-svc"];

    assert!(watched["watch"].is_object(), "watch should be an object");
    assert_eq!(
        watched["watch"]["extensions"],
        serde_json::json!(["txt", "md"])
    );
    assert_eq!(
        watched["watch"]["patterns"],
        serde_json::json!(["**/*.log"])
    );
    assert_eq!(
        watched["watch"]["ignore"],
        serde_json::json!(["**/cache/**"])
    );
    assert_eq!(watched["watch"]["debounce"], serde_json::json!("100ms"));

    // Paths should be resolved relative to working_dir (/tmp)
    assert_eq!(watched["watch"]["paths"], serde_json::json!(["/tmp"]));
}

#[test]
fn watch_paths_default_to_working_dir() {
    let ctx = TestContext::new();
    ctx.write_file(
        "rig.yaml",
        &format!(
            r#"
groups:
  {}:
    services:
      minimal-watch:
        command: echo "test"
        working_dir: /tmp
        watch:
          extensions: [ts]
"#,
            TEST_GROUP
        ),
    );

    let result = ctx.rig(&["config", "--json"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    let clean = strip_ansi(&result.stdout);
    let config: serde_json::Value = serde_json::from_str(&clean).expect("valid JSON");
    let svc = &config["groups"][TEST_GROUP]["services"]["minimal-watch"];

    assert!(svc["watch"].is_object(), "watch should be an object");
    assert_eq!(svc["watch"]["extensions"], serde_json::json!(["ts"]));
    // paths should be null/undefined when not specified (default applied at runtime)
    assert!(
        svc["watch"]["paths"].is_null(),
        "paths should be null when not specified"
    );
}

#[test]
fn watch_service_starts_with_watchexec() {
    if !watchexec_installed() {
        eprintln!("Skipping watch test: watchexec not installed");
        return;
    }

    let ctx = TestContext::new();
    ctx.write_file("rig.yaml", TEST_CONFIG_WITH_WATCH);

    let result = ctx.rig(&["start", "-d", "watched-svc"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("Started watched-svc"),
        "stdout: {}",
        result.stdout
    );
    assert!(session_exists("watched-svc", TEST_GROUP));

    let ps = ctx.rig(&["ps"]);
    assert!(ps.stdout.contains("watched-svc"), "ps: {}", ps.stdout);
    assert!(ps.stdout.contains("running"), "ps: {}", ps.stdout);
}
