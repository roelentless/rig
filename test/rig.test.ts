#!/usr/bin/env -S deno test -A

/**
 * rig tests - runs on both macOS and Linux
 *
 * Run with: deno test -A test/rig.test.ts
 */

import {
  assertEquals,
  assertStringIncludes,
  delay,
  rig,
  sessionExists,
  setupTestConfig,
  stripAnsi,
  teardown,
} from "./_helpers.ts";

// Platform prefix for test names
const PLATFORM = Deno.build.os;

Deno.test({
  name: `[${PLATFORM}] rig help - shows usage`,
  async fn() {
    const { stdout, code } = await rig(["help"]);
    assertEquals(code, 0);
    const clean = stripAnsi(stdout);
    assertStringIncludes(clean, "rig - lightweight, tmux-based process manager");
    assertStringIncludes(clean, "COMMANDS:");
  },
});

Deno.test({
  name: `[${PLATFORM}] rig start -d - starts processes in background`,
  async fn() {
    await setupTestConfig();
    try {
      const { code, stdout } = await rig(["start", "-d", "echo-svc"]);
      assertEquals(code, 0);
      assertStringIncludes(stdout, "Started echo-svc");
      assertEquals(await sessionExists("echo-svc"), true);
    } finally {
      await teardown();
    }
  },
});

Deno.test({
  name: `[${PLATFORM}] rig up -d - alias for start`,
  async fn() {
    await setupTestConfig();
    try {
      const { code, stdout } = await rig(["up", "-d", "echo-svc"]);
      assertEquals(code, 0);
      assertStringIncludes(stdout, "Started echo-svc");
      assertEquals(await sessionExists("echo-svc"), true);
    } finally {
      await teardown();
    }
  },
});

Deno.test({
  name: `[${PLATFORM}] rig ps - shows process status`,
  async fn() {
    await setupTestConfig();
    try {
      await rig(["start", "-d", "echo-svc"]);
      const { code, stdout } = await rig(["ps"]);
      assertEquals(code, 0);
      assertStringIncludes(stdout, "echo-svc");
      assertStringIncludes(stdout, "running");
    } finally {
      await teardown();
    }
  },
});

Deno.test({
  name: `[${PLATFORM}] rig stop - stops processes`,
  async fn() {
    await setupTestConfig();
    try {
      await rig(["start", "-d", "echo-svc"]);
      assertEquals(await sessionExists("echo-svc"), true);

      const { code } = await rig(["stop", "echo-svc"]);
      assertEquals(code, 0);

      await delay(200);
      assertEquals(await sessionExists("echo-svc"), false);
    } finally {
      await teardown();
    }
  },
});

Deno.test({
  name: `[${PLATFORM}] rig down - alias for stop`,
  async fn() {
    await setupTestConfig();
    try {
      await rig(["start", "-d", "echo-svc"]);
      assertEquals(await sessionExists("echo-svc"), true);

      const { code } = await rig(["down", "echo-svc"]);
      assertEquals(code, 0);

      await delay(200);
      assertEquals(await sessionExists("echo-svc"), false);
    } finally {
      await teardown();
    }
  },
});

Deno.test({
  name: `[${PLATFORM}] rig restart - restarts processes`,
  async fn() {
    await setupTestConfig();
    try {
      await rig(["start", "-d", "echo-svc"]);
      await delay(1100);

      const { code } = await rig(["restart", "echo-svc"]);
      assertEquals(code, 0);
      assertEquals(await sessionExists("echo-svc"), true);
    } finally {
      await teardown();
    }
  },
});

Deno.test({
  name: `[${PLATFORM}] rig logs - captures process output`,
  async fn() {
    await setupTestConfig();
    try {
      await rig(["start", "-d", "echo-svc"]);
      await delay(500);

      const { code, stdout } = await rig(["logs", "echo-svc"]);
      assertEquals(code, 0);
      assertStringIncludes(stdout, "hello from echo-svc");
    } finally {
      await teardown();
    }
  },
});

Deno.test({
  name: `[${PLATFORM}] rig start - does not duplicate running processes`,
  async fn() {
    await setupTestConfig();
    try {
      await rig(["start", "-d", "echo-svc"]);
      const { stdout } = await rig(["start", "-d", "echo-svc"]);
      assertStringIncludes(stdout, "already running");
    } finally {
      await teardown();
    }
  },
});

Deno.test({
  name: `[${PLATFORM}] rig start - multiple processes`,
  async fn() {
    await setupTestConfig();
    try {
      const { code, stdout } = await rig(["start", "-d", "echo-svc", "counter"]);
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
  name: `[${PLATFORM}] rig - unknown process errors`,
  async fn() {
    await setupTestConfig();
    try {
      const { code, stderr } = await rig(["start", "-d", "nonexistent"]);
      assertEquals(code, 1);
      assertStringIncludes(stderr, "Unknown");
    } finally {
      await teardown();
    }
  },
});

Deno.test({
  name: `[${PLATFORM}] rig ps -f - shows full metrics with valid values`,
  async fn() {
    await setupTestConfig();
    try {
      await rig(["start", "-d", "echo-svc"]);
      await delay(500);

      const { code, stdout } = await rig(["ps", "-f"]);
      assertEquals(code, 0);

      // Headers present
      assertStringIncludes(stdout, "MEM");
      assertStringIncludes(stdout, "CPU");
      assertStringIncludes(stdout, "PORTS");
      assertStringIncludes(stdout, "PID");

      // Find the echo-svc line and validate values
      const lines = stripAnsi(stdout).split("\n");
      const echoLine = lines.find((l) => l.includes("echo-svc") && l.includes("running"));
      if (!echoLine) {
        throw new Error("echo-svc running line not found in ps -f output");
      }

      // Parse the line: SERVICE STATUS MEM CPU PORTS UPTIME PID
      const parts = echoLine.trim().split(/\s+/);
      assertEquals(parts[0], "echo-svc");
      assertEquals(parts[1], "running");

      // MEM should match format like "5M" or "0M"
      const memMatch = parts[2].match(/^\d+M$/);
      assertEquals(memMatch !== null, true, `MEM should match \\dM format, got: ${parts[2]}`);

      // CPU should match format like "0.1%" or "0%"
      const cpuMatch = parts[3].match(/^\d+(\.\d+)?%$/);
      assertEquals(cpuMatch !== null, true, `CPU should match percentage format, got: ${parts[3]}`);

      // PID is at the end - should be a valid number
      const pid = parseInt(parts[parts.length - 1], 10);
      assertEquals(isNaN(pid), false, `PID should be a number, got: ${parts[parts.length - 1]}`);
      assertEquals(pid > 0, true, `PID should be positive, got: ${pid}`);
    } finally {
      await teardown();
    }
  },
});

// Final cleanup
Deno.test({
  name: `[${PLATFORM}] cleanup`,
  async fn() {
    await teardown();
  },
});
