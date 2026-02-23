/**
 * CLI command implementations: what each command does.
 */

// deno-lint-ignore no-import-prefix
import { parse as parseYaml, stringify as stringifyYaml } from "jsr:@std/yaml@^1.0.11";

import { c, COLORS, KEY_CTRL_C, KEY_Q_LOWER, KEY_Q_UPPER, log, logError, logSystem, logVerbose, print, stripControlCodes } from "./output.ts";
import type { Config, ResolvedService, ResolvedTask } from "./config.ts";
import { CONFIG_NAMES, getAllTasks, parseConfigFile } from "./config.ts";
import { getProcessMetrics, getProcessTree, getServiceColor, streamLogs } from "./process.ts";
import type { SessionManager, SessionStatus, ProcessMetrics } from "./process.ts";

// ============================================================================
// DEPENDENCY ORDERING
// ============================================================================

/**
 * Compute startup order based on depends_on relationships.
 * Returns services grouped by "level" - services in the same level can start together,
 * but must wait for previous levels to complete.
 */
export function computeStartupOrder(
  targets: ResolvedService[]
): ResolvedService[][] {
  const requested = new Set(targets.map((t) => t.name));
  const byName = new Map(targets.map((t) => [t.name, t]));

  // Build dependency graph (only for requested services)
  const deps: Record<string, Set<string>> = {};
  for (const target of targets) {
    deps[target.name] = new Set();
    const svcDeps = target.def.depends_on ?? [];
    for (const dep of svcDeps) {
      // Only include dependencies that are in the requested set
      if (requested.has(dep)) {
        deps[target.name].add(dep);
      }
    }
  }

  // Kahn's algorithm for topological sort with levels
  const levels: ResolvedService[][] = [];
  const remaining = new Set(targets.map((t) => t.name));

  while (remaining.size > 0) {
    // Find all services with no remaining dependencies
    const level: ResolvedService[] = [];
    for (const name of remaining) {
      const unresolvedDeps = [...deps[name]].filter((d) => remaining.has(d));
      if (unresolvedDeps.length === 0) {
        level.push(byName.get(name)!);
      }
    }

    if (level.length === 0) {
      // Circular dependency - just start remaining in any order
      logSystem("Warning: circular dependency detected, starting remaining services");
      levels.push([...remaining].map((n) => byName.get(n)!));
      break;
    }

    levels.push(level);
    for (const svc of level) {
      remaining.delete(svc.name);
    }
  }

  return levels;
}

/**
 * Get the maximum grace_ms for a set of services
 */
function getMaxGraceMs(targets: ResolvedService[]): number {
  let maxGrace = 0;
  for (const target of targets) {
    const grace = target.def.healthcheck?.grace_ms ?? 0;
    if (grace > maxGrace) {
      maxGrace = grace;
    }
  }
  return maxGrace;
}

// ============================================================================
// SERVICE LIFECYCLE
// ============================================================================

export async function cmdStart(
  managers: Map<string, SessionManager>,
  targets: ResolvedService[],
  allServices: ResolvedService[],
  detached: boolean
): Promise<void> {
  logSystem(`Starting ${targets.length} process(es)...`);

  // Compute startup order based on dependencies
  const levels = computeStartupOrder(targets);

  // Start services level by level
  for (let i = 0; i < levels.length; i++) {
    const level = levels[i];

    // Start all services in this level
    for (const target of level) {
      const mgr = managers.get(target.group)!;
      await mgr.start(target.name, target.def);
    }

    // If there are more levels, wait for grace period before starting next level
    if (i < levels.length - 1) {
      const graceMs = getMaxGraceMs(level);
      if (graceMs > 0) {
        logVerbose(`Waiting ${graceMs}ms grace period for: ${level.map((t) => t.name).join(", ")}`);
        await new Promise((r) => setTimeout(r, graceMs));
      }
    }
  }

  if (detached) {
    logSystem("Processes started in background");
    return;
  }

  // Monitor mode
  await monitor(managers, targets, allServices);
}

async function monitor(
  managers: Map<string, SessionManager>,
  targets: ResolvedService[],
  allServices: ResolvedService[]
): Promise<void> {
  logSystem("Monitoring processes... (Ctrl+C to stop all)");

  let running = true;
  let stopping = false;

  // Start streaming logs
  const { cleanup } = streamLogs(managers, targets, allServices);

  // Handle Ctrl+C - ensure only one signal handler executes cleanup
  const handleShutdown = async (signal: string) => {
    if (stopping) return;
    stopping = true;
    if (!running) return;
    running = false;
    cleanup();
    logSystem(`Received ${signal}, stopping all processes...`);
    await cmdStop(managers, targets);
    Deno.exit(0);
  };

  Deno.addSignalListener("SIGINT", () => handleShutdown("SIGINT"));
  Deno.addSignalListener("SIGTERM", () => handleShutdown("SIGTERM"));

  // Monitor for dead services
  const deadServices = new Set<string>();
  while (running) {
    for (const target of targets) {
      if (!running) break;
      if (deadServices.has(target.name)) continue;

      const mgr = managers.get(target.group)!;
      const status = await mgr.status(target.name);
      if (!status.running && status.exitCode !== undefined) {
        log(`Exited with code ${status.exitCode}`, target.name, "red");
        deadServices.add(target.name);
      }
    }

    await new Promise((r) => setTimeout(r, 500));
  }
}

export async function cmdStop(
  managers: Map<string, SessionManager>,
  targets: ResolvedService[]
): Promise<void> {
  let stoppedAny = false;

  // Always loop through all services - idempotent
  for (const target of targets) {
    const mgr = managers.get(target.group)!;
    if (await mgr.exists(target.name)) {
      await mgr.stop(target.name);
      stoppedAny = true;
    }
  }

  if (stoppedAny) {
    logSystem("All processes stopped");
  } else {
    logSystem("No processes were running");
  }
}

export async function cmdKill(
  managers: Map<string, SessionManager>,
  targets: ResolvedService[]
): Promise<void> {
  let killedAny = false;

  for (const target of targets) {
    const mgr = managers.get(target.group)!;
    if (await mgr.exists(target.name)) {
      await mgr.kill(target.name);
      killedAny = true;
    }
  }

  if (killedAny) {
    logSystem("All processes killed");
  } else {
    logSystem("No processes were running");
  }
}

export async function cmdRestart(
  managers: Map<string, SessionManager>,
  targets: ResolvedService[]
): Promise<void> {
  logSystem(`Restarting ${targets.length} process(es)...`);

  for (const target of targets) {
    const mgr = managers.get(target.group)!;
    await mgr.stop(target.name);
    await mgr.start(target.name, target.def);
  }
}

// ============================================================================
// OBSERVABILITY
// ============================================================================

export async function cmdPs(
  managers: Map<string, SessionManager>,
  targets: ResolvedService[],
  showAll: boolean
): Promise<void> {
  // Gather sessions from all groups
  const sessionsByService = new Map<string, SessionStatus>();
  for (const [_groupName, mgr] of managers) {
    const sessions = await mgr.listAll();
    for (const session of sessions) {
      sessionsByService.set(session.name, session);
    }
  }

  print("");

  if (showAll) {
    print(
      `${c("bold")}GROUP          SERVICE        STATUS       MEM    CPU  PORTS            UPTIME      PID${c("reset")}`
    );
    print("─".repeat(95));
  } else {
    print(`${c("bold")}GROUP          SERVICE        STATUS       UPTIME${c("reset")}`);
    print("─".repeat(55));
  }

  for (const target of targets) {
    const session = sessionsByService.get(target.name);
    let statusText: string;
    let statusColor: string;
    let uptime = "-";

    if (!session) {
      statusText = "stopped";
      statusColor = "dim";
    } else if (session.running) {
      statusText = "running";
      statusColor = "green";

      if (session.created) {
        const elapsed = Math.floor(Date.now() / 1000 - session.created);
        if (elapsed >= 3600) {
          const hours = Math.floor(elapsed / 3600);
          const mins = Math.floor((elapsed % 3600) / 60);
          uptime = `${hours}h ${mins}m`;
        } else if (elapsed >= 60) {
          const mins = Math.floor(elapsed / 60);
          const secs = elapsed % 60;
          uptime = `${mins}m ${secs}s`;
        } else {
          uptime = `${elapsed}s`;
        }
      }
    } else {
      statusText = `exit(${session.exitCode})`;
      statusColor = "red";
    }

    if (showAll) {
      let pid = "-";
      let mem = "-";
      let cpu = "-";
      let ports = "-";

      if (session?.running && session.pid) {
        pid = String(session.pid);
        const metrics = await getProcessMetrics(session.pid);
        mem = `${metrics.memoryMB}M`;
        cpu = `${metrics.cpuPercent}%`;
        ports = metrics.ports.length > 0 ? metrics.ports.join(",") : "-";
      }

      const groupCol = target.group.padEnd(14);
      const svcCol = target.name.padEnd(14);
      const statusCol = `${c(statusColor)}${statusText.padEnd(12)}${c("reset")}`;
      const memCol = mem.padStart(5);
      const cpuCol = cpu.padStart(5);
      const portsCol = ports.padEnd(16);
      const uptimeCol = uptime.padEnd(10);
      print(`${groupCol} ${svcCol} ${statusCol} ${memCol} ${cpuCol}  ${portsCol} ${uptimeCol} ${pid}`);
    } else {
      const groupCol = target.group.padEnd(14);
      const statusCol = `${c(statusColor)}${statusText.padEnd(12)}${c("reset")}`;
      print(`${groupCol} ${target.name.padEnd(14)} ${statusCol} ${uptime}`);
    }
  }
  print("");
}

export async function cmdTop(
  managers: Map<string, SessionManager>,
  targets: ResolvedService[]
): Promise<void> {
  // Track metrics and last update time per service
  const metrics: Record<string, ProcessMetrics & { lastUpdate: number }> = {};
  const sessions: Record<string, SessionStatus> = {};

  // Initialize
  for (const target of targets) {
    metrics[target.name] = { memoryMB: 0, cpuPercent: 0, ports: [], processCount: 0, lastUpdate: 0 };
  }

  // Refresh scheduling: services with higher CPU get refreshed more often
  function getRefreshInterval(svc: string): number {
    const cpu = metrics[svc]?.cpuPercent ?? 0;
    if (cpu > 10) return 1000;   // High CPU: every 1s
    if (cpu > 1) return 2000;    // Medium CPU: every 2s
    return 5000;                  // Low/idle: every 5s
  }

  // ANSI helpers
  const CLEAR = "\x1b[2J\x1b[H";
  const HIDE_CURSOR = "\x1b[?25l";
  const SHOW_CURSOR = "\x1b[?25h";

  let running = true;
  let cleanedUp = false;

  function cleanup() {
    if (cleanedUp) return;
    cleanedUp = true;
    running = false;
    try {
      Deno.stdin.setRaw(false);
    } catch (err) {
      logVerbose(`Failed to restore stdin: ${err instanceof Error ? err.message : String(err)}`);
    }
    // Clear screen and restore cursor on exit (like regular top)
    Deno.stdout.writeSync(new TextEncoder().encode(CLEAR + SHOW_CURSOR));
  }

  // Ensure cursor is shown on exit
  Deno.addSignalListener("SIGINT", () => {
    cleanup();
    Deno.exit(0);
  });

  try {
    // Set up raw mode to capture keypresses
    Deno.stdin.setRaw(true);

    // Non-blocking key reader
    const keyReader = async () => {
      const buf = new Uint8Array(1);
      while (running) {
        try {
          const n = await Deno.stdin.read(buf);
          if (n === null) break;
          // 'q', 'Q', or Ctrl+C
          if (buf[0] === KEY_Q_LOWER || buf[0] === KEY_Q_UPPER || buf[0] === KEY_CTRL_C) {
            cleanup();
            Deno.exit(0);
          }
        } catch (err) {
          logVerbose(`Key reader error: ${err instanceof Error ? err.message : String(err)}`);
          break;
        }
      }
    };
    keyReader().catch((err) => {
      logVerbose(`Key reader failed: ${err instanceof Error ? err.message : String(err)}`);
    });

    Deno.stdout.writeSync(new TextEncoder().encode(HIDE_CURSOR));

    let tick = 0;

    while (running) {
      const now = Date.now();

      // Update session status for all services (cheap operation)
      for (const [groupName, mgr] of managers) {
        const sessionList = await mgr.listAll();
        for (const target of targets) {
          if (target.group === groupName) {
            const session = sessionList.find((s) => s.name === target.name);
            sessions[target.name] = session ?? { name: target.name, running: false };
          }
        }
      }

      // Smart refresh: update 2-3 services per tick based on their refresh interval
      let updated = 0;
      for (const target of targets) {
        if (!sessions[target.name]?.running || !sessions[target.name]?.pid) continue;

        const interval = getRefreshInterval(target.name);
        const timeSinceUpdate = now - metrics[target.name].lastUpdate;

        if (timeSinceUpdate >= interval && updated < 3) {
          const m = await getProcessMetrics(sessions[target.name].pid!);
          metrics[target.name] = { ...m, lastUpdate: now };
          updated++;
        }
      }

      // Render
      let output = CLEAR;
      output += `${c("bold")}rig top${c("reset")} - press q or Ctrl+C to exit\n\n`;
      output += `${c("bold")}GROUP          SERVICE        STATUS       MEM    CPU  PORTS            STARTED${c("reset")}\n`;
      output += "─".repeat(87) + "\n";

      for (const target of targets) {
        const session = sessions[target.name];
        const m = metrics[target.name];

        let status: string;
        let mem = "-";
        let cpu = "-";
        let ports = "-";
        let started = "-";

        if (!session || !session.running) {
          if (session?.exitCode !== undefined) {
            status = `${c("red")}exit(${session.exitCode})${c("reset")}`;
          } else {
            status = `${c("dim")}stopped${c("reset")}`;
          }
        } else {
          status = `${c("green")}running${c("reset")}`;
          mem = `${m.memoryMB}M`;

          // Color CPU based on usage
          if (m.cpuPercent > 50) {
            cpu = `${c("red")}${m.cpuPercent}%${c("reset")}`;
          } else if (m.cpuPercent > 10) {
            cpu = `${c("yellow")}${m.cpuPercent}%${c("reset")}`;
          } else {
            cpu = `${m.cpuPercent}%`;
          }

          ports = m.ports.length > 0 ? m.ports.slice(0, 3).join(",") : "-";
          if (m.ports.length > 3) ports += "...";

          if (session.created) {
            const date = new Date(session.created * 1000);
            started = date.toLocaleTimeString("en-US", {
              hour12: false,
              hour: "2-digit",
              minute: "2-digit",
              second: "2-digit",
            });
          }
        }

        const groupCol = target.group.padEnd(14);
        const svcCol = target.name.padEnd(14);
        const statusCol = status.padEnd(20);
        const memCol = mem.padStart(5);
        const cpuCol = cpu.padEnd(10);
        const portsCol = ports.padEnd(16);

        output += `${groupCol} ${svcCol} ${statusCol} ${memCol} ${cpuCol} ${portsCol} ${started}\n`;
      }

      Deno.stdout.writeSync(new TextEncoder().encode(output));

      tick++;
      await new Promise((r) => setTimeout(r, 500));
    }
  } finally {
    cleanup();
  }
}

export async function cmdLogs(
  managers: Map<string, SessionManager>,
  targets: ResolvedService[],
  allServices: ResolvedService[],
  follow: boolean,
  previous: boolean
): Promise<void> {
  // Check if log files exist
  let anyLogs = false;
  for (const target of targets) {
    const mgr = managers.get(target.group)!;
    const logFile = mgr.logFile(target.name, previous);
    try {
      const stat = await Deno.stat(logFile);
      if (stat.size > 0) {
        anyLogs = true;
        break;
      }
    } catch {
      // File doesn't exist
    }
  }

  if (!anyLogs && previous) {
    logError("No previous logs found");
    Deno.exit(1);
  }

  if (!anyLogs && !follow) {
    logError(targets.length === 1 ? `No logs for '${targets[0].name}'` : "No logs found");
    Deno.exit(1);
  }

  if (follow && previous) {
    logError("Cannot follow previous logs");
    Deno.exit(1);
  }

  if (follow) {
    // Follow mode - stream logs, Ctrl+C just exits (doesn't stop services)
    let running = true;
    const { cleanup } = streamLogs(managers, targets, allServices);

    Deno.addSignalListener("SIGINT", () => {
      running = false;
      cleanup();
    });

    // Wait until interrupted
    while (running) {
      await new Promise((r) => setTimeout(r, 500));
    }
  } else {
    // Dump mode - show all logs and exit
    for (const target of targets) {
      const mgr = managers.get(target.group)!;
      const logFile = mgr.logFile(target.name, previous);
      const color = getServiceColor(allServices, target.name);
      try {
        const content = await Deno.readTextFile(logFile);
        for (const line of content.split("\n")) {
          const cleanLine = stripControlCodes(line);
          if (cleanLine.trim()) {
            log(cleanLine, target.name, color);
          }
        }
      } catch {
        // No logs for this service
      }
    }
  }
}

// ============================================================================
// TASKS
// ============================================================================

/**
 * Shell escape a string for safe inclusion in a shell command.
 */
function shellEscape(arg: string): string {
  // If the arg contains only safe characters, return as-is
  if (/^[a-zA-Z0-9_\-./=@:]+$/.test(arg)) {
    return arg;
  }
  // Otherwise, wrap in single quotes and escape any single quotes
  return "'" + arg.replace(/'/g, "'\\''") + "'";
}

/**
 * Run a one-off task (not via tmux).
 * Handles signal forwarding to ensure clean termination.
 */
export async function runTask(
  resolved: ResolvedTask,
  args: string[]
): Promise<number> {
  // Build the full command with args
  const fullCommand = args.length > 0
    ? `${resolved.command} ${args.map(shellEscape).join(" ")}`
    : resolved.command;

  logVerbose(`task=${resolved.path}`);
  logVerbose(`command=${fullCommand}`);
  logVerbose(`working_dir=${resolved.working_dir}`);
  if (resolved.environment) {
    logVerbose(`environment=${Object.keys(resolved.environment).join(",")}`);
  }

  const proc = new Deno.Command("sh", {
    args: ["-c", fullCommand],
    cwd: resolved.working_dir,
    env: { ...Deno.env.toObject(), ...resolved.environment },
    stdin: "inherit",
    stdout: "inherit",
    stderr: "inherit",
  });

  const child = proc.spawn();
  let signalReceived: Deno.Signal | null = null;
  let cleaningUp = false;

  // Forward signals to child process
  const handleSignal = async (signal: Deno.Signal) => {
    if (cleaningUp) return;
    cleaningUp = true;
    signalReceived = signal;

    logVerbose(`Received ${signal}, forwarding to child...`);

    // First try graceful termination
    try {
      child.kill("SIGTERM");
    } catch {
      // Child already dead
    }

    // Set up a timeout for forceful kill
    const forceKillTimeout = setTimeout(async () => {
      logVerbose("Child didn't exit, using SIGKILL...");
      try {
        // Kill the entire process tree
        const pids = await getProcessTree(child.pid);
        for (const pid of pids) {
          try {
            Deno.kill(pid, "SIGKILL");
          } catch {
            // Already dead
          }
        }
      } catch {
        // Process tree lookup failed
      }
    }, 5000);

    // Wait for child and clean up
    try {
      await child.status;
    } catch {
      // Status already retrieved or child dead
    }
    clearTimeout(forceKillTimeout);
  };

  Deno.addSignalListener("SIGINT", () => handleSignal("SIGINT"));
  Deno.addSignalListener("SIGTERM", () => handleSignal("SIGTERM"));

  // Wait for child to complete
  const status = await child.status;

  // Return appropriate code
  if (signalReceived) {
    // Convention: 128 + signal number
    return signalReceived === "SIGINT" ? 130 : 143;
  }
  return status.code;
}

/**
 * Run multiple tasks sequentially (fail-fast) or in parallel.
 */
export async function cmdTasks(
  tasks: ResolvedTask[],
  args: string[],
  parallel: boolean
): Promise<number> {
  // Single task - simple case
  if (tasks.length === 1) {
    return runTask(tasks[0], args);
  }

  // Multiple tasks with args is an error
  if (args.length > 0) {
    logError("Cannot pass arguments when running multiple tasks");
    return 1;
  }

  // Sequential execution (fail-fast)
  if (!parallel) {
    for (const task of tasks) {
      log(`${COLORS.dim}→ ${task.path}${COLORS.reset}`);
      const code = await runTask(task, []);
      if (code !== 0) {
        logError(`Task '${task.path}' failed with exit code ${code}`);
        return code;
      }
    }
    return 0;
  }

  // Parallel execution - all tasks run, failures shown immediately
  const results = await Promise.all(
    tasks.map(async (task) => {
      log(`${COLORS.dim}→ ${task.path}${COLORS.reset}`);
      const code = await runTask(task, []);
      // Show failure immediately when it happens
      if (code !== 0) {
        logError(`Task '${task.path}' failed with exit code ${code}`);
      }
      return { task, code };
    })
  );

  // Return first non-zero exit code, or 0 if all succeeded
  const failed = results.find((r) => r.code !== 0);
  return failed ? failed.code : 0;
}

/**
 * List all tasks in the config.
 */
export function cmdTaskList(
  config: Config,
  groupFilter?: string
): void {
  const tasks = getAllTasks(config);

  // Filter by group if specified
  const filtered = groupFilter
    ? tasks.filter((t) => t.group === groupFilter)
    : tasks;

  if (filtered.length === 0) {
    if (groupFilter) {
      logError(`No tasks found in group '${groupFilter}'`);
    } else {
      logError("No tasks defined in config");
    }
    Deno.exit(1);
  }

  print("");
  for (const task of filtered) {
    // Truncate command if too long
    const maxCmdLen = 50;
    const cmd = task.command.length > maxCmdLen
      ? task.command.slice(0, maxCmdLen - 3) + "..."
      : task.command;
    const desc = task.description ? ` ${c("dim")}${task.description}${c("reset")}` : "";
    print(`${c("cyan")}${task.path.padEnd(30)}${c("reset")} ${cmd}${desc}`);
  }
  print("");
}

// ============================================================================
// SETUP & CONFIG DISPLAY
// ============================================================================

export async function cmdInit(): Promise<void> {
  // Check if config already exists
  for (const name of CONFIG_NAMES) {
    try {
      await Deno.stat(name);
      logError(`${name} already exists`);
      Deno.exit(1);
    } catch {
      // File doesn't exist, continue
    }
  }

  const template = `groups:
  myapp:
    services:
      api:
        command: npm start
        working_dir: ./somewhere
        environment:
          PORT: 3000
`;

  try {
    await Deno.writeTextFile("rig.yaml", template);
  } catch (err) {
    logError(`Failed to create rig.yaml: ${err instanceof Error ? err.message : String(err)}`);
    Deno.exit(1);
  }
  logSystem("Created rig.yaml");
}

export function cmdConfig(
  config: Config,
  targets: ResolvedService[],
  allServices: ResolvedService[],
  raw: boolean,
  json: boolean
): void {
  if (targets.length === 0) {
    logError("No matching services found");
    return;
  }

  if (raw || json) {
    // Build output structure matching config format
    const output: { groups: Record<string, { services: Record<string, unknown> }> } = { groups: {} };

    for (const target of targets) {
      if (!output.groups[target.group]) {
        output.groups[target.group] = { services: {} };
      }

      const cleanDef: Record<string, unknown> = {};
      for (const [key, value] of Object.entries(target.def)) {
        if (value !== undefined) {
          cleanDef[key] = value;
        }
      }

      output.groups[target.group].services[target.name] = cleanDef;
    }

    if (json) {
      print(JSON.stringify(output, null, 2));
    } else {
      print(stringifyYaml(output));
    }
    return;
  }

  const skip = new Set(["command", "color", "tasks", "watch", "requirements"]);
  const cwd = Deno.cwd();

  for (const target of targets) {
    const color = getServiceColor(allServices, target.name);

    const props: string[] = [];
    for (const [key, value] of Object.entries(target.def)) {
      if (!value || skip.has(key)) continue;
      if (Array.isArray(value)) {
        props.push(`${key}=[${value.join(",")}]`);
      } else if (typeof value === "object") {
        for (const [k, v] of Object.entries(value)) props.push(`${k}=${v}`);
      } else {
        let v = String(value);
        if (v.startsWith(cwd + "/")) v = v.slice(cwd.length + 1);
        props.push(`${key}=${v}`);
      }
    }

    const propsStr = props.length > 0 ? ` ${c("dim")}${props.join(" ")}${c("reset")}` : "";
    print(`${c("dim")}${target.group.padEnd(12)}${c("reset")} ${c(color)}${target.name.padEnd(12)}${c("reset")} ${target.def.command}${propsStr}`);
  }
}

// ============================================================================
// DISCOVERY
// ============================================================================

/**
 * Scan for rig config files using fd.
 * Respects .gitignore to avoid pulling in rig files from dependencies.
 */
async function scanForRigFiles(rootDir: string): Promise<string[]> {
  // Require fd
  const fdExists = await (async () => {
    try {
      const proc = new Deno.Command("which", { args: ["fd"] });
      const { code } = await proc.output();
      return code === 0;
    } catch {
      return false;
    }
  })();

  if (!fdExists) {
    print(`
${c("red")}Error: fd is not installed${c("reset")}

fd is required for fast, gitignore-aware file scanning.

Install via the rig installer or manually:

  Installer:     curl -fsSL https://raw.githubusercontent.com/roelentless/rig/develop/install.sh | sh
  macOS:         brew install fd
  Ubuntu/Debian: sudo apt install fd-find
  Fedora:        sudo dnf install fd-find
  Arch:          sudo pacman -S fd
`);
    Deno.exit(1);
  }

  const fdArgs = [
    "--type", "f",
    "--hidden",         // Include hidden directories for completeness
    // Exclusions for common non-project directories
    "--exclude", "node_modules",
    "--exclude", ".git",
    "--exclude", "vendor",
    "--exclude", ".rig",
    "--exclude", "__pycache__",
    "--exclude", ".venv",
    "--exclude", "dist",
    "--exclude", "build",
    // Pattern for rig config files (regex)
    "(^rig\\.ya?ml$|.*\\.rig\\.yaml$)",
    rootDir,
  ];

  const cmd = new Deno.Command("fd", { args: fdArgs });
  const { stdout, code } = await cmd.output();
  if (code !== 0) {
    return [];
  }

  const output = new TextDecoder().decode(stdout).trim();
  if (!output) {
    return [];
  }

  return output.split("\n").filter(Boolean);
}

/**
 * Get the current imports from a config file.
 */
async function getCurrentImports(configPath: string): Promise<string[]> {
  try {
    const { raw } = await parseConfigFile(configPath);
    return raw.imports ?? [];
  } catch {
    return [];
  }
}

/**
 * Discover rig files and suggest/update imports.
 */
export async function cmdDiscover(rootDir: string, dryRun: boolean, autoAccept: boolean): Promise<void> {
  const absRoot = rootDir.startsWith("/") ? rootDir : `${Deno.cwd()}/${rootDir}`;

  print(`Scanning from ${absRoot}...\n`);

  // Find all rig files
  const allFiles = await scanForRigFiles(absRoot);

  if (allFiles.length === 0) {
    print("No rig files found.");
    return;
  }

  // Sort files by path for consistent output
  allFiles.sort();

  // Find the root config (in the absRoot directory)
  const rootConfigs = allFiles.filter((f) => {
    const dir = f.replace(/\/[^/]+$/, "");
    return dir === absRoot || dir === absRoot.replace(/\/$/, "");
  });

  if (rootConfigs.length === 0) {
    print(`No root config found in ${absRoot}`);
    print("\nFound rig files:");
    for (const f of allFiles) {
      const rel = f.startsWith(absRoot) ? f.slice(absRoot.length + 1) : f;
      print(`  ${rel}`);
    }
    print("\nCreate a rig.yaml in the root directory and add imports.");
    return;
  }

  // Use first root config (prefer rig.yaml over others)
  const rootConfig = rootConfigs.find((f) => f.endsWith("/rig.yaml")) ?? rootConfigs[0];
  const rootConfigRel = rootConfig.startsWith(absRoot + "/")
    ? rootConfig.slice(absRoot.length + 1)
    : rootConfig.split("/").pop() ?? rootConfig;

  print("Found rig files:");
  for (const f of allFiles) {
    const rel = f.startsWith(absRoot + "/") ? f.slice(absRoot.length + 1) : f.split("/").pop() ?? f;
    const isRoot = f === rootConfig;
    print(`  ${rel}${isRoot ? " (root)" : ""}`);
  }

  // Get current imports from root config
  const currentImports = await getCurrentImports(rootConfig);

  print(`\nCurrent imports in ${rootConfigRel}:`);
  if (currentImports.length === 0) {
    print("  (none)");
  } else {
    for (const imp of currentImports) {
      print(`  - ${imp}`);
    }
  }

  // Find files that aren't imported (excluding the root config itself)
  const importedSet = new Set(currentImports.map((imp) =>
    imp.startsWith("/") ? imp : `${absRoot}/${imp}`
  ));

  const missing: string[] = [];
  for (const f of allFiles) {
    if (f === rootConfig) continue;
    if (!importedSet.has(f)) {
      const rel = f.startsWith(absRoot + "/") ? f.slice(absRoot.length + 1) : f;
      missing.push(rel);
    }
  }

  if (missing.length === 0) {
    print("\nAll rig files are imported. Nothing to do.");
    return;
  }

  print("\nMissing (not imported):");
  for (const m of missing) {
    print(`  + ${m}`);
  }

  if (dryRun) {
    print("\n[Dry run] Would add the above imports to rig.yaml");
    return;
  }

  // Prompt user unless auto-accept
  if (!autoAccept) {
    const answer = prompt("\nAdd to imports? [y/N]");
    if (answer?.toLowerCase() !== "y") {
      print("Aborted.");
      return;
    }
  }

  // Update the root config with new imports
  const content = await Deno.readTextFile(rootConfig);
  const raw = parseYaml(content) as Record<string, unknown>;

  const newImports = [...(raw.imports as string[] ?? []), ...missing];
  raw.imports = newImports;

  // Rebuild YAML preserving structure
  const newContent = stringifyYaml(raw);
  await Deno.writeTextFile(rootConfig, newContent);

  print(`\nUpdated ${rootConfigRel} with ${missing.length} new import(s).`);
}
