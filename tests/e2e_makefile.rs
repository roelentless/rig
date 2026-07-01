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
fn makefile_tasks_auto_discovered() {
    let ctx = TestContext::new();
    ctx.write_file("Makefile", BASIC_MAKEFILE);
    ctx.write_file(
        "rig.yaml",
        r#"
groups:
  proj:
    working_dir: .
"#,
    );

    let result = ctx.rig(&["tasks"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    let clean = strip_ansi(&result.stdout);
    assert!(clean.contains("proj.build"), "stdout: {}", clean);
    assert!(clean.contains("proj.test"), "stdout: {}", clean);
    assert!(clean.contains("proj.clean"), "stdout: {}", clean);
    assert!(clean.contains("compile the project"), "stdout: {}", clean);
    assert!(clean.contains("run tests"), "stdout: {}", clean);
}

#[test]
fn makefile_task_runs() {
    let ctx = TestContext::new();
    ctx.write_file("Makefile", BASIC_MAKEFILE);
    ctx.write_file(
        "rig.yaml",
        r#"
groups:
  proj:
    working_dir: .
"#,
    );

    let result = ctx.rig(&["run", "proj.build"]);
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
    ctx.write_file(
        "rig.yaml",
        r#"
groups:
  proj:
    working_dir: .
"#,
    );

    let result = ctx.rig(&["run", "proj.greet", "--", "NAME=world"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("hello world"),
        "stdout: {}",
        result.stdout
    );
}

#[test]
fn rigfile_task_overrides_makefile() {
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
    working_dir: .
    tasks:
      build:
        command: echo rigfile-build
        description: overridden by rigfile
"#,
    );

    let result = ctx.rig(&["run", "proj.build"]);
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
fn makefile_explicit_path() {
    let ctx = TestContext::new();
    ctx.write_file(
        "tools/build.mk",
        "\
.PHONY: package

## package: create distributable package
package:
\t@echo packaged
",
    );
    ctx.write_file(
        "rig.yaml",
        r#"
groups:
  proj:
    working_dir: .
    makefiles:
      - ./tools/build.mk
"#,
    );

    let result = ctx.rig(&["tasks"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    let clean = strip_ansi(&result.stdout);
    assert!(clean.contains("proj.package"), "stdout: {}", clean);

    let result = ctx.rig(&["run", "proj.package"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("packaged"),
        "stdout: {}",
        result.stdout
    );
}

#[test]
fn group_working_dir_inherited_by_task() {
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
fn makefile_without_phony_exposes_all_real_targets() {
    // makex semantics: .PHONY does not gate discovery. Every real target
    // surfaces (documented or not), so both `publish` and the undocumented
    // `_internal` appear — the old `_`-prefix hiding convention is gone.
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
    ctx.write_file(
        "rig.yaml",
        r#"
groups:
  proj:
    working_dir: .
"#,
    );

    let result = ctx.rig(&["tasks"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    let clean = strip_ansi(&result.stdout);
    assert!(clean.contains("proj.publish"), "stdout: {}", clean);
    assert!(
        clean.contains("_internal"),
        "undocumented target should now appear: {}",
        clean
    );
}

#[test]
fn makefile_and_services_in_same_group() {
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
    working_dir: .
    services:
      server:
        command: sh -c "echo started; sleep 30"
        working_dir: .
    tasks:
      lint:
        command: echo linted
"#,
    );

    let result = ctx.rig(&["tasks"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    let clean = strip_ansi(&result.stdout);
    // Both Makefile task and rigfile task appear under the same group
    assert!(clean.contains("app.build"), "stdout: {}", clean);
    assert!(clean.contains("app.lint"), "stdout: {}", clean);
}
