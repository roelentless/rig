mod common;

use common::*;

#[test]
fn requirements_check_passes_skips_remediation() {
    let ctx = TestContext::new();
    let _ = std::fs::remove_file("/tmp/rig-req-test-marker");

    ctx.write_file(
        "rig.yaml",
        &format!(
            r#"
groups:
  {}:
    services:
      req-pass:
        command: sh -c "echo 'started'; sleep 30"
        working_dir: /tmp
        requirements:
          - check: "true"
            command: "echo 'should not run' > /tmp/rig-req-test-marker"
"#,
            TEST_GROUP
        ),
    );

    let result = ctx.rig(&["start", "-d", "req-pass"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("Started req-pass"),
        "stdout: {}",
        result.stdout
    );

    // Remediation should NOT have run
    assert!(
        !std::path::Path::new("/tmp/rig-req-test-marker").exists(),
        "Remediation should not have run when check passes"
    );
    let _ = std::fs::remove_file("/tmp/rig-req-test-marker");
}

#[test]
fn requirements_check_fails_remediation_runs() {
    let ctx = TestContext::new();
    let _ = std::fs::remove_file("/tmp/rig-req-remediated");

    ctx.write_file(
        "rig.yaml",
        &format!(
            r#"
groups:
  {}:
    services:
      req-fix:
        command: sh -c "echo 'started'; sleep 30"
        working_dir: /tmp
        requirements:
          - check: test -f /tmp/rig-req-remediated
            command: touch /tmp/rig-req-remediated
"#,
            TEST_GROUP
        ),
    );

    let result = ctx.rig(&["start", "-d", "req-fix"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        result.stdout.contains("Started req-fix"),
        "stdout: {}",
        result.stdout
    );

    assert!(
        std::path::Path::new("/tmp/rig-req-remediated").exists(),
        "Remediation should have created marker file"
    );
    let _ = std::fs::remove_file("/tmp/rig-req-remediated");
}

#[test]
fn requirements_remediation_failure_aborts() {
    let ctx = TestContext::new();
    ctx.write_file(
        "rig.yaml",
        &format!(
            r#"
groups:
  {}:
    services:
      req-fail:
        command: sh -c "echo 'started'; sleep 30"
        working_dir: /tmp
        requirements:
          - check: "false"
            command: "exit 1"
"#,
            TEST_GROUP
        ),
    );

    let result = ctx.rig(&["start", "-d", "req-fail"]);
    assert_eq!(result.code, 1);
    assert!(
        result.stderr.contains("Requirement remediation failed"),
        "stderr: {}",
        result.stderr
    );
}
