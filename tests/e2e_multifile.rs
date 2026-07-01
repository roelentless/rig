mod common;

use common::*;

// ============================================================================
// Group tree composition (replaces the old `imports:` flat-merge model).
//
// Composition is now folders + `dir:`/`paths:` group pointers. A subfolder with
// a rig file auto-becomes a child group named by its folder; an authored group
// with `dir:` renames/reshapes it. Properties cascade ancestor-wins.
// ============================================================================

#[test]
fn dir_group_renames_folder() {
    // Mirror the inference-style rename: the `db` folder is pulled in under the
    // authored name `database` via `dir:`, so the folder name is a lie the
    // authored group overrides. Bare units in db/rig.yaml attach to `database`.
    let ctx = TestContext::new();
    ctx.write_file(
        "db/rig.yaml",
        r#"
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
groups:
  {}:
    services:
      api:
        command: sh -c "echo 'api started'; sleep 30"
        working_dir: /tmp
  database:
    dir: ./db
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

    // Folder `db` was renamed to `database` — the session lives under `database`.
    assert!(session_exists("api", TEST_GROUP));
    assert!(session_exists("postgres", "database"));

    let ps = ctx.rig(&["ps"]);
    assert!(ps.stdout.contains(TEST_GROUP), "ps stdout: {}", ps.stdout);
    assert!(ps.stdout.contains("database"), "ps stdout: {}", ps.stdout);
}

#[test]
fn child_folder_rig_auto_group() {
    // A subfolder with a rig.yaml auto-becomes a child group named by its folder,
    // with no declaration anywhere.
    let ctx = TestContext::new();
    ctx.write_file(
        "backend/rig.yaml",
        r#"
services:
  worker:
    command: sh -c "echo 'worker started'; sleep 30"
    working_dir: /tmp
"#,
    );

    let result = ctx.rig(&["start", "-d", "worker"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("Started worker"),
        "stdout: {}",
        result.stdout
    );
    // Auto group name == folder name `backend`.
    assert!(session_exists("worker", "backend"));
}

#[test]
fn paths_pulls_explicit_file_scoped_under_group() {
    // An authored group with `paths:` pulls an explicit rig file into itself. The
    // file's folder does NOT also auto-become a group (the file is adopted).
    let ctx = TestContext::new();
    ctx.write_file(
        "pkgs/lib.rig.yaml",
        r#"
tasks:
  build-lib:
    command: echo lib-built
    working_dir: /tmp
"#,
    );
    ctx.write_file(
        "rig.yaml",
        r#"
groups:
  libs:
    paths:
      - ./pkgs/lib.rig.yaml
"#,
    );

    // Task is addressable under the authored group, scoped as `libs.build-lib`.
    let run = ctx.rig(&["run", "libs.build-lib"]);
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("lib-built"), "stdout: {}", run.stdout);

    let tasks = ctx.rig(&["tasks"]);
    let clean = strip_ansi(&tasks.stdout);
    assert!(clean.contains("libs.build-lib"), "tasks: {}", clean);
    // The adopted file's folder is not auto-grouped.
    assert!(
        !clean.contains("pkgs."),
        "pkgs leaked as auto group: {}",
        clean
    );
}

#[test]
fn top_level_bare_task_runs() {
    // A top-level (bare, no group wrapper) task attaches to the root group and is
    // addressable by its bare name.
    let ctx = TestContext::new();
    ctx.write_file(
        "rig.yaml",
        r#"
tasks:
  deploy:
    command: echo bare-deploy-output
    working_dir: /tmp
"#,
    );

    let result = ctx.rig(&["run", "deploy"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("bare-deploy-output"),
        "stdout: {}",
        result.stdout
    );

    let tasks = ctx.rig(&["tasks"]);
    let clean = strip_ansi(&tasks.stdout);
    // Bare name, not group-prefixed.
    assert!(clean.contains("deploy"), "tasks: {}", clean);
}

#[test]
fn top_level_bare_service_listed() {
    // A top-level bare service attaches to the root group and surfaces in config.
    let ctx = TestContext::new();
    ctx.write_file(
        "rig.yaml",
        r#"
services:
  web:
    command: sh -c "echo web; sleep 30"
    working_dir: /tmp
"#,
    );

    let result = ctx.rig(&["config"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("web"),
        "config stdout: {}",
        result.stdout
    );
}

#[test]
fn ancestor_wins_env_cascade_reaches_process() {
    // A root-level `environment:` overrides a child group's service env, and the
    // merged value reaches the actual tmux process (control from above).
    let ctx = TestContext::new();
    ctx.write_file(
        "rig.yaml",
        r#"
environment:
  SHARED: from-root
groups:
  app:
    services:
      cascade-svc:
        command: sh -c "echo SHARED=$SHARED; sleep 30"
        working_dir: /tmp
        environment:
          SHARED: from-child
"#,
    );

    let start = ctx.rig(&["start", "-d", "cascade-svc"]);
    assert_eq!(start.code, 0, "stderr: {}", start.stderr);
    assert!(session_exists("cascade-svc", "app"));
    delay_ms(500);

    let logs = ctx.rig(&["logs", "cascade-svc"]);
    assert!(
        logs.stdout.contains("SHARED=from-root"),
        "root env must override child and reach the process, stdout: {}",
        logs.stdout
    );
}

#[test]
fn folder_aware_relative_env_file() {
    // env_file resolves relative to the FILE's own directory, not CWD.
    let ctx = TestContext::new();
    ctx.write_file("backend/backend.env", "BACKEND_VAR=from-backend-env\n");
    ctx.write_file(
        "backend/rig.yaml",
        r#"
services:
  api:
    command: sh -c "echo BACKEND_VAR=$BACKEND_VAR; sleep 30"
    working_dir: .
    env_file: ./backend.env
"#,
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
fn folder_aware_relative_working_dir() {
    // A relative `working_dir: .` resolves to the declaring file's folder.
    let ctx = TestContext::new();
    ctx.write_file(
        "backend/rig.yaml",
        r#"
tasks:
  show-pwd:
    command: pwd
    working_dir: .
"#,
    );

    let result = ctx.rig(&["run", "backend.show-pwd"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("/backend"),
        "working_dir should resolve into backend/, stdout: {}",
        result.stdout
    );
}

#[test]
fn depends_on_across_groups() {
    // depends_on references a service by bare name across folder groups; ordering
    // and healthcheck grace behavior are unchanged.
    let ctx = TestContext::new();
    ctx.write_file(
        "database/rig.yaml",
        r#"
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
        .expect("postgres started");
    let api_idx = result.stdout.find("Started api").expect("api started");
    assert!(pg_idx < api_idx, "postgres should start before api");

    assert!(session_exists("postgres", "database"));
    assert!(session_exists("api", TEST_GROUP));
}

#[test]
fn dir_group_adopts_makefile_no_duplicate() {
    // A `dir:` group whose directory holds a Makefile adopts its targets under
    // the authored group name. The raw folder-namespaced entry (`svc.*`) is
    // suppressed because the dir is adopted.
    let ctx = TestContext::new();
    ctx.write_file(
        "svc/Makefile",
        "build:\n\t@echo svc-make-build\n\nrelease:\n\t@echo svc-release\n",
    );
    ctx.write_file("rig.yaml", "groups:\n  api:\n    dir: ./svc\n");

    let run = ctx.rig(&["run", "api.build"]);
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(
        run.stdout.contains("svc-make-build"),
        "stdout: {}",
        run.stdout
    );

    let tasks = ctx.rig(&["tasks"]);
    let clean = strip_ansi(&tasks.stdout);
    assert!(clean.contains("api.build"), "tasks: {}", clean);
    assert!(clean.contains("api.release"), "tasks: {}", clean);
    // The adopted dir does not also surface a raw `svc.*` auto-entry.
    assert!(!clean.contains("svc."), "duplicate svc entry: {}", clean);
}

#[test]
fn rig_task_overrides_make_target_same_group() {
    // Within one group (a `dir:` group over a Makefile-bearing dir), a rig-
    // authored task of the same name as a make target wins — make never runs.
    let ctx = TestContext::new();
    ctx.write_file("svc/Makefile", "build:\n\t@echo make-build\n");
    ctx.write_file(
        "rig.yaml",
        r#"
groups:
  api:
    dir: ./svc
    tasks:
      build:
        command: echo rig-build
        working_dir: .
"#,
    );

    let run = ctx.rig(&["run", "api.build"]);
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(run.stdout.contains("rig-build"), "stdout: {}", run.stdout);
    assert!(
        !run.stdout.contains("make-build"),
        "make target should not run: {}",
        run.stdout
    );
}

#[test]
fn makefile_only_subdir_auto_child_group() {
    // A subdir with ONLY a Makefile (no rig.yaml) auto-becomes a child group
    // named by its folder, alongside a root rig.yaml.
    let ctx = TestContext::new();
    ctx.write_file("tools/Makefile", "lint:\n\t@echo tools-lint-output\n");
    ctx.write_file(
        "rig.yaml",
        "tasks:\n  root-task:\n    command: echo root-out\n    working_dir: .\n",
    );

    let tasks = ctx.rig(&["tasks"]);
    let clean = strip_ansi(&tasks.stdout);
    assert!(clean.contains("tools.lint"), "tasks: {}", clean);
    assert!(clean.contains("root-task"), "tasks: {}", clean);

    let run = ctx.rig(&["run", "tools.lint"]);
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(
        run.stdout.contains("tools-lint-output"),
        "stdout: {}",
        run.stdout
    );
}

#[test]
fn depends_on_invalid_across_groups_errors() {
    let ctx = TestContext::new();
    ctx.write_file(
        "sub/rig.yaml",
        r#"
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
