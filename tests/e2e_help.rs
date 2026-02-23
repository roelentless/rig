mod common;

use common::*;

#[test]
fn help_shows_usage() {
    let ctx = TestContext::new();
    let result = ctx.rig(&["help"]);
    assert_eq!(result.code, 0);
    assert!(!result.stdout.is_empty(), "expected help output");
}

#[test]
fn dash_h_shows_help() {
    let ctx = TestContext::new();
    let result = ctx.rig(&["-h"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(!result.stdout.is_empty(), "expected output from -h");
}

#[test]
fn dash_dash_help_shows_help() {
    let ctx = TestContext::new();
    let result = ctx.rig(&["--help"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(!result.stdout.is_empty(), "expected output from --help");
}

#[test]
fn no_args_shows_help() {
    let ctx = TestContext::new();
    let result = ctx.rig(&[]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(!result.stdout.is_empty(), "expected output with no args");
}

#[test]
fn version_shows_output() {
    let ctx = TestContext::new();
    let result = ctx.rig(&["version"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(!result.stdout.trim().is_empty(), "expected version string");
}

#[test]
fn all_help_variants_show_same_output() {
    let ctx = TestContext::new();
    let help = strip_ansi(&ctx.rig(&["help"]).stdout);
    let dash_h = strip_ansi(&ctx.rig(&["-h"]).stdout);
    let dash_dash = strip_ansi(&ctx.rig(&["--help"]).stdout);
    let no_args = strip_ansi(&ctx.rig(&[]).stdout);

    assert_eq!(help, dash_h, "help and -h differ");
    assert_eq!(help, dash_dash, "help and --help differ");
    assert_eq!(help, no_args, "help and no-args differ");
}
