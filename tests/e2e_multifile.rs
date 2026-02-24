mod common;

use common::*;

#[test]
fn imports_merge_into_flat_namespace() {
    let ctx = TestContext::new();
    ctx.write_file(
        "db/rig.yaml",
        r#"
groups:
  database:
    services:
      postgres:
        command: sh -c "echo 'postgres started'; sleep 30"
        working_dir: /tmp
"#,
    );
    ctx.write_file(
        "rig.yaml",
        &format!(
            r#"
imports:
  - db/rig.yaml

groups:
  {}:
    services:
      api:
        command: sh -c "echo 'api started'; sleep 30"
        working_dir: /tmp
"#,
            TEST_GROUP
        ),
    );

    let result = ctx.rig(&["start", "-d", "api", "postgres"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("Started api"),
        "stdout: {}",
        result.stdout
    );
    assert!(
        result.stdout.contains("Started postgres"),
        "stdout: {}",
        result.stdout
    );

    assert!(session_exists("api", TEST_GROUP));
    assert!(session_exists("postgres", "database"));

    let ps = ctx.rig(&["ps"]);
    assert!(ps.stdout.contains(TEST_GROUP), "ps stdout: {}", ps.stdout);
    assert!(ps.stdout.contains("database"), "ps stdout: {}", ps.stdout);
}

#[test]
fn star_rig_yaml_recognized() {
    let ctx = TestContext::new();
    ctx.write_file(
        "infra.rig.yaml",
        r#"
groups:
  infra:
    services:
      redis:
        command: sh -c "echo 'redis'; sleep 30"
        working_dir: /tmp
"#,
    );
    ctx.write_file(
        "rig.yaml",
        &format!(
            r#"
imports:
  - infra.rig.yaml

groups:
  {}:
    services:
      app:
        command: sh -c "echo 'app'; sleep 30"
        working_dir: /tmp
"#,
            TEST_GROUP
        ),
    );

    let result = ctx.rig(&["start", "-d", "redis"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("Started redis"),
        "stdout: {}",
        result.stdout
    );
}

#[test]
fn circular_import_error() {
    let ctx = TestContext::new();
    ctx.write_file(
        "rig.yaml",
        r#"
imports:
  - a/rig.yaml

groups:
  root:
    services:
      svc1:
        command: echo "test"
        working_dir: /tmp
"#,
    );
    ctx.write_file(
        "a/rig.yaml",
        r#"
imports:
  - ../rig.yaml

groups:
  a-group:
    services:
      svc2:
        command: echo "test"
        working_dir: /tmp
"#,
    );

    let result = ctx.rig(&["ps"]);
    assert_eq!(result.code, 1);
    assert!(
        result.stderr.contains("Circular import detected"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn import_not_found_error() {
    let ctx = TestContext::new();
    ctx.write_file(
        "rig.yaml",
        &format!(
            r#"
imports:
  - nonexistent/rig.yaml

groups:
  {}:
    services:
      svc:
        command: echo "test"
        working_dir: /tmp
"#,
            TEST_GROUP
        ),
    );

    let result = ctx.rig(&["ps"]);
    assert_eq!(result.code, 1);
    assert!(
        result.stderr.contains("Import not found"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn duplicate_group_error() {
    let ctx = TestContext::new();
    ctx.write_file(
        "sub/rig.yaml",
        r#"
groups:
  mygroup:
    services:
      svc2:
        command: echo "test"
        working_dir: /tmp
"#,
    );
    ctx.write_file(
        "rig.yaml",
        r#"
imports:
  - sub/rig.yaml

groups:
  mygroup:
    services:
      svc1:
        command: echo "test"
        working_dir: /tmp
"#,
    );

    let result = ctx.rig(&["ps"]);
    assert_eq!(result.code, 1);
    assert!(
        result.stderr.contains("Duplicate group"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn duplicate_service_error() {
    let ctx = TestContext::new();
    ctx.write_file(
        "sub/rig.yaml",
        r#"
groups:
  group-b:
    services:
      api:
        command: echo "test"
        working_dir: /tmp
"#,
    );
    ctx.write_file(
        "rig.yaml",
        r#"
imports:
  - sub/rig.yaml

groups:
  group-a:
    services:
      api:
        command: echo "test"
        working_dir: /tmp
"#,
    );

    let result = ctx.rig(&["ps"]);
    assert_eq!(result.code, 1);
    assert!(
        result.stderr.contains("Duplicate service"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn same_file_imported_twice_deduped() {
    let ctx = TestContext::new();
    ctx.write_file(
        "shared/rig.yaml",
        r#"
groups:
  shared:
    services:
      db:
        command: sh -c "echo 'db'; sleep 30"
        working_dir: /tmp
"#,
    );
    ctx.write_file(
        "app/rig.yaml",
        r#"
imports:
  - ../shared/rig.yaml

groups:
  app:
    services:
      api:
        command: sh -c "echo 'api'; sleep 30"
        working_dir: /tmp
"#,
    );
    ctx.write_file(
        "rig.yaml",
        &format!(
            r#"
imports:
  - shared/rig.yaml
  - app/rig.yaml

groups:
  {}:
    services:
      root-svc:
        command: sh -c "echo 'root'; sleep 30"
        working_dir: /tmp
"#,
            TEST_GROUP
        ),
    );

    let result = ctx.rig(&["start", "-d", "db", "api", "root-svc"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("Started db"),
        "stdout: {}",
        result.stdout
    );
    assert!(
        result.stdout.contains("Started api"),
        "stdout: {}",
        result.stdout
    );
    assert!(
        result.stdout.contains("Started root-svc"),
        "stdout: {}",
        result.stdout
    );

    assert!(session_exists("db", "shared"));
    assert!(session_exists("api", "app"));
    assert!(session_exists("root-svc", TEST_GROUP));
}

#[test]
fn paths_relative_to_config_location() {
    let ctx = TestContext::new();
    ctx.write_file("backend/backend.env", "BACKEND_VAR=from-backend-env\n");
    ctx.write_file(
        "backend/rig.yaml",
        r#"
groups:
  backend:
    services:
      api:
        command: sh -c "echo BACKEND_VAR=$BACKEND_VAR; sleep 30"
        working_dir: .
        env_file: ./backend.env
"#,
    );
    ctx.write_file(
        "rig.yaml",
        &format!(
            r#"
imports:
  - backend/rig.yaml

groups:
  {}:
    services:
      root-svc:
        command: sh -c "echo 'root'; sleep 30"
        working_dir: /tmp
"#,
            TEST_GROUP
        ),
    );

    ctx.rig(&["start", "-d", "api"]);
    delay_ms(500);

    let result = ctx.rig(&["logs", "api"]);
    assert!(
        result.stdout.contains("BACKEND_VAR=from-backend-env"),
        "stdout: {}",
        result.stdout
    );
}

#[test]
fn depends_on_across_files() {
    let ctx = TestContext::new();
    ctx.write_file(
        "db/rig.yaml",
        r#"
groups:
  database:
    services:
      postgres:
        command: sh -c "echo 'postgres started'; sleep 30"
        working_dir: /tmp
        healthcheck:
          grace_ms: 100
"#,
    );
    ctx.write_file(
        "rig.yaml",
        &format!(
            r#"
imports:
  - db/rig.yaml

groups:
  {}:
    services:
      api:
        command: sh -c "echo 'api started'; sleep 30"
        working_dir: /tmp
        depends_on: [postgres]
"#,
            TEST_GROUP
        ),
    );

    let result = ctx.rig(&["start", "-d"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);

    let pg_idx = result
        .stdout
        .find("Started postgres")
        .expect("postgres should be started");
    let api_idx = result
        .stdout
        .find("Started api")
        .expect("api should be started");
    assert!(pg_idx < api_idx, "postgres should start before api");

    assert!(session_exists("postgres", "database"));
    assert!(session_exists("api", TEST_GROUP));
}

#[test]
fn depends_on_invalid_across_files_errors() {
    let ctx = TestContext::new();
    ctx.write_file(
        "sub/rig.yaml",
        r#"
groups:
  sub:
    services:
      svc:
        command: echo "test"
        working_dir: /tmp
"#,
    );
    ctx.write_file(
        "rig.yaml",
        &format!(
            r#"
imports:
  - sub/rig.yaml

groups:
  {}:
    services:
      api:
        command: echo "test"
        working_dir: /tmp
        depends_on: [nonexistent]
"#,
            TEST_GROUP
        ),
    );

    let result = ctx.rig(&["ps"]);
    assert_eq!(result.code, 1);
    assert!(
        result.stderr.contains("depends on unknown service"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn discover_lists_files() {
    // Create a temp dir manually since discover needs a specific directory structure
    let dir = tempfile::tempdir_in("/tmp").unwrap();
    let dir_path = dir.path().to_string_lossy().to_string();

    std::fs::create_dir_all(format!("{}/sub", dir_path)).unwrap();
    std::fs::write(
        format!("{}/rig.yaml", dir_path),
        r#"
groups:
  root:
    services:
      svc:
        command: echo "test"
        working_dir: /tmp
"#,
    )
    .unwrap();
    std::fs::write(
        format!("{}/sub/rig.yaml", dir_path),
        r#"
groups:
  sub:
    services:
      svc2:
        command: echo "test"
        working_dir: /tmp
"#,
    )
    .unwrap();

    let ctx = TestContext::new();
    let result = ctx.rig(&["discover", "--dry-run", &dir_path]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("rig.yaml"),
        "stdout: {}",
        result.stdout
    );
    assert!(
        result.stdout.contains("sub/rig.yaml"),
        "stdout: {}",
        result.stdout
    );
    assert!(
        result.stdout.contains("Missing"),
        "stdout: {}",
        result.stdout
    );
    assert!(
        result.stdout.contains("Dry run"),
        "stdout: {}",
        result.stdout
    );
}

#[test]
fn discover_with_yes_updates_config() {
    let dir = tempfile::tempdir_in("/tmp").unwrap();
    let dir_path = dir.path().to_string_lossy().to_string();

    std::fs::create_dir_all(format!("{}/new-service", dir_path)).unwrap();
    std::fs::write(
        format!("{}/rig.yaml", dir_path),
        r#"
groups:
  root:
    services:
      svc:
        command: echo "test"
        working_dir: /tmp
"#,
    )
    .unwrap();
    std::fs::write(
        format!("{}/new-service/rig.yaml", dir_path),
        r#"
groups:
  new:
    services:
      new-svc:
        command: echo "new"
        working_dir: /tmp
"#,
    )
    .unwrap();

    let ctx = TestContext::new();
    let result = ctx.rig(&["discover", "--yes", &dir_path]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);

    let content = std::fs::read_to_string(format!("{}/rig.yaml", dir_path)).unwrap();
    assert!(content.contains("imports"), "config: {}", content);
    assert!(
        content.contains("new-service/rig.yaml"),
        "config: {}",
        content
    );
}
