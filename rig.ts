#!/usr/bin/env -S deno run -A

/**
 * rig - A lightweight process manager using tmux.
 * Inspired by docker-compose, but not aiming for compatibility.
 * No state files - tmux is the source of truth.
 */

const VERSION = "0.1.3";

import { parse as parseYaml } from "jsr:@std/yaml";
import { parseArgs } from "jsr:@std/cli/parse-args";

// ============================================================================
// TYPES
// ============================================================================

interface ServiceDef {
  command: string;
  working_dir: string;
  environment?: Record<string, string>;
  color?: string;
}

interface Config {
  group: string;
  services: Record<string, ServiceDef>;
}

interface SessionStatus {
  name: string;
  running: boolean;
  pid?: number;
  exitCode?: number;
  created?: number;
}

interface ProcessMetrics {
  memoryMB: number;
  cpuPercent: number;
  ports: number[];
  processCount: number;
}

// ============================================================================
// CONSTANTS
// ============================================================================

const COLORS: Record<string, string> = {
  cyan: "\x1b[36m",
  yellow: "\x1b[33m",
  magenta: "\x1b[35m",
  green: "\x1b[32m",
  blue: "\x1b[34m",
  orange: "\x1b[38;5;208m",
  red: "\x1b[31m",
  lavender: "\x1b[38;5;183m",
  pink: "\x1b[38;5;213m",
  teal: "\x1b[38;5;51m",
  lime: "\x1b[38;5;154m",
  coral: "\x1b[38;5;209m",
  sky: "\x1b[38;5;117m",
  gold: "\x1b[38;5;220m",
  violet: "\x1b[38;5;135m",
  reset: "\x1b[0m",
  dim: "\x1b[2m",
  bold: "\x1b[1m",
};

// Deterministic color palette for services (assigned by index)
// Order matters: avoid red/yellow early (error/warning associations)
// First ~10 should be distinct, non-alarming colors
const SERVICE_COLORS = [
  "cyan",
  "yellow",
  "magenta",
  "teal",
  "green",
  "blue",
  "orange",
  "pink",
  "lavender",
  "violet",
  "lime",
  "coral",
  "sky",
  "gold",
  "violet",
];

const CONFIG_NAMES = ["rig.yaml", "rig.yml"];

// Disable colors when not a TTY (piping to other commands)
const IS_TTY = Deno.stdout.isTerminal();

// ============================================================================
// UTILITIES
// ============================================================================

// Get color code only if TTY
function c(color: string): string {
  if (!IS_TTY) return "";
  return COLORS[color] ?? "";
}

// Safe print that handles broken pipe (EPIPE) gracefully
function print(msg: string): void {
  try {
    console.log(msg);
  } catch (e) {
    if (e instanceof Error && e.message.includes("Broken pipe")) {
      Deno.exit(0);
    }
    throw e;
  }
}

function log(msg: string, prefix?: string, color?: string): void {
  const now = new Date();
  const ts =
    now.toLocaleTimeString("en-US", {
      hour12: false,
      hour: "2-digit",
      minute: "2-digit",
      second: "2-digit",
    }) +
    "." +
    now.getMilliseconds().toString().padStart(3, "0");

  const colorCode = color ? c(color) : "";
  const prefixStr = prefix
    ? `${colorCode}${prefix.padEnd(12)}${c("reset")} `
    : "";

  print(`${c("dim")}${ts}${c("reset")} ${prefixStr}${msg}`);
}

function logSystem(msg: string): void {
  log(msg, "rig", "bold");
}

function logError(msg: string): void {
  log(msg, "rig", "red");
}

function buildEnvString(env: Record<string, string>): string {
  return Object.entries(env)
    .map(([k, v]) => `${k}=${JSON.stringify(v)}`)
    .join(" ");
}

function getServiceColor(services: Record<string, ServiceDef>, serviceName: string): string {
  // Allow config override, otherwise use deterministic color by index
  const def = services[serviceName];
  if (def?.color) return def.color;
  
  const index = Object.keys(services).indexOf(serviceName);
  return SERVICE_COLORS[index % SERVICE_COLORS.length];
}

// ============================================================================
// PROCESS METRICS
// ============================================================================

async function getProcessTree(rootPid: number): Promise<number[]> {
  const allPids = new Set<number>([rootPid]);
  const toCheck = [rootPid];

  while (toCheck.length > 0) {
    const parentPid = toCheck.shift()!;
    try {
      const cmd = new Deno.Command("pgrep", { args: ["-P", String(parentPid)] });
      const { stdout } = await cmd.output();
      const output = new TextDecoder().decode(stdout).trim();
      if (output) {
        for (const line of output.split("\n")) {
          const pid = parseInt(line.trim(), 10);
          if (!isNaN(pid) && !allPids.has(pid)) {
            allPids.add(pid);
            toCheck.push(pid);
          }
        }
      }
    } catch {
      // Process might be dead
    }
  }

  return Array.from(allPids);
}

async function getProcessMetrics(rootPid: number): Promise<ProcessMetrics> {
  const pids = await getProcessTree(rootPid);
  
  let totalMemoryKB = 0;
  let totalCpu = 0;
  const ports = new Set<number>();

  // Get memory and CPU from ps
  if (pids.length > 0) {
    try {
      const cmd = new Deno.Command("ps", {
        args: ["-o", "pid,rss,%cpu", "-p", pids.join(",")],
      });
      const { stdout } = await cmd.output();
      const lines = new TextDecoder().decode(stdout).trim().split("\n");
      
      // Skip header
      for (let i = 1; i < lines.length; i++) {
        const parts = lines[i].trim().split(/\s+/);
        if (parts.length >= 3) {
          totalMemoryKB += parseInt(parts[1], 10) || 0;
          totalCpu += parseFloat(parts[2]) || 0;
        }
      }
    } catch {
      // ps failed
    }
  }

  // Get listening ports from lsof - need to filter by PID in output since -p doesn't filter with -i on macOS
  if (pids.length > 0) {
    try {
      const cmd = new Deno.Command("lsof", {
        args: ["-i", "-P", "-n"],
      });
      const { stdout } = await cmd.output();
      const output = new TextDecoder().decode(stdout);
      const pidSet = new Set(pids.map(String));

      for (const line of output.split("\n")) {
        if (line.includes("LISTEN")) {
          // Line format: COMMAND PID USER FD TYPE DEVICE SIZE/OFF NODE NAME
          const parts = line.split(/\s+/);
          if (parts.length >= 2 && pidSet.has(parts[1])) {
            // Extract port from NAME column like *:4001 or localhost:4001
            const match = line.match(/:(\d+)\s+\(LISTEN\)/);
            if (match) {
              ports.add(parseInt(match[1], 10));
            }
          }
        }
      }
    } catch {
      // lsof failed
    }
  }

  return {
    memoryMB: Math.round(totalMemoryKB / 1024),
    cpuPercent: Math.round(totalCpu * 10) / 10,
    ports: Array.from(ports).sort((a, b) => a - b),
    processCount: pids.length,
  };
}

// ============================================================================
// TMUX CHECK
// ============================================================================

async function checkTmux(): Promise<boolean> {
  try {
    const cmd = new Deno.Command("which", { args: ["tmux"] });
    const { code } = await cmd.output();
    return code === 0;
  } catch {
    return false;
  }
}

function printTmuxInstallGuide(): void {
  print(`
${c("red")}Error: tmux is not installed${c("reset")}

rig requires tmux to manage background processes.

Install tmux:

  macOS:        brew install tmux
  Ubuntu/Debian: sudo apt install tmux
  Fedora:       sudo dnf install tmux
  Arch:         sudo pacman -S tmux

After installing, run this command again.
`);
}

// ============================================================================
// CONFIG
// ============================================================================

async function findConfig(): Promise<string> {
  for (const name of CONFIG_NAMES) {
    try {
      await Deno.stat(name);
      return name;
    } catch {
      // Continue
    }
  }
  throw new Error(`Config file not found. Expected: ${CONFIG_NAMES.join(" or ")}`);
}

async function loadConfig(configPath?: string): Promise<{ config: Config; configDir: string }> {
  const path = configPath ?? (await findConfig());
  const content = await Deno.readTextFile(path);
  const raw = parseYaml(content) as Record<string, unknown>;

  if (!raw.group || typeof raw.group !== "string") {
    throw new Error("Config must have a 'group' field (string)");
  }

  if (!raw.services || typeof raw.services !== "object") {
    throw new Error("Config must have a 'services' field (object)");
  }

  const configDir = Deno.cwd();
  const services: Record<string, ServiceDef> = {};

  for (const [name, def] of Object.entries(raw.services as Record<string, unknown>)) {
    const d = def as Record<string, unknown>;
    if (!d.command || typeof d.command !== "string") {
      throw new Error(`Service '${name}' must have a 'command' field`);
    }
    if (!d.working_dir || typeof d.working_dir !== "string") {
      throw new Error(`Service '${name}' must have a 'working_dir' field`);
    }

    // Resolve relative working_dir paths
    let working_dir = d.working_dir as string;
    if (!working_dir.startsWith("/")) {
      working_dir = `${configDir}/${working_dir}`;
    }

    // Convert all environment values to strings for noob-friendliness
    const environment = d.environment
      ? Object.fromEntries(
          Object.entries(d.environment as Record<string, unknown>).map(([k, v]) => [k, String(v)])
        )
      : undefined;

    services[name] = {
      command: d.command as string,
      working_dir,
      environment,
      color: d.color as string | undefined,
    };
  }

  return {
    config: { group: raw.group as string, services },
    configDir,
  };
}

// ============================================================================
// SESSION MANAGER
// ============================================================================

class SessionManager {
  constructor(private group: string) {}

  sessionName(service: string): string {
    return `${this.group}-${service}`;
  }

  serviceFromSession(sessionName: string): string | null {
    const prefix = `${this.group}-`;
    if (sessionName.startsWith(prefix)) {
      return sessionName.slice(prefix.length);
    }
    return null;
  }

  async start(service: string, def: ServiceDef): Promise<void> {
    const session = this.sessionName(service);

    // Check if already running
    if (await this.exists(service)) {
      const status = await this.status(service);
      if (status.running) {
        logSystem(`${service} is already running (pid ${status.pid})`);
        return;
      }
      // Dead session exists, kill it first
      await this.stop(service);
    }

    // Build command with environment vars
    const envStr = def.environment ? buildEnvString(def.environment) + " " : "";
    const cmd = `${envStr}exec ${def.command}`;

    // Create tmux session
    const tmux = new Deno.Command("tmux", {
      args: ["new-session", "-d", "-s", session, "-c", def.working_dir, cmd],
    });
    const result = await tmux.output();

    if (result.code !== 0) {
      const err = new TextDecoder().decode(result.stderr);
      throw new Error(`Failed to start ${service}: ${err}`);
    }

    // Enable remain-on-exit to preserve crash output
    await new Deno.Command("tmux", {
      args: ["set-option", "-t", session, "remain-on-exit", "on"],
    }).output();

    // Get PID
    const status = await this.status(service);
    logSystem(`Started ${service} (pid ${status.pid})`);
  }

  async stop(service: string): Promise<void> {
    const session = this.sessionName(service);

    if (!(await this.exists(service))) {
      return;
    }

    logSystem(`Stopping ${service}...`);

    const cmd = new Deno.Command("tmux", {
      args: ["kill-session", "-t", session],
    });
    await cmd.output();
  }

  async exists(service: string): Promise<boolean> {
    const session = this.sessionName(service);
    const cmd = new Deno.Command("tmux", {
      args: ["has-session", "-t", session],
    });
    const { code } = await cmd.output();
    return code === 0;
  }

  async status(service: string): Promise<SessionStatus> {
    const session = this.sessionName(service);

    if (!(await this.exists(service))) {
      return { name: service, running: false };
    }

    const cmd = new Deno.Command("tmux", {
      args: [
        "display-message",
        "-t",
        session,
        "-p",
        "#{pane_pid}:#{pane_dead}:#{pane_dead_status}:#{session_created}",
      ],
    });
    const { stdout } = await cmd.output();
    const output = new TextDecoder().decode(stdout).trim();
    const [pid, dead, exitCode, created] = output.split(":");

    return {
      name: service,
      running: dead !== "1",
      pid: Number(pid),
      exitCode: dead === "1" ? Number(exitCode) : undefined,
      created: Number(created),
    };
  }

  async listAll(): Promise<SessionStatus[]> {
    const cmd = new Deno.Command("tmux", {
      args: [
        "list-sessions",
        "-F",
        "#{session_name}:#{pane_pid}:#{pane_dead}:#{pane_dead_status}:#{session_created}",
      ],
    });
    const { stdout, code } = await cmd.output();

    if (code !== 0) {
      return []; // No tmux server running
    }

    const output = new TextDecoder().decode(stdout).trim();
    if (!output) return [];

    const prefix = `${this.group}-`;
    return output
      .split("\n")
      .filter((line) => line.startsWith(prefix))
      .map((line) => {
        const [sessionName, pid, dead, exitCode, created] = line.split(":");
        return {
          name: sessionName.slice(prefix.length),
          running: dead !== "1",
          pid: Number(pid),
          exitCode: dead === "1" ? Number(exitCode) : undefined,
          created: Number(created),
        };
      });
  }

  async logs(service: string, lines?: number): Promise<string> {
    const session = this.sessionName(service);

    if (!(await this.exists(service))) {
      return "";
    }

    const args = ["capture-pane", "-t", session, "-p"];
    if (lines) {
      args.push("-S", `-${lines}`);
    } else {
      args.push("-S", "-");
    }

    const cmd = new Deno.Command("tmux", { args });
    const { stdout } = await cmd.output();
    return new TextDecoder().decode(stdout);
  }

}

// ============================================================================
// COMMANDS
// ============================================================================

async function cmdStart(
  mgr: SessionManager,
  config: Config,
  names: string[],
  detached: boolean
): Promise<void> {
  const serviceNames =
    names.length > 0 ? names : Object.keys(config.services);

  // Validate service names
  for (const name of serviceNames) {
    if (!config.services[name]) {
      throw new Error(`Unknown: ${name}`);
    }
  }

  logSystem(`Starting ${serviceNames.length} process(es)...`);

  // Start all services
  for (const name of serviceNames) {
    await mgr.start(name, config.services[name]);
  }

  if (detached) {
    logSystem("Processes started in background");
    return;
  }

  // Monitor mode
  await monitor(mgr, config, serviceNames);
}

async function monitor(
  mgr: SessionManager,
  config: Config,
  serviceNames: string[]
): Promise<void> {
  logSystem("Monitoring processes... (Ctrl+C to stop all)");

  const lastLines: Record<string, number> = {};
  const deadServices = new Set<string>();
  for (const svc of serviceNames) lastLines[svc] = 0;

  let running = true;

  // Handle Ctrl+C
  Deno.addSignalListener("SIGINT", async () => {
    if (!running) return;
    running = false;
    logSystem("Received SIGINT, stopping all processes...");
    await cmdStop(mgr, config, serviceNames);
    Deno.exit(0);
  });

  Deno.addSignalListener("SIGTERM", async () => {
    if (!running) return;
    running = false;
    logSystem("Received SIGTERM, stopping all processes...");
    await cmdStop(mgr, config, serviceNames);
    Deno.exit(0);
  });

  // Poll loop
  while (running) {
    for (const svc of serviceNames) {
      if (!running) break;
      if (deadServices.has(svc)) continue;

      const logs = await mgr.logs(svc);
      const lines = logs.split("\n");
      const newLines = lines.slice(lastLines[svc]);

      const color = getServiceColor(config.services, svc);
      for (const line of newLines) {
        if (line.trim()) {
          log(line, svc, color);
        }
      }
      lastLines[svc] = lines.length;

      // Check if dead - just report, don't kill others
      const status = await mgr.status(svc);
      if (!status.running && status.exitCode !== undefined && !deadServices.has(svc)) {
        log(`Exited with code ${status.exitCode}`, svc, "red");
        deadServices.add(svc);
      }
    }

    await new Promise((r) => setTimeout(r, 100));
  }
}

async function cmdStop(
  mgr: SessionManager,
  config: Config,
  names: string[]
): Promise<void> {
  const serviceNames =
    names.length > 0 ? names : Object.keys(config.services);

  let stoppedAny = false;

  // Always loop through all services - idempotent
  for (const name of serviceNames) {
    if (await mgr.exists(name)) {
      await mgr.stop(name);
      stoppedAny = true;
    }
  }

  if (stoppedAny) {
    logSystem("All processes stopped");
  } else {
    logSystem("No processes were running");
  }
}

async function cmdRestart(
  mgr: SessionManager,
  config: Config,
  names: string[]
): Promise<void> {
  const serviceNames =
    names.length > 0 ? names : Object.keys(config.services);

  for (const name of serviceNames) {
    if (!config.services[name]) {
      throw new Error(`Unknown: ${name}`);
    }
  }

  logSystem(`Restarting ${serviceNames.length} process(es)...`);

  for (const name of serviceNames) {
    await mgr.stop(name);
    await mgr.start(name, config.services[name]);
  }
}

async function cmdPs(mgr: SessionManager, config: Config, showAll: boolean): Promise<void> {
  const sessions = await mgr.listAll();
  const allServices = Object.keys(config.services);

  print("");
  
  if (showAll) {
    print(
      `${c("bold")}SERVICE        STATUS       MEM    CPU  PORTS            UPTIME      PID${c("reset")}`
    );
    print("─".repeat(80));
  } else {
    print(`${c("bold")}SERVICE        STATUS       UPTIME${c("reset")}`);
    print("─".repeat(40));
  }

  for (const svc of allServices) {
    const session = sessions.find((s) => s.name === svc);
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

      const svcCol = svc.padEnd(14);
      const statusCol = `${c(statusColor)}${statusText.padEnd(12)}${c("reset")}`;
      const memCol = mem.padStart(5);
      const cpuCol = cpu.padStart(5);
      const portsCol = ports.padEnd(16);
      const uptimeCol = uptime.padEnd(10);
      print(`${svcCol} ${statusCol} ${memCol} ${cpuCol}  ${portsCol} ${uptimeCol} ${pid}`);
    } else {
      const statusCol = `${c(statusColor)}${statusText.padEnd(12)}${c("reset")}`;
      print(`${svc.padEnd(14)} ${statusCol} ${uptime}`);
    }
  }
  print("");
}

async function cmdTop(mgr: SessionManager, config: Config): Promise<void> {
  const allServices = Object.keys(config.services);
  
  // Track metrics and last update time per service
  const metrics: Record<string, ProcessMetrics & { lastUpdate: number }> = {};
  const sessions: Record<string, SessionStatus> = {};
  
  // Initialize
  for (const svc of allServices) {
    metrics[svc] = { memoryMB: 0, cpuPercent: 0, ports: [], processCount: 0, lastUpdate: 0 };
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

  function cleanup() {
    running = false;
    Deno.stdin.setRaw(false);
    // Clear screen and restore cursor on exit (like regular top)
    Deno.stdout.writeSync(new TextEncoder().encode(CLEAR + SHOW_CURSOR));
  }

  // Ensure cursor is shown on exit
  Deno.addSignalListener("SIGINT", () => {
    cleanup();
    Deno.exit(0);
  });

  // Set up raw mode to capture keypresses
  Deno.stdin.setRaw(true);
  
  // Non-blocking key reader
  const keyReader = async () => {
    const buf = new Uint8Array(1);
    while (running) {
      try {
        const n = await Deno.stdin.read(buf);
        if (n === null) break;
        // 'q', 'Q', or Ctrl+C (byte 3)
        if (buf[0] === 113 || buf[0] === 81 || buf[0] === 3) {
          cleanup();
          Deno.exit(0);
        }
      } catch {
        break;
      }
    }
  };
  keyReader(); // Start listening (don't await)

  Deno.stdout.writeSync(new TextEncoder().encode(HIDE_CURSOR));

  let tick = 0;

  while (running) {
    const now = Date.now();

    // Update session status for all services (cheap operation)
    const sessionList = await mgr.listAll();
    for (const svc of allServices) {
      const session = sessionList.find((s) => s.name === svc);
      sessions[svc] = session ?? { name: svc, running: false };
    }

    // Smart refresh: update 2-3 services per tick based on their refresh interval
    let updated = 0;
    for (const svc of allServices) {
      if (!sessions[svc]?.running || !sessions[svc]?.pid) continue;
      
      const interval = getRefreshInterval(svc);
      const timeSinceUpdate = now - metrics[svc].lastUpdate;
      
      if (timeSinceUpdate >= interval && updated < 3) {
        const m = await getProcessMetrics(sessions[svc].pid!);
        metrics[svc] = { ...m, lastUpdate: now };
        updated++;
      }
    }

    // Render
    let output = CLEAR;
    output += `${c("bold")}rig top${c("reset")} - press q or Ctrl+C to exit\n\n`;
    output += `${c("bold")}SERVICE        STATUS       MEM    CPU  PORTS            STARTED${c("reset")}\n`;
    output += "─".repeat(72) + "\n";

    for (const svc of allServices) {
      const session = sessions[svc];
      const m = metrics[svc];
      
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

      const svcCol = svc.padEnd(14);
      const statusCol = status.padEnd(20);
      const memCol = mem.padStart(5);
      const cpuCol = cpu.padEnd(10);
      const portsCol = ports.padEnd(16);

      output += `${svcCol} ${statusCol} ${memCol} ${cpuCol} ${portsCol} ${started}\n`;
    }

    Deno.stdout.writeSync(new TextEncoder().encode(output));
    
    tick++;
    await new Promise((r) => setTimeout(r, 500));
  }
}

async function cmdLogs(
  mgr: SessionManager,
  config: Config,
  serviceName: string | undefined,
  follow: boolean
): Promise<void> {
  const serviceNames = serviceName ? [serviceName] : Object.keys(config.services);

  // Validate service exists
  if (serviceName && !config.services[serviceName]) {
    logError(`Unknown: ${serviceName}`);
    Deno.exit(1);
  }

  // Check at least one service is running
  let anyRunning = false;
  for (const svc of serviceNames) {
    if (await mgr.exists(svc)) {
      anyRunning = true;
      break;
    }
  }

  if (!anyRunning) {
    logError(serviceName ? `'${serviceName}' is not running` : "No processes running");
    Deno.exit(1);
  }

  if (follow) {
    // Follow mode - tail logs, Ctrl+C just exits (doesn't stop services)
    const lastLines: Record<string, number> = {};
    for (const svc of serviceNames) lastLines[svc] = 0;

    let running = true;

    Deno.addSignalListener("SIGINT", () => {
      running = false;
    });

    // Initial dump of existing logs
    for (const svc of serviceNames) {
      if (!(await mgr.exists(svc))) continue;
      const logs = await mgr.logs(svc);
      const lines = logs.split("\n");
      const color = getServiceColor(config.services, svc);
      for (const line of lines) {
        if (line.trim()) {
          log(line, svc, color);
        }
      }
      lastLines[svc] = lines.length;
    }

    // Follow new output
    while (running) {
      for (const svc of serviceNames) {
        if (!running) break;
        if (!(await mgr.exists(svc))) continue;

        const logs = await mgr.logs(svc);
        const lines = logs.split("\n");
        const newLines = lines.slice(lastLines[svc]);

        const color = getServiceColor(config.services, svc);
        for (const line of newLines) {
          if (line.trim()) {
            log(line, svc, color);
          }
        }
        lastLines[svc] = lines.length;
      }
      await new Promise((r) => setTimeout(r, 100));
    }
  } else {
    // Dump mode - show all logs and exit
    for (const svc of serviceNames) {
      if (!(await mgr.exists(svc))) continue;
      const logs = await mgr.logs(svc);
      const color = getServiceColor(config.services, svc);
      for (const line of logs.split("\n")) {
        if (line.trim()) {
          log(line, svc, color);
        }
      }
    }
  }
}

async function cmdInit(): Promise<void> {
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

  const template = `group: myapp

services:
  api:
    command: deno run -A server.ts
    working_dir: ./backend
    environment:
      PORT: "3000"

  web:
    command: npm run dev
    working_dir: ./frontend
`;

  await Deno.writeTextFile("rig.yaml", template);
  logSystem("Created rig.yaml");
}

// ============================================================================
// CLI
// ============================================================================

function printUsage(): void {
  print(`
rig - lightweight, tmux-based process manager

USAGE:
  rig <command> [options] [names...]

COMMANDS:
  init                      Create rig.yaml in current directory
  start/up [names...]       Start processes (foreground, streaming logs)
  start/up -d [names...]    Start processes in background (detached)
  stop/down [names...]      Stop processes
  restart [names...]        Restart processes
  ps/list [-f|--full]       Show status (add -f for mem/cpu/ports)
  top                       Live dashboard with auto-refreshing metrics
  logs/tail [-f] [name]     Show logs (all or specific process)
  version                   Show version

EXAMPLES:
  rig up                    Start all processes
  rig up -d                 Start all in background
  rig start api worker      Start specific processes
  rig down                  Stop all processes
  rig restart api           Restart single process
  rig ps                    Show status
  rig logs                  Dump all logs
  rig logs -f               Follow all logs (Ctrl+C to exit)
  rig logs api              Dump api logs
  rig logs -f api           Follow api logs

CONFIG:
  Looks for rig.yaml or rig.yml in current directory.
`);
}

async function main(): Promise<void> {
  // Check tmux is installed
  if (!(await checkTmux())) {
    printTmuxInstallGuide();
    Deno.exit(1);
  }

  // Parse args with @std/cli
  const args = parseArgs(Deno.args, {
    boolean: ["d", "f", "full", "help", "h", "V", "version"],
    alias: { f: "full", h: "help", V: "version" },
  });

  const [command, ...services] = args._ as string[];

  if (args.version || command === "version") {
    print(VERSION);
    Deno.exit(0);
  }

  if (!command || args.help || command === "help") {
    printUsage();
    Deno.exit(0);
  }

  // Commands that need config
  if (["start", "up", "stop", "down", "restart", "ps", "list", "top"].includes(command)) {
    try {
      const { config } = await loadConfig();
      const mgr = new SessionManager(config.group);

      switch (command) {
        case "start":
        case "up":
          await cmdStart(mgr, config, services, args.d);
          break;
        case "stop":
        case "down":
          await cmdStop(mgr, config, services);
          break;
        case "restart":
          await cmdRestart(mgr, config, services);
          break;
        case "ps":
        case "list":
          await cmdPs(mgr, config, args.full);
          break;
        case "top":
          await cmdTop(mgr, config);
          break;
      }
    } catch (err) {
      if (err instanceof Error && err.message.includes("Config file not found")) {
        console.error("No config file found. Run 'rig init' to create one.");
        Deno.exit(1);
      }
      throw err;
    }
  } else if (command === "init") {
    await cmdInit();
  } else if (command === "logs" || command === "tail") {
    try {
      const { config } = await loadConfig();
      const mgr = new SessionManager(config.group);
      await cmdLogs(mgr, config, services[0], args.f);
    } catch (err) {
      if (err instanceof Error && err.message.includes("Config file not found")) {
        console.error("No config file found. Run 'rig init' to create one.");
        Deno.exit(1);
      }
      throw err;
    }
  } else {
    logError(`Unknown command: ${command}`);
    printUsage();
    Deno.exit(1);
  }
}

main();
