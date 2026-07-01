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

#[test]
fn upward_search_discovers_ancestor_root() {
    // Run from a nested subdir with no config: upward search finds the ancestor
    // rig.yaml as the project root and lists its tasks.
    let ctx = TestContext::new();
    ctx.write_file(
        "rig.yaml",
        "tasks:\n  root-task:\n    command: echo root-out\n    working_dir: /tmp\n",
    );
    ctx.write_file("nested/deep/placeholder.txt", "");

    let tasks = ctx.rig_in("nested/deep", &["tasks"]);
    assert_eq!(tasks.code, 0, "stderr: {}", tasks.stderr);
    let clean = strip_ansi(&tasks.stdout);
    assert!(clean.contains("root-task"), "tasks: {}", clean);
}

#[test]
fn sibling_rig_files_compose_same_level() {
    // rig.yaml + extra.rig.yaml in one dir compose into the same (root) group:
    // both files' tasks and services surface together.
    let ctx = TestContext::new();
    ctx.write_file(
        "rig.yaml",
        "tasks:\n  build-main:\n    command: echo main\n    working_dir: /tmp\n",
    );
    ctx.write_file(
        "extra.rig.yaml",
        "tasks:\n  build-extra:\n    command: echo extra\n    working_dir: /tmp\n\
services:\n  svc-extra:\n    command: sh -c \"echo svc; sleep 30\"\n    working_dir: /tmp\n",
    );

    let tasks = ctx.rig(&["tasks"]);
    assert_eq!(tasks.code, 0, "stderr: {}", tasks.stderr);
    let clean = strip_ansi(&tasks.stdout);
    assert!(clean.contains("build-main"), "tasks: {}", clean);
    assert!(clean.contains("build-extra"), "tasks: {}", clean);

    let config = ctx.rig(&["config"]);
    assert_eq!(config.code, 0, "stderr: {}", config.stderr);
    assert!(
        config.stdout.contains("svc-extra"),
        "config: {}",
        config.stdout
    );
}

#[test]
fn authored_group_named_like_config_folder_errors() {
    // An authored group sharing its name with a config-bearing folder (not
    // adopted via `dir:`) is a hard error naming both resolutions — silently
    // dropping the folder's config was the bug.
    let ctx = TestContext::new();
    ctx.write_file(
        "web/rig.yaml",
        "tasks:\n  serve:\n    command: echo folder-serve\n    working_dir: /tmp\n",
    );
    ctx.write_file(
        "rig.yaml",
        r#"
groups:
  web:
    tasks:
      deploy:
        command: echo authored-deploy
        working_dir: /tmp
"#,
    );

    let result = ctx.rig(&["tasks"]);
    assert_eq!(result.code, 1, "stdout: {}", result.stdout);
    assert!(
        result.stderr.contains("Group 'web'"),
        "stderr: {}",
        result.stderr
    );
    assert!(
        result.stderr.contains("dir: ./web"),
        "error must offer adoption via dir:, stderr: {}",
        result.stderr
    );
    assert!(
        result.stderr.contains("rename one of them"),
        "error must offer renaming, stderr: {}",
        result.stderr
    );
}

#[test]
fn authored_group_adopting_same_named_folder_works() {
    // The sanctioned spelling of the case above: the authored group adopts the
    // same-named folder via `dir:` — no conflict, folder tasks scope under it.
    let ctx = TestContext::new();
    ctx.write_file(
        "web/rig.yaml",
        "tasks:\n  serve:\n    command: echo folder-serve\n    working_dir: /tmp\n",
    );
    ctx.write_file("rig.yaml", "groups:\n  web:\n    dir: ./web\n");

    let run = ctx.rig(&["run", "web.serve"]);
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(
        run.stdout.contains("folder-serve"),
        "stdout: {}",
        run.stdout
    );
}

#[test]
fn authored_group_named_like_plain_folder_works() {
    // A folder with NO rig file or Makefile never becomes a group, so an
    // authored group of the same name is not a conflict.
    let ctx = TestContext::new();
    ctx.write_file("web/placeholder.txt", "");
    ctx.write_file(
        "rig.yaml",
        r#"
groups:
  web:
    tasks:
      deploy:
        command: echo authored-deploy
        working_dir: /tmp
"#,
    );

    let run = ctx.rig(&["run", "web.deploy"]);
    assert_eq!(run.code, 0, "stderr: {}", run.stderr);
    assert!(
        run.stdout.contains("authored-deploy"),
        "stdout: {}",
        run.stdout
    );
}

// ============================================================================
// Duplicate service names across groups: legitimate (independent subprojects
// compose into one tree). Bare CLI refs resolve unique-or-error; the FQ dotted
// path is the identity.
// ============================================================================

/// Two sibling folder groups each defining a service `api`.
fn write_duplicate_api_groups(ctx: &TestContext) {
    ctx.write_file(
        "group-a/rig.yaml",
        r#"
services:
  api:
    command: sh -c "echo from-a; sleep 30"
    working_dir: /tmp
"#,
    );
    ctx.write_file(
        "group-b/rig.yaml",
        r#"
services:
  api:
    command: sh -c "echo from-b; sleep 30"
    working_dir: /tmp
"#,
    );
}

#[test]
fn duplicate_service_bare_name_start_is_ambiguous() {
    // `rig start api` with `api` in two groups must error listing both FQ
    // paths, never silently pick one.
    let ctx = TestContext::new();
    write_duplicate_api_groups(&ctx);

    let result = ctx.rig(&["start", "-d", "api"]);
    assert_eq!(result.code, 1, "stdout: {}", result.stdout);
    assert!(
        result.stderr.contains("Ambiguous service 'api'"),
        "stderr: {}",
        result.stderr
    );
    assert!(
        result.stderr.contains("group-a.api") && result.stderr.contains("group-b.api"),
        "error must list both FQ paths, stderr: {}",
        result.stderr
    );
    assert!(!session_exists("api", "group-a"));
    assert!(!session_exists("api", "group-b"));
}

#[test]
fn duplicate_service_dotted_path_starts_only_that_one() {
    let ctx = TestContext::new();
    write_duplicate_api_groups(&ctx);

    let result = ctx.rig(&["start", "-d", "group-a.api"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("Started api"),
        "stdout: {}",
        result.stdout
    );
    assert!(session_exists("api", "group-a"));
    assert!(!session_exists("api", "group-b"));
}

#[test]
fn duplicate_service_config_shows_both() {
    // No silent collapse: `rig config` lists the service under both groups.
    let ctx = TestContext::new();
    write_duplicate_api_groups(&ctx);

    let result = ctx.rig(&["config"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    let clean = strip_ansi(&result.stdout);
    let api_lines: Vec<&str> = clean.lines().filter(|l| l.contains("api")).collect();
    assert!(
        api_lines.iter().any(|l| l.contains("group-a"))
            && api_lines.iter().any(|l| l.contains("group-b")),
        "config must show api in both groups: {}",
        clean
    );
}

#[test]
fn duplicate_service_start_all_starts_both() {
    // Startup ordering is keyed by FQ path, so a bare-name collision must not
    // swallow one of the services.
    let ctx = TestContext::new();
    write_duplicate_api_groups(&ctx);

    let result = ctx.rig(&["start", "-d"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(session_exists("api", "group-a"));
    assert!(session_exists("api", "group-b"));

    // Both surface as distinct rows in ps.
    let ps = ctx.rig(&["ps"]);
    let clean = strip_ansi(&ps.stdout);
    let running_api = clean
        .lines()
        .filter(|l| l.contains("api") && l.contains("running"))
        .count();
    assert_eq!(running_api, 2, "ps stdout: {}", clean);
}

#[test]
fn nested_group_service_session_uses_flattened_path() {
    // A service in a nested group (dotted path `sub.inner`) gets a tmux session
    // named by the '-'-flattened path — tmux rejects '.' in session names.
    let ctx = TestContext::new();
    ctx.write_file(
        "sub/inner/rig.yaml",
        r#"
services:
  deep-svc:
    command: sh -c "echo deep; sleep 30"
    working_dir: /tmp
"#,
    );

    let result = ctx.rig(&["start", "-d", "sub.inner.deep-svc"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(session_exists("deep-svc", "sub-inner"));

    let ps = ctx.rig(&["ps"]);
    let clean = strip_ansi(&ps.stdout);
    assert!(
        clean
            .lines()
            .any(|l| l.contains("deep-svc") && l.contains("running")),
        "ps stdout: {}",
        clean
    );

    let stop = ctx.rig(&["stop", "sub.inner.deep-svc"]);
    assert_eq!(stop.code, 0, "stderr: {}", stop.stderr);
    delay_ms(200);
    assert!(!session_exists("deep-svc", "sub-inner"));
}

#[test]
fn dir_group_inline_task_duplicating_dir_rig_task_errors() {
    // A `dir:` group's inline task colliding with a rig task of the same name
    // from the dir's own rig file is a hard error naming the full task path —
    // rig-vs-rig never silently keeps both (only make targets are displaced).
    let ctx = TestContext::new();
    ctx.write_file(
        "svc/rig.yaml",
        "tasks:\n  build:\n    command: echo dir-build\n    working_dir: /tmp\n",
    );
    ctx.write_file(
        "rig.yaml",
        r#"
groups:
  api:
    dir: ./svc
    tasks:
      build:
        command: echo inline-build
        working_dir: /tmp
"#,
    );

    let result = ctx.rig(&["tasks"]);
    assert_eq!(result.code, 1, "stdout: {}", result.stdout);
    assert!(
        result.stderr.contains("Duplicate task 'api.build'"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn dir_group_inline_service_duplicating_dir_rig_service_errors() {
    // Same hard error for services: a `dir:` group's inline service colliding
    // with the dir rig file's service of the same name names the full path.
    let ctx = TestContext::new();
    ctx.write_file(
        "svc/rig.yaml",
        "services:\n  worker:\n    command: sh -c \"echo dir-worker; sleep 30\"\n    working_dir: /tmp\n",
    );
    ctx.write_file(
        "rig.yaml",
        r#"
groups:
  api:
    dir: ./svc
    services:
      worker:
        command: sh -c "echo inline-worker; sleep 30"
        working_dir: /tmp
"#,
    );

    let result = ctx.rig(&["config"]);
    assert_eq!(result.code, 1, "stdout: {}", result.stdout);
    assert!(
        result.stderr.contains("Duplicate service 'api.worker'"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn sibling_duplicate_task_name_errors() {
    // The same task name in two sibling rig files in one dir is a hard error —
    // the conflict is named, not silently dropped.
    let ctx = TestContext::new();
    ctx.write_file(
        "rig.yaml",
        "tasks:\n  dup:\n    command: echo one\n    working_dir: /tmp\n",
    );
    ctx.write_file(
        "extra.rig.yaml",
        "tasks:\n  dup:\n    command: echo two\n    working_dir: /tmp\n",
    );

    let result = ctx.rig(&["tasks"]);
    assert_eq!(result.code, 1, "stdout: {}", result.stdout);
    assert!(
        result.stderr.contains("Duplicate task 'dup'"),
        "stderr: {}",
        result.stderr
    );
}
