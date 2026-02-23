/**
 * Shared test helpers for rig tests
 */

import { assertEquals, assertStringIncludes } from "jsr:@std/assert";

export { assertEquals, assertStringIncludes };

export const TEST_GROUP = "rig-test";

export const TEST_CONFIG = `
groups:
  ${TEST_GROUP}:
    services:
      echo-svc:
        command: sh -c "echo 'hello from echo-svc'; sleep 30"
        working_dir: /tmp
        color: cyan
        tasks:
          greet:
            command: echo "hello from greet"
          show-env:
            command: "sh -c 'echo PORT=\$PORT'"
            environment:
              PORT: "3001"

      counter:
        command: sh -c "for i in 1 2 3 4 5; do echo count-\$i; sleep 1; done; sleep 30"
        working_dir: /tmp
        color: yellow
        environment:
          COUNT_VAR: "from-service"
        tasks:
          check-env:
            command: "sh -c 'echo COUNT_VAR=\$COUNT_VAR EXTRA=\$EXTRA'"
            environment:
              EXTRA: "from-task"

      quick-exit:
        command: sh -c "echo 'quick exit'; exit 42"
        working_dir: /tmp
        color: red

    tasks:
      group-cmd:
        command: echo "group command output"
        working_dir: /tmp
        description: A test group task
      exit-with-code:
        command: sh -c "exit 7"
        working_dir: /tmp
      echo-args:
        command: "sh -c 'echo args: \$*' --"
        working_dir: /tmp
      task-a:
        command: echo "task-a-output"
        working_dir: /tmp
      task-b:
        command: echo "task-b-output"
        working_dir: /tmp
      task-c:
        command: echo "task-c-output"
        working_dir: /tmp
      fail-task:
        command: sh -c "echo 'fail-task-ran'; exit 3"
        working_dir: /tmp
`;

// Get the repo root (parent of test folder)
export function getRepoRoot(): string {
  const testDir = import.meta.dirname!;
  return testDir.replace(/\/test$/, "");
}

// Get the test tmp directory for test artifacts
export function getTestTmpDir(): string {
  return `${import.meta.dirname!}/tmp`;
}

// Ensure test tmp directory exists
export async function ensureTestTmpDir(): Promise<void> {
  const tmpDir = getTestTmpDir();
  await Deno.mkdir(tmpDir, { recursive: true });
}

// Helper to run rig commands (runs from test/tmp directory)
export async function rig(args: string[]): Promise<{ code: number; stdout: string; stderr: string }> {
  const repoRoot = getRepoRoot();
  const testTmpDir = getTestTmpDir();
  await ensureTestTmpDir();
  const cmd = new Deno.Command("deno", {
    args: ["run", "-A", `${repoRoot}/rig.ts`, ...args],
    cwd: testTmpDir,
  });
  const result = await cmd.output();
  return {
    code: result.code,
    stdout: new TextDecoder().decode(result.stdout),
    stderr: new TextDecoder().decode(result.stderr),
  };
}

// Helper to run tmux commands directly
export async function tmux(args: string[]): Promise<{ code: number; stdout: string }> {
  const cmd = new Deno.Command("tmux", { args });
  const result = await cmd.output();
  return {
    code: result.code,
    stdout: new TextDecoder().decode(result.stdout),
  };
}

// Helper to check if a session exists (default group: TEST_GROUP)
export async function sessionExists(name: string, group: string = TEST_GROUP): Promise<boolean> {
  const { code } = await tmux(["has-session", "-t", `${group}-${name}`]);
  return code === 0;
}

// Helper to check if any tmux session with given name exists (no group prefix)
export async function sessionExistsByFullName(sessionName: string): Promise<boolean> {
  const { code } = await tmux(["has-session", "-t", sessionName]);
  return code === 0;
}

// Test group prefixes used in multi-file tests
const ALL_TEST_GROUPS = [TEST_GROUP, "database", "backend", "frontend", "infra", "shared", "app", "root", "mygroup", "group-a", "group-b", "a-group", "sub", "new"];

// Helper to kill all test sessions (from all test-related groups)
export async function cleanupSessions(): Promise<void> {
  const { stdout } = await tmux(["list-sessions", "-F", "#{session_name}"]);
  const sessions = stdout.trim().split("\n").filter(Boolean);
  for (const session of sessions) {
    // Kill any session that starts with a known test group prefix
    for (const prefix of ALL_TEST_GROUPS) {
      if (session.startsWith(`${prefix}-`)) {
        await tmux(["kill-session", "-t", session]);
        break;
      }
    }
  }
}

// Setup: write test config to test/tmp
export async function setupTestConfig(): Promise<void> {
  const testTmpDir = getTestTmpDir();
  await ensureTestTmpDir();
  await Deno.writeTextFile(`${testTmpDir}/rig.yaml`, TEST_CONFIG);
}

// Teardown: remove test config and cleanup sessions
export async function teardown(): Promise<void> {
  const testTmpDir = getTestTmpDir();
  try {
    await Deno.remove(`${testTmpDir}/rig.yaml`);
  } catch { /* ignore */ }
  await cleanupSessions();
}

// Strip ANSI codes for assertions
export function stripAnsi(str: string): string {
  return str.replace(/\x1b\[[0-9;]*m/g, "");
}

// Small delay helper
export function delay(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

// Check if watchexec is installed
export async function watchexecInstalled(): Promise<boolean> {
  try {
    const cmd = new Deno.Command("which", { args: ["watchexec"] });
    const { code } = await cmd.output();
    return code === 0;
  } catch {
    return false;
  }
}

// Test config with watch
export const TEST_CONFIG_WITH_WATCH = `
groups:
  ${TEST_GROUP}:
    services:
      watched-svc:
        command: sh -c "echo 'started'; sleep 30"
        working_dir: /tmp
        watch:
          paths: ['.']
          extensions: [txt, md]
          patterns: ['**/*.log']
          ignore: ['**/cache/**']
          debounce: 100ms
`;
