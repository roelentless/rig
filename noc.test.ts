#!/usr/bin/env -S deno test -A

/**
 * Tests for noc - no-containers service manager
 *
 * Run with: deno test -A noc.test.ts
 */

import {
  assertEquals,
  assertStringIncludes,
} from "jsr:@std/assert";

const TEST_GROUP = "noc-test";
const TEST_CONFIG = `
group: ${TEST_GROUP}

services:
  echo-svc:
    command: sh -c "echo 'hello from echo-svc'; sleep 30"
    cwd: /tmp
    color: cyan

  counter:
    command: sh -c "for i in 1 2 3 4 5; do echo count-\\$i; sleep 1; done; sleep 30"
    cwd: /tmp
    color: yellow

  quick-exit:
    command: sh -c "echo 'quick exit'; exit 42"
    cwd: /tmp
    color: red
`;

// Helper to run noc commands
async function noc(args: string[]): Promise<{ code: number; stdout: string; stderr: string }> {
  const cmd = new Deno.Command("deno", {
    args: ["run", "-A", "noc.ts", ...args],
    cwd: import.meta.dirname,
  });
  const result = await cmd.output();
  return {
    code: result.code,
    stdout: new TextDecoder().decode(result.stdout),
    stderr: new TextDecoder().decode(result.stderr),
  };
}

// Helper to run tmux commands directly
async function tmux(args: string[]): Promise<{ code: number; stdout: string }> {
  const cmd = new Deno.Command("tmux", { args });
  const result = await cmd.output();
  return {
    code: result.code,
    stdout: new TextDecoder().decode(result.stdout),
  };
}

// Helper to check if a session exists
async function sessionExists(name: string): Promise<boolean> {
  const { code } = await tmux(["has-session", "-t", `${TEST_GROUP}-${name}`]);
  return code === 0;
}

// Helper to kill all test sessions
async function cleanupSessions(): Promise<void> {
  const { stdout } = await tmux(["list-sessions", "-F", "#{session_name}"]);
  const sessions = stdout.trim().split("\n").filter((s) => s.startsWith(`${TEST_GROUP}-`));
  for (const session of sessions) {
    await tmux(["kill-session", "-t", session]);
  }
}

// Setup: write test config
async function setupTestConfig(): Promise<void> {
  await Deno.writeTextFile(`${import.meta.dirname}/noc.yaml`, TEST_CONFIG);
}

// Teardown: remove test config and cleanup sessions
async function teardown(): Promise<void> {
  try {
    await Deno.remove(`${import.meta.dirname}/noc.yaml`);
  } catch { /* ignore */ }
  await cleanupSessions();
}

// ============================================================================
// TESTS
// ============================================================================

// Strip ANSI codes for assertions
function stripAnsi(str: string): string {
  return str.replace(/\x1b\[[0-9;]*m/g, "");
}

Deno.test({
  name: "noc help - shows usage",
  async fn() {
    const { stdout, code } = await noc(["help"]);
    assertEquals(code, 0);
    const clean = stripAnsi(stdout);
    assertStringIncludes(clean, "noc - no-containers");
    assertStringIncludes(clean, "COMMANDS:");
  },
});

Deno.test({
  name: "noc start -d - starts services in background",
  async fn() {
    await setupTestConfig();
    try {
      // Start in detached mode
      const { code, stdout } = await noc(["start", "-d", "echo-svc"]);
      assertEquals(code, 0);
      assertStringIncludes(stdout, "Started echo-svc");

      // Verify session exists
      assertEquals(await sessionExists("echo-svc"), true);
    } finally {
      await teardown();
    }
  },
});

Deno.test({
  name: "noc ps - shows service status",
  async fn() {
    await setupTestConfig();
    try {
      // Start a service
      await noc(["start", "-d", "echo-svc"]);

      // Check ps output
      const { code, stdout } = await noc(["ps"]);
      assertEquals(code, 0);
      assertStringIncludes(stdout, "echo-svc");
      assertStringIncludes(stdout, "running");
    } finally {
      await teardown();
    }
  },
});

Deno.test({
  name: "noc stop - stops services",
  async fn() {
    await setupTestConfig();
    try {
      // Start then stop
      await noc(["start", "-d", "echo-svc"]);
      assertEquals(await sessionExists("echo-svc"), true);

      const { code } = await noc(["stop", "echo-svc"]);
      assertEquals(code, 0);

      // Give tmux a moment to clean up
      await new Promise((r) => setTimeout(r, 200));

      // Session should be gone
      assertEquals(await sessionExists("echo-svc"), false);
    } finally {
      await teardown();
    }
  },
});

Deno.test({
  name: "noc restart - restarts services",
  async fn() {
    await setupTestConfig();
    try {
      // Start service
      await noc(["start", "-d", "echo-svc"]);
      const { stdout: ps1 } = await noc(["ps"]);

      // Small delay so uptime changes
      await new Promise((r) => setTimeout(r, 1100));

      // Restart
      const { code } = await noc(["restart", "echo-svc"]);
      assertEquals(code, 0);

      // Session should still exist
      assertEquals(await sessionExists("echo-svc"), true);
    } finally {
      await teardown();
    }
  },
});

Deno.test({
  name: "noc logs - captures service output",
  async fn() {
    await setupTestConfig();
    try {
      // Start service that outputs something
      await noc(["start", "-d", "echo-svc"]);

      // Give it time to output
      await new Promise((r) => setTimeout(r, 500));

      // Get logs
      const { code, stdout } = await noc(["logs", "echo-svc"]);
      assertEquals(code, 0);
      assertStringIncludes(stdout, "hello from echo-svc");
    } finally {
      await teardown();
    }
  },
});

Deno.test({
  name: "noc start - does not duplicate running services",
  async fn() {
    await setupTestConfig();
    try {
      // Start twice
      await noc(["start", "-d", "echo-svc"]);
      const { stdout } = await noc(["start", "-d", "echo-svc"]);

      // Should indicate already running
      assertStringIncludes(stdout, "already running");
    } finally {
      await teardown();
    }
  },
});

Deno.test({
  name: "noc start - multiple services",
  async fn() {
    await setupTestConfig();
    try {
      const { code, stdout } = await noc(["start", "-d", "echo-svc", "counter"]);
      assertEquals(code, 0);
      assertStringIncludes(stdout, "Started echo-svc");
      assertStringIncludes(stdout, "Started counter");

      assertEquals(await sessionExists("echo-svc"), true);
      assertEquals(await sessionExists("counter"), true);
    } finally {
      await teardown();
    }
  },
});

Deno.test({
  name: "noc - unknown service errors",
  async fn() {
    await setupTestConfig();
    try {
      const { code, stderr } = await noc(["start", "-d", "nonexistent"]);
      assertEquals(code, 1);
      assertStringIncludes(stderr, "Unknown service");
    } finally {
      await teardown();
    }
  },
});

// Final cleanup in case tests fail
Deno.test({
  name: "cleanup",
  async fn() {
    await teardown();
  },
});
