mod common;

use common::*;

/// A Makefile with .PHONY targets and inline ## descriptions (makex convention).
const BASIC_MAKEFILE: &str = "\
.PHONY: build test clean

build: ## compile the project
\t@echo build-output

test: ## run tests
\t@echo test-output

clean: ## remove artifacts
\t@echo clean-output
";

#[test]
fn zero_config_makefile_tasks_listed() {
    // Headline capability: a directory with ONLY a Makefile (no rig.yaml) lists
    // its targets under bare names.
    let ctx = TestContext::new();
    ctx.write_file("Makefile", BASIC_MAKEFILE);

    let result = ctx.rig(&["tasks"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    let clean = strip_ansi(&result.stdout);
    // Bare names, not group.target.
    assert!(clean.contains("build"), "stdout: {}", clean);
    assert!(clean.contains("test"), "stdout: {}", clean);
    assert!(clean.contains("clean"), "stdout: {}", clean);
    assert!(clean.contains("compile the project"), "stdout: {}", clean);
    assert!(clean.contains("run tests"), "stdout: {}", clean);
}

#[test]
fn zero_config_makefile_task_runs() {
    // Headline capability: `rig run <target>` executes with only a Makefile.
    let ctx = TestContext::new();
    ctx.write_file("Makefile", BASIC_MAKEFILE);

    let result = ctx.rig(&["run", "build"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("build-output"),
        "stdout: {}",
        result.stdout
    );
}

#[test]
fn makefile_task_args_passthrough() {
    let ctx = TestContext::new();
    ctx.write_file(
        "Makefile",
        "\
.PHONY: greet

## greet: say hello
greet:
\t@echo hello $(NAME)
",
    );

    let result = ctx.rig(&["run", "greet", "--", "NAME=world"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("hello world"),
        "stdout: {}",
        result.stdout
    );
}

#[test]
fn rigfile_task_overrides_makefile() {
    // A rig task and a make target share the short name `build`. Resolution tries
    // rig first (index 0), so the unambiguous rig task wins — make never runs.
    let ctx = TestContext::new();
    ctx.write_file(
        "Makefile",
        "\
.PHONY: build

## build: makefile build
build:
\t@echo makefile-build
",
    );
    ctx.write_file(
        "rig.yaml",
        r#"
groups:
  proj:
    tasks:
      build:
        command: echo rigfile-build
        working_dir: .
        description: overridden by rigfile
"#,
    );

    let result = ctx.rig(&["run", "build"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("rigfile-build"),
        "stdout: {}",
        result.stdout
    );
    assert!(
        !result.stdout.contains("makefile-build"),
        "should not run make: {}",
        result.stdout
    );
}

#[test]
fn makefile_without_phony_exposes_all_real_targets() {
    // makex semantics: .PHONY does not gate discovery. Every real target
    // surfaces (documented or not), so both `publish` and the undocumented
    // `_internal` appear under bare names.
    let ctx = TestContext::new();
    ctx.write_file(
        "Makefile",
        "\
publish: ## publish to registry
\t@echo published

_internal:
\t@echo internal
",
    );

    let result = ctx.rig(&["tasks"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    let clean = strip_ansi(&result.stdout);
    assert!(clean.contains("publish"), "stdout: {}", clean);
    assert!(
        clean.contains("_internal"),
        "undocumented target should now appear: {}",
        clean
    );
}

#[test]
fn makefile_and_rig_tasks_coexist() {
    // With both a Makefile and a rig.yaml present, `rig tasks` shows the union:
    // bare make targets alongside namespaced rig tasks.
    let ctx = TestContext::new();
    ctx.write_file(
        "Makefile",
        "\
.PHONY: build

## build: compile
build:
\t@echo built
",
    );
    ctx.write_file(
        "rig.yaml",
        r#"
groups:
  app:
    tasks:
      lint:
        command: echo linted
        working_dir: .
"#,
    );

    let result = ctx.rig(&["tasks"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    let clean = strip_ansi(&result.stdout);
    // Bare make target and namespaced rig task both appear.
    assert!(clean.contains("build"), "stdout: {}", clean);
    assert!(clean.contains("app.lint"), "stdout: {}", clean);
}

#[test]
fn subfolder_walk_namespaces_by_relative_path() {
    // A tree of Makefiles (no rig.yaml): CWD → bare names, subfolders → dotted
    // namespaces derived from their relative path.
    let ctx = TestContext::new();
    ctx.write_file("Makefile", "build:\n\t@echo root-build\n");
    ctx.write_file(
        "backend/Makefile",
        "build:\n\t@echo backend-build\n\nmigrate:\n\t@echo backend-migrate\n",
    );
    ctx.write_file(
        "apps/web/Makefile",
        "build:\n\t@echo web-build\n\nbundle:\n\t@echo web-bundle\n",
    );

    let result = ctx.rig(&["tasks"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    let clean = strip_ansi(&result.stdout);
    assert!(clean.contains("build"), "stdout: {}", clean);
    assert!(clean.contains("backend.build"), "stdout: {}", clean);
    assert!(clean.contains("backend.migrate"), "stdout: {}", clean);
    assert!(clean.contains("apps.web.build"), "stdout: {}", clean);
    assert!(clean.contains("apps.web.bundle"), "stdout: {}", clean);
}

#[test]
fn subfolder_dotted_run_executes_that_makefile() {
    // A fully-qualified name runs the target in its own Makefile's folder.
    let ctx = TestContext::new();
    ctx.write_file("Makefile", "build:\n\t@echo root-build\n");
    ctx.write_file(
        "backend/Makefile",
        "migrate:\n\t@echo backend-migrate-output\n",
    );

    let result = ctx.rig(&["run", "backend.migrate"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("backend-migrate-output"),
        "stdout: {}",
        result.stdout
    );
}

#[test]
fn subfolder_short_name_in_two_folders_is_ambiguous() {
    // `build` exists in the root and two subfolders — a short name is ambiguous
    // across folder namespaces and must error rather than pick one.
    let ctx = TestContext::new();
    ctx.write_file("Makefile", "build:\n\t@echo root-build\n");
    ctx.write_file("backend/Makefile", "build:\n\t@echo backend-build\n");
    ctx.write_file("apps/web/Makefile", "build:\n\t@echo web-build\n");

    let result = ctx.rig(&["run", "build"]);
    assert_eq!(result.code, 1, "stdout: {}", result.stdout);
    assert!(
        result.stderr.contains("Ambiguous task"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn gitignored_makefile_is_not_discovered() {
    // A Makefile under a gitignored directory must be excluded from the walk.
    let ctx = TestContext::new();
    ctx.write_file("Makefile", "build:\n\t@echo root-build\n");
    ctx.write_file("ignored/Makefile", "secret:\n\t@echo secret-target\n");
    ctx.write_file(".gitignore", "ignored/\n");

    let result = ctx.rig(&["tasks"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    let clean = strip_ansi(&result.stdout);
    assert!(clean.contains("build"), "stdout: {}", clean);
    assert!(
        !clean.contains("secret"),
        "gitignored Makefile leaked: {}",
        clean
    );
    assert!(
        !clean.contains("ignored."),
        "gitignored namespace leaked: {}",
        clean
    );
}

#[test]
fn hidden_dir_makefile_is_not_discovered() {
    // A Makefile inside a hidden directory (e.g. .git, .cache) must be skipped at
    // any depth — the walk ignores dot-dirs.
    let ctx = TestContext::new();
    ctx.write_file("Makefile", "build:\n\t@echo root-build\n");
    ctx.write_file(".hidden/Makefile", "secret:\n\t@echo secret-target\n");
    ctx.write_file("nested/.git/Makefile", "reflog:\n\t@echo reflog-target\n");

    let result = ctx.rig(&["tasks"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    let clean = strip_ansi(&result.stdout);
    assert!(clean.contains("build"), "stdout: {}", clean);
    assert!(
        !clean.contains("secret") && !clean.contains("reflog"),
        "hidden-dir Makefile leaked: {}",
        clean
    );
}

/// The listing row for `name` (matched on the leading path column, marker and
/// color already stripped from the front by `trim_start`). Panics if absent.
fn row_for<'a>(clean: &'a str, name: &str) -> &'a str {
    clean
        .lines()
        .find(|l| l.trim_start_matches(['→', ' ']).starts_with(name))
        .unwrap_or_else(|| panic!("no listing row for '{}' in:\n{}", name, clean))
}

#[test]
fn makefile_explicit_default_goal_marked() {
    // `.DEFAULT_GOAL := test` marks `test` with `→`; the others get no arrow.
    let ctx = TestContext::new();
    ctx.write_file(
        "Makefile",
        "\
.DEFAULT_GOAL := test

build:
\t@echo build

test:
\t@echo test

deploy:
\t@echo deploy
",
    );

    let result = ctx.rig(&["tasks"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    let clean = strip_ansi(&result.stdout);

    assert!(
        row_for(&clean, "test").starts_with('→'),
        "default goal 'test' not marked: {}",
        clean
    );
    assert!(
        !row_for(&clean, "build").starts_with('→'),
        "non-default 'build' wrongly marked: {}",
        clean
    );
    assert!(
        !row_for(&clean, "deploy").starts_with('→'),
        "non-default 'deploy' wrongly marked: {}",
        clean
    );
}

#[test]
fn makefile_default_goal_falls_back_to_first_target() {
    // No `.DEFAULT_GOAL` → the first target in file order is the default goal.
    let ctx = TestContext::new();
    ctx.write_file(
        "Makefile",
        "\
build:
\t@echo build

test:
\t@echo test
",
    );

    let result = ctx.rig(&["tasks"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    let clean = strip_ansi(&result.stdout);

    assert!(
        row_for(&clean, "build").starts_with('→'),
        "first target 'build' not marked: {}",
        clean
    );
    assert!(
        !row_for(&clean, "test").starts_with('→'),
        "non-default 'test' wrongly marked: {}",
        clean
    );
}

#[test]
fn rig_tasks_never_marked_default_goal() {
    // The `→` marker is a make-only concept; rig-authored tasks never get it.
    let ctx = TestContext::new();
    ctx.write_file(
        "rig.yaml",
        r#"
groups:
  app:
    working_dir: .
    tasks:
      build:
        command: echo build
      test:
        command: echo test
"#,
    );

    let result = ctx.rig(&["tasks"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    let clean = strip_ansi(&result.stdout);
    assert!(
        !clean.contains('→'),
        "rig tasks must never be marked with →: {}",
        clean
    );
}

#[test]
fn group_working_dir_inherited_by_task() {
    // Group-level working_dir remains the default working dir for group tasks
    // that omit their own (unchanged by the make-provider work).
    let ctx = TestContext::new();
    ctx.write_file(
        "rig.yaml",
        r#"
groups:
  proj:
    working_dir: .
    tasks:
      hello:
        command: echo hello-from-task
"#,
    );

    let result = ctx.rig(&["run", "proj.hello"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("hello-from-task"),
        "stdout: {}",
        result.stdout
    );
}

#[test]
fn upward_search_makefile_ancestor_root() {
    // Run from a nested subdir with no config: upward search finds the ancestor
    // Makefile as the project root and lists its targets.
    let ctx = TestContext::new();
    ctx.write_file("Makefile", "root-target:\n\t@echo root-target-out\n");
    ctx.write_file("nested/deep/placeholder.txt", "");

    let tasks = ctx.rig_in("nested/deep", &["tasks"]);
    assert_eq!(tasks.code, 0, "stderr: {}", tasks.stderr);
    let clean = strip_ansi(&tasks.stdout);
    assert!(clean.contains("root-target"), "tasks: {}", clean);
}

#[test]
fn nearest_root_wins_over_farther() {
    // A closer ancestor with a Makefile stops the upward search before a farther
    // rig.yaml: the root becomes `mid`, so `mid`'s target lists and the farther
    // rig.yaml's task is out of scope.
    let ctx = TestContext::new();
    ctx.write_file(
        "rig.yaml",
        "tasks:\n  far-task:\n    command: echo far\n    working_dir: /tmp\n",
    );
    ctx.write_file("mid/Makefile", "mid-target:\n\t@echo mid-out\n");
    ctx.write_file("mid/deep/placeholder.txt", "");

    let tasks = ctx.rig_in("mid/deep", &["tasks"]);
    assert_eq!(tasks.code, 0, "stderr: {}", tasks.stderr);
    let clean = strip_ansi(&tasks.stdout);
    assert!(
        clean.contains("mid-target"),
        "nearest root should be mid: {}",
        clean
    );
    assert!(
        !clean.contains("far-task"),
        "farther rig.yaml must not be the root: {}",
        clean
    );
}

#[test]
fn tasks_group_filter_scopes_whole_subtree() {
    // `-g sub` scopes the entire sub.* subtree, not just the exact `sub` level:
    // both sub/Makefile and sub/deep/Makefile targets appear; the root target
    // does not. A leaf-group filter (`-g sub.deep`) still matches itself.
    let ctx = TestContext::new();
    ctx.write_file("Makefile", "rootonly:\n\t@echo hello-root\n");
    ctx.write_file("sub/Makefile", "buildsub:\n\t@echo sub-build\n");
    ctx.write_file("sub/deep/Makefile", "buildeep:\n\t@echo deep-build\n");

    let result = ctx.rig(&["tasks", "-g", "sub"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    let clean = strip_ansi(&result.stdout);
    assert!(clean.contains("sub.buildsub"), "stdout: {}", clean);
    assert!(clean.contains("sub.deep.buildeep"), "stdout: {}", clean);
    assert!(!clean.contains("rootonly"), "root leaked: {}", clean);

    // Exact leaf-group filter still matches itself and excludes the parent.
    let deep = ctx.rig(&["tasks", "-g", "sub.deep"]);
    assert_eq!(deep.code, 0, "stderr: {}", deep.stderr);
    let dclean = strip_ansi(&deep.stdout);
    assert!(dclean.contains("sub.deep.buildeep"), "stdout: {}", dclean);
    assert!(!dclean.contains("sub.buildsub"), "stdout: {}", dclean);
}

#[test]
fn config_json_emits_group_tree_with_task_sources() {
    // config --json serializes the actual group tree: root-level units at the
    // top (no "" wrapper), tasks included with a `source` field distinguishing
    // rig-authored tasks from Makefile-sourced targets.
    let ctx = TestContext::new();
    ctx.write_file(
        "rig.yaml",
        "tasks:\n  deploy:\n    command: echo deploy\n    working_dir: .\n",
    );
    ctx.write_file("Makefile", "build:\n\t@echo build\n");

    let result = ctx.rig(&["config", "--json"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    let clean = strip_ansi(&result.stdout);
    let cfg: serde_json::Value = serde_json::from_str(&clean).expect("valid JSON");

    assert!(
        cfg.get("").is_none(),
        "unexpected empty-string root key: {}",
        clean
    );
    assert_eq!(cfg["tasks"]["deploy"]["source"], serde_json::json!("rig"));
    assert_eq!(cfg["tasks"]["build"]["source"], serde_json::json!("make"));
}
