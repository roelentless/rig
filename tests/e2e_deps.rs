mod common;

use common::*;

#[test]
fn depends_on_ordering() {
    let ctx = TestContext::new();
    ctx.write_file(
        "rig.yaml",
        &format!(
            r#"
groups:
  {}:
    services:
      db:
        command: sh -c "echo 'db started'; sleep 30"
        working_dir: /tmp
        healthcheck:
          grace_ms: 200

      api:
        command: sh -c "echo 'api started'; sleep 30"
        working_dir: /tmp
        depends_on: [db]

      worker:
        command: sh -c "echo 'worker started'; sleep 30"
        working_dir: /tmp
        depends_on: [db]
"#,
            TEST_GROUP
        ),
    );

    let result = ctx.rig(&["start", "-d"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);

    let stdout = &result.stdout;
    let db_idx = stdout.find("Started db").expect("db should be started");
    let api_idx = stdout.find("Started api").expect("api should be started");
    let worker_idx = stdout
        .find("Started worker")
        .expect("worker should be started");

    assert!(db_idx < api_idx, "db should start before api");
    assert!(db_idx < worker_idx, "db should start before worker");

    assert!(session_exists("db", TEST_GROUP));
    assert!(session_exists("api", TEST_GROUP));
    assert!(session_exists("worker", TEST_GROUP));
}

#[test]
fn healthcheck_grace_ms_delays() {
    let ctx = TestContext::new();
    ctx.write_file(
        "rig.yaml",
        &format!(
            r#"
groups:
  {}:
    services:
      slow-db:
        command: sh -c "echo 'slow-db started'; sleep 30"
        working_dir: /tmp
        healthcheck:
          grace_ms: 300

      client:
        command: sh -c "echo 'client started'; sleep 30"
        working_dir: /tmp
        depends_on: [slow-db]
"#,
            TEST_GROUP
        ),
    );

    let start = std::time::Instant::now();
    let result = ctx.rig(&["start", "-d"]);
    let elapsed = start.elapsed().as_millis();

    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(
        elapsed >= 200,
        "Expected at least 200ms delay, got {}ms",
        elapsed
    );

    assert!(session_exists("slow-db", TEST_GROUP));
    assert!(session_exists("client", TEST_GROUP));
}

#[test]
fn invalid_depends_on_errors() {
    let ctx = TestContext::new();
    ctx.write_file(
        "rig.yaml",
        &format!(
            r#"
groups:
  {}:
    services:
      api:
        command: sh -c "echo 'api'; sleep 30"
        working_dir: /tmp
        depends_on: [nonexistent]
"#,
            TEST_GROUP
        ),
    );

    let result = ctx.rig(&["start", "-d"]);
    assert_eq!(result.code, 1);
    assert!(
        result.stderr.contains("depends on unknown service"),
        "stderr: {}",
        result.stderr
    );
}

#[test]
fn depends_on_same_group_wins_over_duplicate_elsewhere() {
    // `db` exists in both groups; group-a's api depends on bare `db`, which
    // must resolve to its own group's db — no ambiguity error.
    let ctx = TestContext::new();
    ctx.write_file(
        "group-a/rig.yaml",
        r#"
services:
  db:
    command: sh -c "echo a-db; sleep 30"
    working_dir: /tmp
    healthcheck:
      grace_ms: 100
  api:
    command: sh -c "echo a-api; sleep 30"
    working_dir: /tmp
    depends_on: [db]
"#,
    );
    ctx.write_file(
        "group-b/rig.yaml",
        r#"
services:
  db:
    command: sh -c "echo b-db; sleep 30"
    working_dir: /tmp
"#,
    );

    let result = ctx.rig(&["start", "-d"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);

    let api_idx = result.stdout.find("Started api").expect("api started");
    let db_idx = result.stdout.find("Started db").expect("db started");
    assert!(db_idx < api_idx, "db should start before api");

    assert!(session_exists("db", "group-a"));
    assert!(session_exists("db", "group-b"));
    assert!(session_exists("api", "group-a"));
}

#[test]
fn ambiguous_depends_on_errors() {
    // A bare depends_on with no same-group match and multiple tree-wide matches
    // must fail at load, listing the FQ candidates.
    let ctx = TestContext::new();
    ctx.write_file(
        "group-a/rig.yaml",
        "services:\n  db:\n    command: sh -c \"echo a-db; sleep 30\"\n    working_dir: /tmp\n",
    );
    ctx.write_file(
        "group-b/rig.yaml",
        "services:\n  db:\n    command: sh -c \"echo b-db; sleep 30\"\n    working_dir: /tmp\n",
    );
    ctx.write_file(
        "sub/rig.yaml",
        "services:\n  web:\n    command: sh -c \"echo web; sleep 30\"\n    working_dir: /tmp\n    depends_on: [db]\n",
    );

    let result = ctx.rig(&["ps"]);
    assert_eq!(result.code, 1, "stdout: {}", result.stdout);
    assert!(
        result.stderr.contains("ambiguous depends_on 'db'"),
        "stderr: {}",
        result.stderr
    );
    assert!(
        result.stderr.contains("group-a.db") && result.stderr.contains("group-b.db"),
        "error must list both FQ paths, stderr: {}",
        result.stderr
    );
}

#[test]
fn dotted_depends_on_disambiguates() {
    // A group-qualified depends_on picks the named service where a bare ref
    // would be ambiguous.
    let ctx = TestContext::new();
    ctx.write_file(
        "group-a/rig.yaml",
        "services:\n  db:\n    command: sh -c \"echo a-db; sleep 30\"\n    working_dir: /tmp\n",
    );
    ctx.write_file(
        "group-b/rig.yaml",
        "services:\n  db:\n    command: sh -c \"echo b-db; sleep 30\"\n    working_dir: /tmp\n",
    );
    ctx.write_file(
        "sub/rig.yaml",
        "services:\n  web:\n    command: sh -c \"echo web; sleep 30\"\n    working_dir: /tmp\n    depends_on: [group-b.db]\n",
    );

    let result = ctx.rig(&["start", "-d"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(session_exists("web", "sub"));
    assert!(session_exists("db", "group-a"));
    assert!(session_exists("db", "group-b"));
}

#[test]
fn ps_f_shows_full_metrics() {
    let ctx = TestContext::new();
    ctx.setup_test_config();
    ctx.rig(&["start", "-d", "echo-svc"]);
    delay_ms(500);

    let result = ctx.rig(&["ps", "-f"]);
    assert_eq!(result.code, 0, "stderr: {}", result.stderr);

    let stdout = &result.stdout;
    assert!(stdout.contains("MEM"), "stdout: {}", stdout);
    assert!(stdout.contains("CPU"), "stdout: {}", stdout);
    assert!(stdout.contains("PORTS"), "stdout: {}", stdout);
    assert!(stdout.contains("PID"), "stdout: {}", stdout);

    let clean = strip_ansi(stdout);
    let echo_line = clean
        .lines()
        .find(|l| l.contains("echo-svc") && l.contains("running"))
        .expect("echo-svc running line not found");

    let parts: Vec<&str> = echo_line.split_whitespace().collect();
    assert_eq!(parts[0], TEST_GROUP, "GROUP column");
    assert_eq!(parts[1], "echo-svc", "SERVICE column");
    assert_eq!(parts[2], "running", "STATUS column");

    // MEM should match format like "5M" or "0M"
    assert!(parts[3].ends_with('M'), "MEM format, got: {}", parts[3]);

    // CPU should match format like "0.1%" or "0%"
    assert!(parts[4].ends_with('%'), "CPU format, got: {}", parts[4]);

    // PID at the end should be a valid number
    let pid: u32 = parts
        .last()
        .unwrap()
        .parse()
        .expect("PID should be a number");
    assert!(pid > 0, "PID should be positive, got: {}", pid);
}
