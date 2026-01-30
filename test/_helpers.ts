/**
 * Shared test helpers for rig tests
 */

import { assertEquals, assertStringIncludes } from "jsr:@std/assert";

export { assertEquals, assertStringIncludes };

export const TEST_GROUP = "rig-test";

export const TEST_CONFIG = `
group: ${TEST_GROUP}

services:
  echo-svc:
    command: sh -c "echo 'hello from echo-svc'; sleep 30"
    working_dir: /tmp
    color: cyan

  counter:
    command: sh -c "for i in 1 2 3 4 5; do echo count-\\$i; sleep 1; done; sleep 30"
    working_dir: /tmp
    color: yellow

  quick-exit:
    command: sh -c "echo 'quick exit'; exit 42"
    working_dir: /tmp
    color: red
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

// Helper to check if a session exists
export async function sessionExists(name: string): Promise<boolean> {
  const { code } = await tmux(["has-session", "-t", `${TEST_GROUP}-${name}`]);
  return code === 0;
}

// Helper to kill all test sessions
export async function cleanupSessions(): Promise<void> {
  const { stdout } = await tmux(["list-sessions", "-F", "#{session_name}"]);
  const sessions = stdout.trim().split("\n").filter((s) => s.startsWith(`${TEST_GROUP}-`));
  for (const session of sessions) {
    await tmux(["kill-session", "-t", session]);
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
