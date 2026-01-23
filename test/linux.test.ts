#!/usr/bin/env -S deno test -A

/**
 * rig tests for Linux
 *
 * These tests run inside Docker. Use: deno task test:linux
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

// Skip if not on linux
const IS_LINUX = Deno.build.os === "linux";

Deno.test({
  name: "[linux] rig help - shows usage",
  ignore: !IS_LINUX,
  async fn() {
    const { stdout, code } = await rig(["help"]);
    assertEquals(code, 0);
    const clean = stripAnsi(stdout);
    assertStringIncludes(clean, "rig - lightweight, tmux-based process manager");
    assertStringIncludes(clean, "COMMANDS:");
  },
});

Deno.test({
  name: "[linux] rig start -d - starts processes in background",
  ignore: !IS_LINUX,
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
  name: "[linux] rig up -d - alias for start",
  ignore: !IS_LINUX,
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
  name: "[linux] rig ps - shows process status",
  ignore: !IS_LINUX,
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
  name: "[linux] rig stop - stops processes",
  ignore: !IS_LINUX,
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
  name: "[linux] rig down - alias for stop",
  ignore: !IS_LINUX,
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
  name: "[linux] rig restart - restarts processes",
  ignore: !IS_LINUX,
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
  name: "[linux] rig logs - captures process output",
  ignore: !IS_LINUX,
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
  name: "[linux] rig start - does not duplicate running processes",
  ignore: !IS_LINUX,
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
  name: "[linux] rig start - multiple processes",
  ignore: !IS_LINUX,
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
  name: "[linux] rig - unknown process errors",
  ignore: !IS_LINUX,
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
  name: "[linux] rig ps -f - shows full metrics with valid values",
  ignore: !IS_LINUX,
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

      // Parse the line: SERVICE STATUS PID MEM CPU PORTS UPTIME
      const parts = echoLine.trim().split(/\s+/);
      assertEquals(parts[0], "echo-svc");
      assertEquals(parts[1], "running");

      // PID should be a valid number
      const pid = parseInt(parts[2], 10);
      assertEquals(isNaN(pid), false, `PID should be a number, got: ${parts[2]}`);
      assertEquals(pid > 0, true, `PID should be positive, got: ${pid}`);

      // MEM should match format like "5M" or "0M"
      const memMatch = parts[3].match(/^\d+M$/);
      assertEquals(memMatch !== null, true, `MEM should match \\dM format, got: ${parts[3]}`);

      // CPU should match format like "0.1%" or "0%"
      const cpuMatch = parts[4].match(/^\d+(\.\d+)?%$/);
      assertEquals(cpuMatch !== null, true, `CPU should match percentage format, got: ${parts[4]}`);
    } finally {
      await teardown();
    }
  },
});

// Final cleanup
Deno.test({
  name: "[linux] cleanup",
  ignore: !IS_LINUX,
  async fn() {
    await teardown();
  },
});
