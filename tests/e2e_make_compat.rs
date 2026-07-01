//! Makefile-compatibility suite: proves rig faithfully reproduces common `make`
//! behavior when running Makefile targets — exit codes, env forwarding, streams,
//! working directory, arg passthrough, and signal-driven cancellation cleanup.
//!
//! Fixtures live under `tests/fixtures/make/` (committed) and are copied into each
//! test's temp project so rig discovers them from CWD.

mod common;

use common::*;

use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Read a committed fixture file under `tests/fixtures/make/`.
fn fixture(rel: &str) -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/make")
        .join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read fixture {}: {}", p.display(), e))
}

// ============================================================================
// 1. EXIT CODES
// ============================================================================

#[test]
fn exit_code_success_is_zero() {
    let ctx = TestContext::new();
    ctx.write_file("Makefile", &fixture("basic/Makefile"));

    let r = ctx.rig(&["run", "ok"]);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert!(r.stdout.contains("ok-ran"), "stdout: {}", r.stdout);
}

#[test]
fn exit_code_recipe_failure_surfaced() {
    // The recipe does `exit 7`; GNU make surfaces that as its own exit code 2.
    // rig must pass make's real exit through — not swallow it, not report 0.
    let ctx = TestContext::new();
    ctx.write_file("Makefile", &fixture("basic/Makefile"));

    let r = ctx.rig(&["run", "boom"]);
    assert_ne!(
        r.code, 0,
        "recipe failure must be non-zero; stdout: {} stderr: {}",
        r.stdout, r.stderr
    );
    assert_eq!(
        r.code, 2,
        "GNU make surfaces exit 2 on recipe failure; got {} stderr: {}",
        r.code, r.stderr
    );
}

#[test]
fn exit_code_unknown_target_nonzero() {
    // rig resolves against discovered targets, so an unknown name fails at rig's
    // own resolution (Unknown task) with a non-zero exit — it never silently
    // succeeds. (rig never reaches make's own "No rule to make target" here.)
    let ctx = TestContext::new();
    ctx.write_file("Makefile", &fixture("basic/Makefile"));

    let r = ctx.rig(&["run", "does-not-exist"]);
    assert_ne!(r.code, 0, "unknown target must be non-zero");
    assert!(r.stderr.contains("Unknown task"), "stderr: {}", r.stderr);
}

// ============================================================================
// 2. ENV FORWARDING
// ============================================================================

#[test]
fn env_group_ancestor_ambient_and_make_var_reach_recipe() {
    // One recipe echoes four sources at once:
    //   ANCESTOR_VAR — root-level group env, reaching a nested group's make target
    //                  by ancestor cascade;
    //   GROUP_VAR    — the immediate group's own `environment:`;
    //   AMBIENT_VAR  — inherited from rig's own process environment;
    //   NAME         — a make variable set via `-- NAME=world`.
    let ctx = TestContext::new();
    ctx.write_file("rig.yaml", &fixture("env/rig.yaml"));
    ctx.write_file("mk/Makefile", &fixture("env/mk/Makefile"));

    let r = ctx.rig_envs(
        &[("AMBIENT_VAR", "from-ambient")],
        &["run", "svc.greet", "--", "NAME=world"],
    );
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    let out = &r.stdout;
    assert!(
        out.contains("ANCESTOR=from-root"),
        "ancestor group env missing: {}",
        out
    );
    assert!(
        out.contains("GROUP=from-group"),
        "group env missing: {}",
        out
    );
    assert!(
        out.contains("AMBIENT=from-ambient"),
        "ambient env missing: {}",
        out
    );
    assert!(
        out.contains("NAME=world"),
        "make var via -- missing: {}",
        out
    );
}

// ============================================================================
// 3. STREAMS
// ============================================================================

#[test]
fn streams_stdout_and_stderr_both_captured() {
    let ctx = TestContext::new();
    ctx.write_file("Makefile", &fixture("basic/Makefile"));

    let r = ctx.rig(&["run", "streams"]);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    assert!(
        r.stdout.contains("out-line"),
        "stdout missing out-line: {}",
        r.stdout
    );
    assert!(
        r.stderr.contains("err-line"),
        "stderr missing err-line: {}",
        r.stderr
    );
}

#[test]
fn streams_silenced_vs_echoed_recipe() {
    let ctx = TestContext::new();
    ctx.write_file("Makefile", &fixture("basic/Makefile"));

    // `@`-silenced: make does not print the command line, only its output.
    let quiet = ctx.rig(&["run", "quiet"]);
    assert_eq!(quiet.code, 0, "stderr: {}", quiet.stderr);
    assert!(
        quiet.stdout.contains("quiet-line"),
        "stdout: {}",
        quiet.stdout
    );
    assert!(
        !quiet.stdout.contains("echo quiet-line"),
        "silenced recipe must not echo the command: {}",
        quiet.stdout
    );

    // No `@`: make echoes the recipe command line to stdout before its output.
    let loud = ctx.rig(&["run", "loud"]);
    assert_eq!(loud.code, 0, "stderr: {}", loud.stderr);
    assert!(loud.stdout.contains("loud-line"), "stdout: {}", loud.stdout);
    assert!(
        loud.stdout.contains("echo loud-line"),
        "echoed recipe must print the command line: {}",
        loud.stdout
    );
}

// ============================================================================
// 4. WORKING DIR
// ============================================================================

#[test]
fn working_dir_subfolder_runs_in_place() {
    // A folder-namespaced target runs in its own Makefile's directory.
    let ctx = TestContext::new();
    ctx.write_file("Makefile", &fixture("workdir/Makefile"));
    ctx.write_file("probe/Makefile", &fixture("workdir/probe/Makefile"));

    let r = ctx.rig(&["run", "probe.pwd"]);
    assert_eq!(r.code, 0, "stderr: {}", r.stderr);
    // pwd may resolve symlinks (macOS /var -> /private/var), so match the leaf.
    assert!(
        r.stdout.trim().ends_with("/probe"),
        "recipe did not run in the subfolder: {}",
        r.stdout
    );
}

// ============================================================================
// 5. ARGS PASSTHROUGH
// ============================================================================

#[test]
fn args_passthrough_var_and_flag() {
    let ctx = TestContext::new();
    ctx.write_file("Makefile", &fixture("basic/Makefile"));

    // VAR=val passthrough sets a make variable used by the recipe.
    let var = ctx.rig(&["run", "greet", "--", "NAME=rig"]);
    assert_eq!(var.code, 0, "stderr: {}", var.stderr);
    assert!(
        var.stdout.contains("hello rig"),
        "VAR=val not passed: {}",
        var.stdout
    );

    // Flag passthrough: `-s` silences make, so the echoed recipe line vanishes
    // while the actual output remains — proving the flag reached make.
    let flag = ctx.rig(&["run", "loud", "--", "-s"]);
    assert_eq!(flag.code, 0, "stderr: {}", flag.stderr);
    assert!(
        flag.stdout.contains("loud-line"),
        "output missing: {}",
        flag.stdout
    );
    assert!(
        !flag.stdout.contains("echo loud-line"),
        "`-s` flag not passed through to make: {}",
        flag.stdout
    );
}

// ============================================================================
// 6. SIGNALS / CANCELLATION
// ============================================================================

fn wait_until<F: FnMut() -> bool>(mut cond: F, timeout: Duration) -> bool {
    let start = Instant::now();
    loop {
        if cond() {
            return true;
        }
        if start.elapsed() > timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// PIDs whose full command line matches `pattern` (portable across macOS and
/// Linux, incl. busybox — `pgrep -f` is available on all).
fn pgrep_f(pattern: &str) -> Vec<u32> {
    let out = Command::new("pgrep")
        .arg("-f")
        .arg(pattern)
        .output()
        .expect("pgrep not available");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.trim().parse::<u32>().ok())
        .collect()
}

fn kill_signal(pid: u32, signal: &str) {
    let _ = Command::new("kill")
        .arg(format!("-{}", signal))
        .arg(pid.to_string())
        .status();
}

/// Spawn `rig run <target>` (with MARKER/READY make vars plus `extra_vars`),
/// wait for the recipe child to be running, deliver `signal` to rig's PID only
/// (not the process group), then report whether rig terminated promptly and how
/// many recipe processes survived. Always cleans up. `ctx` is caller-owned so
/// tests can inspect recipe-written files afterwards.
fn run_cancellation_case_for(
    ctx: &TestContext,
    target: &str,
    signal: &str,
    extra_vars: &[String],
) -> (bool, usize) {
    ctx.write_file("Makefile", &fixture("signals/Makefile"));

    let nonce = format!(
        "RIGSIG_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let ready = ctx.path().join("recipe_ready");
    let marker_arg = format!("MARKER={}", nonce);
    let ready_arg = format!("READY={}", ready.display());

    let mut args = vec!["run", target, "--", &marker_arg, &ready_arg];
    args.extend(extra_vars.iter().map(String::as_str));

    let mut child = Command::new(rig_binary_path())
        .args(&args)
        .current_dir(ctx.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn rig");
    let rig_pid = child.id();

    // Race-free gate: READY appears only after the recipe starts, by which point
    // rig has already installed its SIGINT/SIGTERM handlers.
    let started = wait_until(|| ready.exists(), Duration::from_secs(15));
    assert!(started, "recipe never started for signal {}", signal);
    assert!(
        !pgrep_f(&nonce).is_empty(),
        "recipe process not found for signal {}",
        signal
    );

    kill_signal(rig_pid, signal);

    let rig_terminated = wait_until(
        || matches!(child.try_wait(), Ok(Some(_))),
        Duration::from_secs(15),
    );

    // Poll for the recipe subtree to disappear.
    let _ = wait_until(|| pgrep_f(&nonce).is_empty(), Duration::from_secs(15));
    let survivors = pgrep_f(&nonce);
    let survivor_count = survivors.len();

    // Teardown: never leak the long-lived recipe or a lingering rig.
    for pid in &survivors {
        kill_signal(*pid, "KILL");
    }
    if !rig_terminated {
        kill_signal(rig_pid, "KILL");
        let _ = child.wait();
    }

    (rig_terminated, survivor_count)
}

#[test]
fn cancellation_sigint_terminates_rig_without_orphaning_child() {
    let ctx = TestContext::new();
    let (terminated, survivors) = run_cancellation_case_for(&ctx, "sleeper", "INT", &[]);
    assert!(terminated, "rig did not terminate promptly on SIGINT");
    assert_eq!(
        survivors, 0,
        "SIGINT to rig left {} orphaned make/recipe process(es)",
        survivors
    );
}

#[test]
fn cancellation_sigterm_terminates_rig_without_orphaning_child() {
    let ctx = TestContext::new();
    let (terminated, survivors) = run_cancellation_case_for(&ctx, "sleeper", "TERM", &[]);
    assert!(terminated, "rig did not terminate promptly on SIGTERM");
    assert_eq!(
        survivors, 0,
        "SIGTERM to rig left {} orphaned make/recipe process(es)",
        survivors
    );
}

#[test]
fn cancellation_delivers_catchable_signal_before_kill() {
    // The recipe traps TERM/INT and touches $(GRACEFUL) before exiting. The
    // marker can only exist if cancellation forwarded a catchable signal to the
    // recipe — a straight SIGKILL sweep would never let the trap run. This is
    // the contract make's delete-partial-target cleanup depends on.
    let ctx = TestContext::new();
    let graceful = ctx.path().join("graceful_marker");
    let graceful_arg = format!("GRACEFUL={}", graceful.display());

    let (terminated, survivors) =
        run_cancellation_case_for(&ctx, "graceful", "TERM", &[graceful_arg]);
    assert!(terminated, "rig did not terminate promptly on SIGTERM");
    assert!(
        graceful.exists(),
        "recipe trap never ran — cancellation did not deliver a catchable signal"
    );
    assert_eq!(
        survivors, 0,
        "graceful cancellation left {} recipe process(es)",
        survivors
    );
}

#[test]
fn cancellation_kill_backstop_sweeps_stuck_child() {
    // The recipe ignores TERM/INT outright, so it survives the graceful phase;
    // only the SIGKILL backstop after the ~5s grace window reclaims it. The
    // generous waits inside the helper (15s) cover the full window.
    let ctx = TestContext::new();
    let (terminated, survivors) = run_cancellation_case_for(&ctx, "stubborn", "TERM", &[]);
    assert!(terminated, "rig did not terminate after the grace window");
    assert_eq!(
        survivors, 0,
        "SIGKILL backstop left {} stuck recipe process(es)",
        survivors
    );
}
