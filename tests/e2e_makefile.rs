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
