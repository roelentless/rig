mod common;

use common::*;

#[test]
fn schema_catches_unknown_service_keys() {
    let ctx = TestContext::new();
    ctx.write_file(
        "rig.yaml",
        &format!(
            r#"
groups:
  {}:
    services:
      api:
        command: echo hello
        working_dir: /tmp
        env:
          MY_VAR: test
"#,
            TEST_GROUP
        ),
    );

    let result = ctx.rig(&["start", "-d"]);
    assert_eq!(result.code, 1);
    assert!(
        result.stderr.contains("Unknown key 'env'"),
        "stderr: {}",
        result.stderr
    );
    assert!(
        result.stderr.contains("Valid keys:"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn schema_catches_unknown_requirement_keys() {
    let ctx = TestContext::new();
    ctx.write_file(
        "rig.yaml",
        &format!(
            r#"
groups:
  {}:
    services:
      req-bad:
        command: echo hello
        working_dir: /tmp
        requirements:
          - check: "true"
            command: "true"
            timeout: 30
"#,
            TEST_GROUP
        ),
    );

    let result = ctx.rig(&["start", "-d"]);
    assert_eq!(result.code, 1);
    assert!(
        result.stderr.contains("Unknown key 'timeout'"),
        "stderr: {}",
        result.stderr
    );
    assert!(
        result.stderr.contains("requirement"),
        "stderr: {}",
        result.stderr
    );
}
