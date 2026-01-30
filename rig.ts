#!/usr/bin/env -S deno run -A

/**
 * rig - A lightweight process manager using tmux.
 * Inspired by docker-compose, but not aiming for compatibility.
 * No state files - tmux is the source of truth.
 */

// JSR imports required for global install - deno install resolves JSR packages correctly
// deno-lint-ignore no-import-prefix
import { parse as parseYaml, stringify as stringifyYaml } from "jsr:@std/yaml@^1.0.11";
// deno-lint-ignore no-import-prefix
import { parseArgs } from "jsr:@std/cli@^1.0.25/parse-args";
// deno-lint-ignore no-import-prefix
import { parse as parseEnv } from "jsr:@std/dotenv@^0.225";

import { VERSION } from "./version.ts";

// ============================================================================
// TYPES
// ============================================================================

interface HealthCheck {
  grace_ms?: number;
}

interface EnvFileEntry {
  path: string;
  required?: boolean;  // defaults to true
}

interface ServiceDef {
  command: string;
  working_dir: string;
  environment?: Record<string, string>;
  env_file?: string | EnvFileEntry[];
  color?: string;
  depends_on?: string[];
  healthcheck?: HealthCheck;
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
const LOG_DIR = ".rig/logs";

// Disable colors when not a TTY (piping to other commands)
const IS_TTY = Deno.stdout.isTerminal();

// ASCII key codes for cmdTop
const KEY_Q_LOWER = 113;
const KEY_Q_UPPER = 81;
const KEY_CTRL_C = 3;

// Global verbose flag
let VERBOSE = false;

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
    if (e instanceof Deno.errors.BrokenPipe) {
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

function logVerbose(msg: string): void {
  if (VERBOSE) {
    log(msg, "rig", "dim");
  }
}

function buildEnvString(env: Record<string, string>): string {
  return Object.entries(env)
    .map(([k, v]) => `${k}=${JSON.stringify(v)}`)
    .join(" ");
}

async function loadEnvFiles(
  entries: EnvFileEntry[],
  configDir: string
): Promise<Record<string, string>> {
  const result: Record<string, string> = {};

  for (const entry of entries) {
    // Resolve path relative to config dir
    const fullPath = entry.path.startsWith("/")
      ? entry.path
      : `${configDir}/${entry.path}`;

    try {
      const content = await Deno.readTextFile(fullPath);
      const parsed = parseEnv(content);
      Object.assign(result, parsed);
    } catch (err) {
      if (entry.required !== false) {
        throw new Error(`Failed to load env file '${entry.path}': ${err instanceof Error ? err.message : String(err)}`);
      }
      // required: false - silently skip
    }
  }

  return result;
}

function getServiceColor(services: Record<string, ServiceDef>, serviceName: string): string {
  // Allow config override, otherwise use deterministic color by index
  const def = services[serviceName];
  if (def?.color) return def.color;

  const index = Object.keys(services).indexOf(serviceName);
  return SERVICE_COLORS[index % SERVICE_COLORS.length];
}

/**
 * Strip ANSI control codes that would mess up log prefixes.
 * Preserves color codes but removes cursor movement, line clearing, etc.
 */
function stripControlCodes(line: string): string {
  return line
    .replace(/\r/g, "")                     // Carriage return
    .replace(/\x1b\[\d*[ABCD]/g, "")        // Cursor movement (up/down/forward/back)
    .replace(/\x1b\[\d*;\d*[Hf]/g, "")      // Cursor position
    .replace(/\x1b\[\d*G/g, "")             // Cursor to column
    .replace(/\x1b\[\d*[JK]/g, "")          // Clear screen/line
    .replace(/\x1b\[\?25[lh]/g, "");        // Hide/show cursor
}

async function ensureGitignore(configDir: string): Promise<void> {
  const gitignorePath = `${configDir}/.gitignore`;

  try {
    const content = await Deno.readTextFile(gitignorePath);
    // Check if .rig is already in gitignore (with or without trailing slash)
    const lines = content.split("\n");
    if (lines.some((line) => line.trim() === ".rig" || line.trim() === ".rig/")) {
      return; // Already present
    }
    // Append .rig/ to existing gitignore
    const newContent = content.endsWith("\n") ? content + ".rig/\n" : content + "\n.rig/\n";
    await Deno.writeTextFile(gitignorePath, newContent);
  } catch {
    // No .gitignore exists, create one
    await Deno.writeTextFile(gitignorePath, ".rig/\n");
  }
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
    } catch (err) {
      logVerbose(`pgrep failed for parent PID ${parentPid}: ${err instanceof Error ? err.message : String(err)}`);
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
    } catch (err) {
      logVerbose(`ps command failed: ${err instanceof Error ? err.message : String(err)}`);
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
    } catch (err) {
      logVerbose(`lsof command failed: ${err instanceof Error ? err.message : String(err)}`);
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

  let content: string;
  try {
    content = await Deno.readTextFile(path);
  } catch (err) {
    throw new Error(`Failed to read config file '${path}': ${err instanceof Error ? err.message : String(err)}`);
  }

  let raw: Record<string, unknown>;
  try {
    raw = parseYaml(content) as Record<string, unknown>;
  } catch (err) {
    throw new Error(`Invalid YAML in ${path}: ${err instanceof Error ? err.message : String(err)}`);
  }

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

    // Parse depends_on
    let depends_on: string[] | undefined;
    if (d.depends_on) {
      if (!Array.isArray(d.depends_on)) {
        throw new Error(`Service '${name}' depends_on must be an array`);
      }
      depends_on = d.depends_on as string[];
    }

    // Parse healthcheck
    let healthcheck: HealthCheck | undefined;
    if (d.healthcheck) {
      const hc = d.healthcheck as Record<string, unknown>;
      healthcheck = {};
      if (hc.grace_ms !== undefined) {
        healthcheck.grace_ms = Number(hc.grace_ms);
      }
    }

    // Parse env_file and merge with inline environment
    let envFileEntries: EnvFileEntry[] = [];
    if (d.env_file) {
      if (typeof d.env_file === "string") {
        envFileEntries = [{ path: d.env_file, required: true }];
      } else if (Array.isArray(d.env_file)) {
        envFileEntries = (d.env_file as unknown[]).map((entry) => {
          if (typeof entry === "string") {
            return { path: entry, required: true };
          }
          const e = entry as Record<string, unknown>;
          return {
            path: e.path as string,
            required: e.required !== false,  // default true
          };
        });
      }
    }

    // Load env files and merge (inline environment overrides env_file)
    let mergedEnvironment = environment;
    if (envFileEntries.length > 0) {
      const envFromFiles = await loadEnvFiles(envFileEntries, configDir);
      mergedEnvironment = { ...envFromFiles, ...environment };
    }

    services[name] = {
      command: d.command as string,
      working_dir,
      environment: mergedEnvironment,
      color: d.color as string | undefined,
      depends_on,
      healthcheck,
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
  constructor(private group: string, private configDir: string) {}

  // Log file paths - grouped by config group name
  logDir(service: string): string {
    return `${this.configDir}/${LOG_DIR}/${this.group}/${service}`;
  }

  logFile(service: string, previous = false): string {
    return `${this.logDir(service)}/${previous ? "previous" : "current"}.log`;
  }

  async rotateLog(service: string): Promise<void> {
    const dir = this.logDir(service);
    const current = this.logFile(service);
    const previous = this.logFile(service, true);

    // Create log directory if needed
    await Deno.mkdir(dir, { recursive: true });

    // Ensure .rig is in .gitignore
    await ensureGitignore(this.configDir);

    // Rotate: current -> previous
    try {
      await Deno.rename(current, previous);
    } catch {
      // current doesn't exist, that's fine
    }

    // Create empty current log file
    await Deno.writeTextFile(current, "");
  }

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

    // Set up log file with pipe-pane
    await this.rotateLog(service);
    const logFile = this.logFile(service);
    await new Deno.Command("tmux", {
      args: ["pipe-pane", "-t", session, "-o", `cat >> "${logFile}"`],
    }).output();

    // Capture any output that happened before pipe-pane was set up
    const existingOutput = await this.logs(service);
    if (existingOutput.trim()) {
      await Deno.writeTextFile(logFile, existingOutput, { append: true });
    }

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

  async kill(service: string): Promise<void> {
    const session = this.sessionName(service);

    if (!(await this.exists(service))) {
      return;
    }

    logSystem(`Killing ${service}...`);

    // Get PID and kill process tree with SIGKILL
    const status = await this.status(service);
    if (status.pid) {
      const pids = await getProcessTree(status.pid);
      // Concurrent kill - blast them all at once
      for (const pid of pids) {
        try {
          Deno.kill(pid, "SIGKILL");
        } catch {
          // Already dead, ignore
        }
      }
    }

    // Clean up tmux session
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
// LOG STREAMING
// ============================================================================

/**
 * Stream logs from multiple services using tail -F on their log files.
 * Returns cleanup function to stop all tail processes.
 */
function streamLogs(
  mgr: SessionManager,
  services: string[],
  config: Config,
  options: { previous?: boolean } = {}
): { cleanup: () => void } {
  const tails: Deno.ChildProcess[] = [];
  const aborted = { value: false };

  for (const svc of services) {
    const logFile = mgr.logFile(svc, options.previous);
    const color = getServiceColor(config.services, svc);

    // tail -F follows by name (handles rotation/creation)
    // -s 0.1 = check every 100ms for changes (default 1s is too slow)
    // -n +1 = start from beginning of file
    const proc = new Deno.Command("tail", {
      args: ["-F", "-s", "0.1", "-n", "+1", logFile],
      stdout: "piped",
      stderr: "piped", // suppress "file replaced" messages
    }).spawn();

    tails.push(proc);

    // Read lines and log them
    (async () => {
      const reader = proc.stdout.getReader();
      const decoder = new TextDecoder();
      let buffer = "";

      try {
        while (!aborted.value) {
          const { done, value } = await reader.read();
          if (done) break;

          buffer += decoder.decode(value, { stream: true });
          const lines = buffer.split("\n");
          buffer = lines.pop() ?? "";

          for (const line of lines) {
            // Strip control codes that would overwrite the prefix
            const cleanLine = stripControlCodes(line);
            if (cleanLine.trim()) {
              log(cleanLine, svc, color);
            }
          }
        }
      } catch {
        // Reader closed, ignore
      }
    })();
  }

  return {
    cleanup: () => {
      aborted.value = true;
      for (const proc of tails) {
        try {
          proc.kill("SIGTERM");
        } catch {
          // Already dead
        }
      }
    },
  };
}

// ============================================================================
// DEPENDENCY ORDERING
// ============================================================================

/**
 * Compute startup order based on depends_on relationships.
 * Returns services grouped by "level" - services in the same level can start together,
 * but must wait for previous levels to complete.
 */
function computeStartupOrder(
  services: Record<string, ServiceDef>,
  requestedNames: string[]
): string[][] {
  const requested = new Set(requestedNames);

  // Build dependency graph (only for requested services)
  const deps: Record<string, Set<string>> = {};
  for (const name of requestedNames) {
    deps[name] = new Set();
    const svcDeps = services[name]?.depends_on ?? [];
    for (const dep of svcDeps) {
      // Only include dependencies that are in the requested set
      if (requested.has(dep)) {
        deps[name].add(dep);
      }
    }
  }

  // Kahn's algorithm for topological sort with levels
  const levels: string[][] = [];
  const remaining = new Set(requestedNames);

  while (remaining.size > 0) {
    // Find all services with no remaining dependencies
    const level: string[] = [];
    for (const name of remaining) {
      const unresolvedDeps = [...deps[name]].filter((d) => remaining.has(d));
      if (unresolvedDeps.length === 0) {
        level.push(name);
      }
    }

    if (level.length === 0) {
      // Circular dependency - just start remaining in any order
      logSystem("Warning: circular dependency detected, starting remaining services");
      levels.push([...remaining]);
      break;
    }

    levels.push(level);
    for (const name of level) {
      remaining.delete(name);
    }
  }

  return levels;
}

/**
 * Get the maximum grace_ms for a set of services
 */
function getMaxGraceMs(services: Record<string, ServiceDef>, names: string[]): number {
  let maxGrace = 0;
  for (const name of names) {
    const grace = services[name]?.healthcheck?.grace_ms ?? 0;
    if (grace > maxGrace) {
      maxGrace = grace;
    }
  }
  return maxGrace;
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

  // Validate depends_on references
  for (const name of serviceNames) {
    const deps = config.services[name]?.depends_on ?? [];
    for (const dep of deps) {
      if (!config.services[dep]) {
        throw new Error(`Service '${name}' depends on unknown service '${dep}'`);
      }
    }
  }

  logSystem(`Starting ${serviceNames.length} process(es)...`);

  // Compute startup order based on dependencies
  const levels = computeStartupOrder(config.services, serviceNames);

  // Start services level by level
  for (let i = 0; i < levels.length; i++) {
    const level = levels[i];

    // Start all services in this level
    for (const name of level) {
      await mgr.start(name, config.services[name]);
    }

    // If there are more levels, wait for grace period before starting next level
    if (i < levels.length - 1) {
      const graceMs = getMaxGraceMs(config.services, level);
      if (graceMs > 0) {
        logVerbose(`Waiting ${graceMs}ms grace period for: ${level.join(", ")}`);
        await new Promise((r) => setTimeout(r, graceMs));
      }
    }
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

  let running = true;
  let stopping = false;

  // Start streaming logs
  const { cleanup } = streamLogs(mgr, serviceNames, config);

  // Handle Ctrl+C - ensure only one signal handler executes cleanup
  const handleShutdown = async (signal: string) => {
    if (stopping) return;
    stopping = true;
    if (!running) return;
    running = false;
    cleanup();
    logSystem(`Received ${signal}, stopping all processes...`);
    await cmdStop(mgr, config, serviceNames);
    Deno.exit(0);
  };

  Deno.addSignalListener("SIGINT", () => handleShutdown("SIGINT"));
  Deno.addSignalListener("SIGTERM", () => handleShutdown("SIGTERM"));

  // Monitor for dead services
  const deadServices = new Set<string>();
  while (running) {
    for (const svc of serviceNames) {
      if (!running) break;
      if (deadServices.has(svc)) continue;

      const status = await mgr.status(svc);
      if (!status.running && status.exitCode !== undefined) {
        log(`Exited with code ${status.exitCode}`, svc, "red");
        deadServices.add(svc);
      }
    }

    await new Promise((r) => setTimeout(r, 500));
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

async function cmdKill(
  mgr: SessionManager,
  config: Config,
  names: string[]
): Promise<void> {
  const serviceNames =
    names.length > 0 ? names : Object.keys(config.services);

  let killedAny = false;

  for (const name of serviceNames) {
    if (await mgr.exists(name)) {
      await mgr.kill(name);
      killedAny = true;
    }
  }

  if (killedAny) {
    logSystem("All processes killed");
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
  } finally {
    cleanup();
  }
}

async function cmdLogs(
  mgr: SessionManager,
  config: Config,
  serviceName: string | undefined,
  follow: boolean,
  previous: boolean
): Promise<void> {
  const serviceNames = serviceName ? [serviceName] : Object.keys(config.services);

  // Validate service exists
  if (serviceName && !config.services[serviceName]) {
    logError(`Unknown: ${serviceName}`);
    Deno.exit(1);
  }

  // Check if log files exist
  let anyLogs = false;
  for (const svc of serviceNames) {
    const logFile = mgr.logFile(svc, previous);
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
    logError(serviceName ? `No logs for '${serviceName}'` : "No logs found");
    Deno.exit(1);
  }

  if (follow && previous) {
    logError("Cannot follow previous logs");
    Deno.exit(1);
  }

  if (follow) {
    // Follow mode - stream logs, Ctrl+C just exits (doesn't stop services)
    let running = true;
    const { cleanup } = streamLogs(mgr, serviceNames, config);

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
    for (const svc of serviceNames) {
      const logFile = mgr.logFile(svc, previous);
      const color = getServiceColor(config.services, svc);
      try {
        const content = await Deno.readTextFile(logFile);
        for (const line of content.split("\n")) {
          const cleanLine = stripControlCodes(line);
          if (cleanLine.trim()) {
            log(cleanLine, svc, color);
          }
        }
      } catch {
        // No logs for this service
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

function cmdConfig(config: Config, serviceNames: string[], raw: boolean, json: boolean): void {
  const servicesToShow =
    serviceNames.length > 0
      ? serviceNames.filter((s) => config.services[s])
      : Object.keys(config.services);

  if (servicesToShow.length === 0) {
    logError("No matching services found");
    return;
  }

  if (raw || json) {
    const output: Config = {
      group: config.group,
      services: {},
    };

    for (const name of servicesToShow) {
      const def = config.services[name];
      const cleanDef: Record<string, unknown> = {};
      
      for (const [key, value] of Object.entries(def)) {
        if (value !== undefined) {
          cleanDef[key] = value;
        }
      }
      
      output.services[name] = cleanDef as unknown as ServiceDef;
    }

    if (json) {
      print(JSON.stringify(output, null, 2));
    } else {
      print(stringifyYaml(output));
    }
    return;
  }

  const skip = new Set(["command", "color"]);
  const cwd = Deno.cwd();

  for (const name of servicesToShow) {
    const def = config.services[name];
    const color = getServiceColor(config.services, name);

    const props: string[] = [];
    for (const [key, value] of Object.entries(def)) {
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
    print(`${c(color)}${name.padEnd(12)}${c("reset")} ${def.command}${propsStr}`);
  }
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
  stop/down [names...]      Stop processes (graceful)
  kill [names...]           Force kill with SIGKILL
  restart [names...]        Restart processes
  ps/list [-f|--full]       Show status (add -f for mem/cpu/ports)
  top                       Live dashboard with auto-refreshing metrics
  logs/tail [-f] [--prev] [name]  Show logs (--prev for last run)
  config [--raw|--json] [names...] Show tmux commands (--raw for YAML, --json for JSON)
  version                   Show version

OPTIONS:
  -v, --verbose             Enable verbose logging for debugging

EXAMPLES:
  rig up                    Start all processes
  rig up -d                 Start all in background
  rig start api worker      Start specific processes
  rig down                  Stop all processes (graceful)
  rig kill                  Force kill all processes
  rig kill api              Force kill specific process
  rig restart api           Restart single process
  rig ps                    Show status
  rig logs                  Dump all logs
  rig logs -f               Follow all logs (Ctrl+C to exit)
  rig logs api              Dump api logs
  rig logs -f api           Follow api logs
  rig logs --prev           Show previous run's logs
  rig config                Show all tmux commands
  rig config --raw          Show raw YAML config
  rig config --json         Show raw JSON config
  rig config api            Show command for specific process
  rig config --raw api      Show raw YAML for specific service
  rig config --json api     Show raw JSON for specific service

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
    boolean: ["d", "f", "full", "help", "h", "V", "version", "raw", "json", "verbose", "v", "prev"],
    alias: { f: "full", h: "help", V: "version", v: "verbose" },
  });

  // Set global verbose flag
  VERBOSE = args.verbose;

  const [command, ...servicesRaw] = args._.map(String);
  const services = servicesRaw.flatMap((s) => s.split(",").map((n) => n.trim()).filter((n) => n.length > 0));

  if (args.version || command === "version") {
    print(VERSION);
    Deno.exit(0);
  }

  if (!command || args.help || command === "help") {
    printUsage();
    Deno.exit(0);
  }

  // Commands that need config
  if (["start", "up", "stop", "down", "kill", "restart", "ps", "list", "top", "config"].includes(command)) {
    try {
      const { config, configDir } = await loadConfig();
      const mgr = new SessionManager(config.group, configDir);

      switch (command) {
        case "start":
        case "up":
          await cmdStart(mgr, config, services, args.d);
          break;
        case "stop":
        case "down":
          await cmdStop(mgr, config, services);
          break;
        case "kill":
          await cmdKill(mgr, config, services);
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
        case "config":
          cmdConfig(config, services, args.raw, args.json);
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
      const { config, configDir } = await loadConfig();
      const mgr = new SessionManager(config.group, configDir);
      await cmdLogs(mgr, config, services[0], args.f, args.prev);
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
