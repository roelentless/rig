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

interface TaskDef {
  command: string;
  working_dir?: string;                    // Required for group tasks, inherited for service tasks
  environment?: Record<string, string>;
  env_file?: string | EnvFileEntry[];
  description?: string;
}

interface ServiceDef {
  command: string;
  working_dir: string;
  environment?: Record<string, string>;
  env_file?: string | EnvFileEntry[];
  color?: string;
  depends_on?: string[];
  healthcheck?: HealthCheck;
  tasks?: Record<string, TaskDef>;         // Service-level tasks
}

interface GroupDef {
  services?: Record<string, ServiceDef>;   // Optional - group can have only tasks
  tasks?: Record<string, TaskDef>;         // Group-level tasks
}

interface Config {
  groups: Record<string, GroupDef>;
}

// Raw config as parsed from YAML (before processing)
interface RawConfig {
  imports?: string[];
  groups?: Record<string, unknown>;
}

// Context for recursive config loading
interface LoadContext {
  configPath: string;      // absolute path to this rig file
  configDir: string;       // dirname (execution context for this file)
  loaded: Set<string>;     // already-imported files (dedup by absolute path)
  importChain: string[];   // for circular import detection
}

// Resolved task with merged config from hierarchy
interface ResolvedTask {
  path: string;           // e.g., "backend.api.build" or "backend.deploy"
  group: string;
  service?: string;       // undefined for group-level tasks
  name: string;           // task name
  command: string;
  working_dir: string;
  environment?: Record<string, string>;
  description?: string;
}

// Flattened service for easy lookup
interface ResolvedService {
  group: string;
  name: string;
  def: ServiceDef;
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
const CONFIG_PATTERN = /^(rig\.ya?ml|.*\.rig\.yaml)$/;  // matches rig.yaml, rig.yml, *.rig.yaml
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

function getServiceColor(allServices: ResolvedService[], serviceName: string): string {
  // Allow config override, otherwise use deterministic color by index
  const idx = allServices.findIndex((s) => s.name === serviceName);
  if (idx === -1) return SERVICE_COLORS[0];

  const def = allServices[idx].def;
  if (def?.color) return def.color;

  return SERVICE_COLORS[idx % SERVICE_COLORS.length];
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

/**
 * Normalize a path by resolving . and .. components.
 */
function normalizePath(path: string): string {
  const parts = path.split("/");
  const result: string[] = [];

  for (const part of parts) {
    if (part === "..") {
      if (result.length > 0 && result[result.length - 1] !== "..") {
        result.pop();
      } else if (!path.startsWith("/")) {
        result.push(part);
      }
    } else if (part !== "." && part !== "") {
      result.push(part);
    }
  }

  const normalized = result.join("/");
  return path.startsWith("/") ? "/" + normalized : normalized;
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

/**
 * Walk upward from startDir to find the nearest rig config file.
 * Checks for rig.yaml, rig.yml first, then *.rig.yaml files.
 * Returns absolute path to the config file.
 */
async function findNearestConfig(startDir: string): Promise<string> {
  // Resolve to absolute path
  let dir = startDir.startsWith("/") ? startDir : await Deno.realPath(startDir);

  while (true) {
    // Check for standard names first (rig.yaml, rig.yml)
    for (const name of CONFIG_NAMES) {
      const path = `${dir}/${name}`;
      try {
        await Deno.stat(path);
        return path;
      } catch {
        // Continue
      }
    }

    // Check for *.rig.yaml files
    try {
      for await (const entry of Deno.readDir(dir)) {
        if (entry.isFile && entry.name.endsWith(".rig.yaml") && entry.name !== "rig.yaml") {
          return `${dir}/${entry.name}`;
        }
      }
    } catch {
      // Directory not readable, continue to parent
    }

    // Move to parent directory
    const parent = dir.replace(/\/[^/]+$/, "") || "/";
    if (parent === dir) {
      // Reached filesystem root
      throw new Error("No rig config found (searched up to filesystem root). Expected: rig.yaml, rig.yml, or *.rig.yaml");
    }
    dir = parent;
  }
}

/**
 * Legacy function for backward compatibility - finds config in current directory only.
 */
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

/**
 * Normalize a path for env_file entries, resolving relative to configDir.
 */
function normalizeEnvFilePath(envFile: string, configDir: string): string {
  if (envFile.startsWith("/")) {
    return envFile;
  }
  return `${configDir}/${envFile}`;
}

/**
 * Parse a single rig config file and expand paths relative to its directory.
 * Does not process imports - returns raw parsed content with expanded paths.
 */
async function parseConfigFile(configPath: string): Promise<{ raw: RawConfig; configDir: string }> {
  let content: string;
  try {
    content = await Deno.readTextFile(configPath);
  } catch (err) {
    throw new Error(`Failed to read config file '${configPath}': ${err instanceof Error ? err.message : String(err)}`);
  }

  let raw: RawConfig;
  try {
    raw = parseYaml(content) as RawConfig;
  } catch (err) {
    throw new Error(`Invalid YAML in ${configPath}: ${err instanceof Error ? err.message : String(err)}`);
  }

  // Handle empty file or null content
  if (!raw) {
    raw = {};
  }

  const configDir = configPath.replace(/\/[^/]+$/, "");
  return { raw, configDir };
}

/**
 * Recursively load a config tree, processing imports and merging groups.
 * Each file's paths are expanded relative to its own location before merging.
 */
async function loadConfigRecursive(ctx: LoadContext): Promise<{ groups: Record<string, GroupDef>; seenServices: Map<string, string> }> {
  const { configPath, configDir, loaded, importChain } = ctx;

  // Circular import check
  if (importChain.includes(configPath)) {
    const cycle = [...importChain, configPath].map(p => p.replace(Deno.cwd() + "/", "./")).join("\n  → ");
    throw new Error(`Circular import detected:\n  ${cycle}`);
  }

  // Dedup check - already loaded this exact file
  if (loaded.has(configPath)) {
    return { groups: {}, seenServices: new Map() };
  }
  loaded.add(configPath);

  // Parse the file
  const { raw } = await parseConfigFile(configPath);

  // Track groups and services from this file
  const groups: Record<string, GroupDef> = {};
  const seenServices = new Map<string, string>(); // serviceName -> groupName

  // Process groups from this file
  if (raw.groups && typeof raw.groups === "object") {
    for (const [groupName, groupDef] of Object.entries(raw.groups as Record<string, unknown>)) {
      // Validate group name
      if (!/^[a-zA-Z0-9_-]+$/.test(groupName)) {
        throw new Error(`Invalid group name '${groupName}' in ${configPath}: must be alphanumeric with hyphens/underscores only`);
      }

      const g = groupDef as Record<string, unknown>;

      // Groups can have services, tasks, or both
      if (!g.services && !g.tasks) {
        throw new Error(`Group '${groupName}' in ${configPath} must have 'services' and/or 'tasks'`);
      }

      const services: Record<string, ServiceDef> = {};
      const groupTasks: Record<string, TaskDef> = {};

      // Parse services
      if (g.services && typeof g.services === "object") {
        for (const [name, def] of Object.entries(g.services as Record<string, unknown>)) {
          const d = def as Record<string, unknown>;
          if (!d.command || typeof d.command !== "string") {
            throw new Error(`Service '${groupName}.${name}' in ${configPath} must have a 'command' field`);
          }
          if (!d.working_dir || typeof d.working_dir !== "string") {
            throw new Error(`Service '${groupName}.${name}' in ${configPath} must have a 'working_dir' field`);
          }

          // Resolve relative working_dir paths relative to this config's directory
          let working_dir = d.working_dir as string;
          if (!working_dir.startsWith("/")) {
            working_dir = `${configDir}/${working_dir}`;
          }

          // Convert all environment values to strings
          const environment = d.environment
            ? Object.fromEntries(
                Object.entries(d.environment as Record<string, unknown>).map(([k, v]) => [k, String(v)])
              )
            : undefined;

          // Parse depends_on
          let depends_on: string[] | undefined;
          if (d.depends_on) {
            if (!Array.isArray(d.depends_on)) {
              throw new Error(`Service '${groupName}.${name}' in ${configPath} depends_on must be an array`);
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

          // Parse env_file - normalize paths relative to this config's directory
          let envFileEntries: EnvFileEntry[] = [];
          if (d.env_file) {
            if (typeof d.env_file === "string") {
              envFileEntries = [{ path: normalizeEnvFilePath(d.env_file, configDir), required: true }];
            } else if (Array.isArray(d.env_file)) {
              envFileEntries = (d.env_file as unknown[]).map((entry) => {
                if (typeof entry === "string") {
                  return { path: normalizeEnvFilePath(entry, configDir), required: true };
                }
                const e = entry as Record<string, unknown>;
                return {
                  path: normalizeEnvFilePath(e.path as string, configDir),
                  required: e.required !== false,
                };
              });
            }
          }

          // Load env files and merge (inline environment overrides env_file)
          let mergedEnvironment = environment;
          if (envFileEntries.length > 0) {
            const envFromFiles = await loadEnvFiles(envFileEntries, "/"); // paths already absolute
            mergedEnvironment = { ...envFromFiles, ...environment };
          }

          // Parse service-level tasks
          let serviceTasks: Record<string, TaskDef> | undefined;
          if (d.tasks && typeof d.tasks === "object") {
            serviceTasks = {};
            for (const [taskName, taskDef] of Object.entries(d.tasks as Record<string, unknown>)) {
              const r = taskDef as Record<string, unknown>;
              if (!r.command || typeof r.command !== "string") {
                throw new Error(`Task '${groupName}.${name}.${taskName}' in ${configPath} must have a 'command' field`);
              }

              // Resolve working_dir if specified
              let taskWorkingDir = r.working_dir as string | undefined;
              if (taskWorkingDir && !taskWorkingDir.startsWith("/")) {
                taskWorkingDir = `${configDir}/${taskWorkingDir}`;
              }

              // Parse env_file for service task
              let taskEnvFileEntries: EnvFileEntry[] = [];
              if (r.env_file) {
                if (typeof r.env_file === "string") {
                  taskEnvFileEntries = [{ path: normalizeEnvFilePath(r.env_file, configDir), required: true }];
                } else if (Array.isArray(r.env_file)) {
                  taskEnvFileEntries = (r.env_file as unknown[]).map((entry) => {
                    if (typeof entry === "string") {
                      return { path: normalizeEnvFilePath(entry, configDir), required: true };
                    }
                    const e = entry as Record<string, unknown>;
                    return {
                      path: normalizeEnvFilePath(e.path as string, configDir),
                      required: e.required !== false,
                    };
                  });
                }
              }

              // Load env files for service task
              let taskEnvironment = r.environment
                ? Object.fromEntries(
                    Object.entries(r.environment as Record<string, unknown>).map(([k, v]) => [k, String(v)])
                  )
                : undefined;
              if (taskEnvFileEntries.length > 0) {
                const envFromFiles = await loadEnvFiles(taskEnvFileEntries, "/");
                taskEnvironment = { ...envFromFiles, ...taskEnvironment };
              }

              serviceTasks[taskName] = {
                command: r.command as string,
                working_dir: taskWorkingDir,
                environment: taskEnvironment,
                description: r.description as string | undefined,
              };
            }
          }

          services[name] = {
            command: d.command as string,
            working_dir,
            environment: mergedEnvironment,
            color: d.color as string | undefined,
            depends_on,
            healthcheck,
            tasks: serviceTasks,
          };

          seenServices.set(name, groupName);
        }
      }

      // Parse group-level tasks
      if (g.tasks && typeof g.tasks === "object") {
        for (const [taskName, taskDef] of Object.entries(g.tasks as Record<string, unknown>)) {
          const r = taskDef as Record<string, unknown>;
          if (!r.command || typeof r.command !== "string") {
            throw new Error(`Task '${groupName}.${taskName}' in ${configPath} must have a 'command' field`);
          }
          if (!r.working_dir || typeof r.working_dir !== "string") {
            throw new Error(`Task '${groupName}.${taskName}' in ${configPath} must have a 'working_dir' field (group-level tasks cannot inherit)`);
          }

          // Resolve relative working_dir
          let taskWorkingDir = r.working_dir as string;
          if (!taskWorkingDir.startsWith("/")) {
            taskWorkingDir = `${configDir}/${taskWorkingDir}`;
          }

          // Parse env_file for group task
          let taskEnvFileEntries: EnvFileEntry[] = [];
          if (r.env_file) {
            if (typeof r.env_file === "string") {
              taskEnvFileEntries = [{ path: normalizeEnvFilePath(r.env_file, configDir), required: true }];
            } else if (Array.isArray(r.env_file)) {
              taskEnvFileEntries = (r.env_file as unknown[]).map((entry) => {
                if (typeof entry === "string") {
                  return { path: normalizeEnvFilePath(entry, configDir), required: true };
                }
                const e = entry as Record<string, unknown>;
                return {
                  path: normalizeEnvFilePath(e.path as string, configDir),
                  required: e.required !== false,
                };
              });
            }
          }

          // Load env files for group task
          let taskEnvironment = r.environment
            ? Object.fromEntries(
                Object.entries(r.environment as Record<string, unknown>).map(([k, v]) => [k, String(v)])
              )
            : undefined;
          if (taskEnvFileEntries.length > 0) {
            const envFromFiles = await loadEnvFiles(taskEnvFileEntries, "/");
            taskEnvironment = { ...envFromFiles, ...taskEnvironment };
          }

          groupTasks[taskName] = {
            command: r.command as string,
            working_dir: taskWorkingDir,
            environment: taskEnvironment,
            description: r.description as string | undefined,
          };
        }
      }

      groups[groupName] = {
        services: Object.keys(services).length > 0 ? services : undefined,
        tasks: Object.keys(groupTasks).length > 0 ? groupTasks : undefined,
      };
    }
  }

  // Process imports recursively
  if (raw.imports && Array.isArray(raw.imports)) {
    for (const importPath of raw.imports) {
      // Resolve import path relative to this config's directory and normalize
      let absImportPath = importPath.startsWith("/")
        ? importPath
        : `${configDir}/${importPath}`;

      // Normalize path (resolve .. and .)
      absImportPath = normalizePath(absImportPath);

      // Check if import exists
      try {
        await Deno.stat(absImportPath);
      } catch {
        throw new Error(`Import not found: ${importPath}\n  in ${configPath}`);
      }

      const childCtx: LoadContext = {
        configPath: absImportPath,
        configDir: absImportPath.replace(/\/[^/]+$/, ""),
        loaded,                                    // shared dedup set
        importChain: [...importChain, configPath], // track for circular detection
      };

      const childResult = await loadConfigRecursive(childCtx);

      // Merge child groups into current groups, checking for duplicates
      for (const [groupName, groupDef] of Object.entries(childResult.groups)) {
        if (groups[groupName]) {
          throw new Error(`Duplicate group '${groupName}' defined in:\n  - ${configPath}\n  - ${absImportPath}`);
        }
        groups[groupName] = groupDef;
      }

      // Merge child services, checking for duplicates
      for (const [serviceName, groupName] of childResult.seenServices) {
        if (seenServices.has(serviceName)) {
          throw new Error(
            `Duplicate service '${serviceName}' found in:\n` +
            `  - group '${seenServices.get(serviceName)}'\n` +
            `  - group '${groupName}' in ${absImportPath}`
          );
        }
        seenServices.set(serviceName, groupName);
      }
    }
  }

  return { groups, seenServices };
}

/**
 * Load the full config tree starting from a root config file.
 */
async function loadConfigTree(rootPath: string): Promise<{ config: Config; configDir: string }> {
  // Normalize to absolute path
  const absPath = rootPath.startsWith("/") ? rootPath : `${Deno.cwd()}/${rootPath}`;
  const configDir = absPath.replace(/\/[^/]+$/, "");

  const ctx: LoadContext = {
    configPath: absPath,
    configDir,
    loaded: new Set(),
    importChain: [],
  };

  const result = await loadConfigRecursive(ctx);

  // Validate depends_on references exist across all groups
  for (const [groupName, groupDef] of Object.entries(result.groups)) {
    if (!groupDef.services) continue;
    for (const [serviceName, serviceDef] of Object.entries(groupDef.services)) {
      for (const dep of serviceDef.depends_on ?? []) {
        if (!result.seenServices.has(dep)) {
          throw new Error(`Service '${groupName}.${serviceName}' depends on unknown service '${dep}'`);
        }
      }
    }
  }

  return {
    config: { groups: result.groups },
    configDir,
  };
}

/**
 * Load config from a path (uses tree-based loading with import support).
 * If no path is provided, searches upward from CWD for nearest config.
 */
async function loadConfig(configPath?: string): Promise<{ config: Config; configDir: string }> {
  const path = configPath ?? (await findNearestConfig(Deno.cwd()));
  return loadConfigTree(path);
}

/**
 * Build a flat lookup map from service name to its group and definition.
 * Since service names are unique across groups, this enables O(1) lookup.
 */
function buildServiceLookup(config: Config): Map<string, ResolvedService> {
  const lookup = new Map<string, ResolvedService>();
  for (const [groupName, groupDef] of Object.entries(config.groups)) {
    if (!groupDef.services) continue;
    for (const [serviceName, serviceDef] of Object.entries(groupDef.services)) {
      lookup.set(serviceName, { group: groupName, name: serviceName, def: serviceDef });
    }
  }
  return lookup;
}

/**
 * Get all services from config as a flat array.
 */
function getAllServices(config: Config): ResolvedService[] {
  const services: ResolvedService[] = [];
  for (const [groupName, groupDef] of Object.entries(config.groups)) {
    if (!groupDef.services) continue;
    for (const [serviceName, serviceDef] of Object.entries(groupDef.services)) {
      services.push({ group: groupName, name: serviceName, def: serviceDef });
    }
  }
  return services;
}

/**
 * Resolve a task path (e.g., "backend.api.build" or "backend.deploy") to a ResolvedTask.
 * For service-level tasks, merges service config with task config.
 */
function resolveTask(path: string, config: Config): ResolvedTask {
  const parts = path.split(".");

  if (parts.length < 2 || parts.length > 3) {
    throw new Error(`Invalid task path '${path}'. Use 'group.task' or 'group.service.task'`);
  }

  const [groupName, secondPart, thirdPart] = parts;

  const groupDef = config.groups[groupName];
  if (!groupDef) {
    throw new Error(`Unknown group '${groupName}'`);
  }

  if (parts.length === 2) {
    // Could be group.task or group.service (with implicit task name?)
    // First try group-level task
    const taskName = secondPart;
    if (groupDef.tasks?.[taskName]) {
      const taskDef = groupDef.tasks[taskName];
      return {
        path,
        group: groupName,
        name: taskName,
        command: taskDef.command,
        working_dir: taskDef.working_dir!, // Required for group tasks
        environment: taskDef.environment,
        description: taskDef.description,
      };
    }

    // Not a group task - error
    throw new Error(`Unknown task '${path}'. Did you mean 'group.service.task'?`);
  }

  // parts.length === 3: group.service.task
  const serviceName = secondPart;
  const taskName = thirdPart;

  const serviceDef = groupDef.services?.[serviceName];
  if (!serviceDef) {
    throw new Error(`Unknown service '${groupName}.${serviceName}'`);
  }

  const taskDef = serviceDef.tasks?.[taskName];
  if (!taskDef) {
    throw new Error(`Unknown task '${path}'`);
  }

  // Merge service config with task config
  // Order: service env -> task env (task overrides)
  const mergedEnv = taskDef.environment
    ? { ...serviceDef.environment, ...taskDef.environment }
    : serviceDef.environment;

  return {
    path,
    group: groupName,
    service: serviceName,
    name: taskName,
    command: taskDef.command,
    working_dir: taskDef.working_dir ?? serviceDef.working_dir, // Inherit from service
    environment: mergedEnv,
    description: taskDef.description,
  };
}

/**
 * Get all tasks from config as a flat array.
 */
function getAllTasks(config: Config): ResolvedTask[] {
  const tasks: ResolvedTask[] = [];

  for (const [groupName, groupDef] of Object.entries(config.groups)) {
    // Group-level tasks
    if (groupDef.tasks) {
      for (const [taskName, taskDef] of Object.entries(groupDef.tasks)) {
        tasks.push({
          path: `${groupName}.${taskName}`,
          group: groupName,
          name: taskName,
          command: taskDef.command,
          working_dir: taskDef.working_dir!,
          environment: taskDef.environment,
          description: taskDef.description,
        });
      }
    }

    // Service-level tasks
    if (groupDef.services) {
      for (const [serviceName, serviceDef] of Object.entries(groupDef.services)) {
        if (serviceDef.tasks) {
          for (const [taskName, taskDef] of Object.entries(serviceDef.tasks)) {
            const mergedEnv = taskDef.environment
              ? { ...serviceDef.environment, ...taskDef.environment }
              : serviceDef.environment;

            tasks.push({
              path: `${groupName}.${serviceName}.${taskName}`,
              group: groupName,
              service: serviceName,
              name: taskName,
              command: taskDef.command,
              working_dir: taskDef.working_dir ?? serviceDef.working_dir,
              environment: mergedEnv,
              description: taskDef.description,
            });
          }
        }
      }
    }
  }

  return tasks;
}

/**
 * Resolve CLI targets to a list of services.
 * - If groupNames provided: return all services in those groups
 * - If serviceNames provided: resolve each service name
 * - If neither: return all services
 */
function resolveTargets(
  config: Config,
  lookup: Map<string, ResolvedService>,
  serviceNames: string[],
  groupNames: string[]
): ResolvedService[] {
  // If -g flags provided, get all services from those groups
  if (groupNames.length > 0) {
    const services: ResolvedService[] = [];
    for (const groupName of groupNames) {
      const groupDef = config.groups[groupName];
      if (!groupDef) {
        throw new Error(`Unknown group: ${groupName}`);
      }
      if (!groupDef.services) continue;
      for (const [serviceName, serviceDef] of Object.entries(groupDef.services)) {
        services.push({ group: groupName, name: serviceName, def: serviceDef });
      }
    }
    return services;
  }

  // If service names provided, resolve them
  if (serviceNames.length > 0) {
    const services: ResolvedService[] = [];
    for (const name of serviceNames) {
      const resolved = lookup.get(name);
      if (!resolved) {
        throw new Error(`Unknown service: ${name}`);
      }
      services.push(resolved);
    }
    return services;
  }

  // No args - return all services
  return getAllServices(config);
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
  managers: Map<string, SessionManager>,
  targets: ResolvedService[],
  allServices: ResolvedService[],
  options: { previous?: boolean } = {}
): { cleanup: () => void } {
  const tails: Deno.ChildProcess[] = [];
  const aborted = { value: false };

  for (const target of targets) {
    const mgr = managers.get(target.group)!;
    const logFile = mgr.logFile(target.name, options.previous);
    const color = getServiceColor(allServices, target.name);

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
              log(cleanLine, target.name, color);
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
// COMMANDS
// ============================================================================

/**
 * Create SessionManagers for all groups that have services in the target list.
 */
function createManagers(targets: ResolvedService[], configDir: string): Map<string, SessionManager> {
  const managers = new Map<string, SessionManager>();
  for (const target of targets) {
    if (!managers.has(target.group)) {
      managers.set(target.group, new SessionManager(target.group, configDir));
    }
  }
  return managers;
}

/**
 * Create SessionManagers for all groups in config.
 */
function createAllManagers(config: Config, configDir: string): Map<string, SessionManager> {
  const managers = new Map<string, SessionManager>();
  for (const groupName of Object.keys(config.groups)) {
    managers.set(groupName, new SessionManager(groupName, configDir));
  }
  return managers;
}

async function cmdStart(
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

async function cmdStop(
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

async function cmdKill(
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

async function cmdRestart(
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

async function cmdPs(
  managers: Map<string, SessionManager>,
  targets: ResolvedService[],
  showAll: boolean
): Promise<void> {
  // Gather sessions from all groups
  const sessionsByService = new Map<string, SessionStatus>();
  for (const [groupName, mgr] of managers) {
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

async function cmdTop(
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

async function cmdLogs(
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
async function cmdTask(
  resolved: ResolvedTask,
  args: string[]
): Promise<void> {
  // Build the full command with args
  const fullCommand = args.length > 0
    ? `${resolved.command} ${args.map(shellEscape).join(" ")}`
    : resolved.command;

  logVerbose(`Running: ${fullCommand}`);
  logVerbose(`Working dir: ${resolved.working_dir}`);
  if (resolved.environment) {
    logVerbose(`Environment: ${Object.keys(resolved.environment).join(", ")}`);
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

  // Exit with appropriate code
  if (signalReceived) {
    // Convention: 128 + signal number
    Deno.exit(signalReceived === "SIGINT" ? 130 : 143);
  }
  Deno.exit(status.code);
}

/**
 * List all tasks in the config.
 */
function cmdTaskList(
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

function cmdConfig(
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

  const skip = new Set(["command", "color"]);
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
// DISCOVER COMMAND
// ============================================================================

/**
 * Check if a command exists in PATH.
 */
async function commandExists(cmd: string): Promise<boolean> {
  try {
    const proc = new Deno.Command("which", { args: [cmd] });
    const { code } = await proc.output();
    return code === 0;
  } catch {
    return false;
  }
}

/**
 * Scan for rig config files using fd.
 * Respects .gitignore to avoid pulling in rig files from dependencies.
 */
async function scanForRigFiles(rootDir: string): Promise<string[]> {
  // Require fd
  if (!(await commandExists("fd"))) {
    print(`
${c("red")}Error: fd is not installed${c("reset")}

rig discover requires fd for fast, gitignore-aware file scanning.

Install fd:

  macOS:         brew install fd
  Ubuntu/Debian: sudo apt install fd-find
  Fedora:        sudo dnf install fd-find
  Arch:          sudo pacman -S fd

After installing, run this command again.
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
async function cmdDiscover(rootDir: string, dryRun: boolean, autoAccept: boolean): Promise<void> {
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
  const raw = parseYaml(content) as RawConfig;

  const newImports = [...(raw.imports ?? []), ...missing];
  raw.imports = newImports;

  // Rebuild YAML preserving structure
  const newContent = stringifyYaml(raw as Record<string, unknown>);
  await Deno.writeTextFile(rootConfig, newContent);

  print(`\nUpdated ${rootConfigRel} with ${missing.length} new import(s).`);
}

// ============================================================================
// CLI
// ============================================================================

function printUsage(): void {
  print(`
rig - lightweight dev workflow tool for services and tasks

USAGE:
  rig <command> [options] [services...]

SERVICES:
  init                      Create rig.yaml in current directory
  start/up [services...]    Start processes (foreground, streaming logs)
  start/up -d [services...] Start processes in background (detached)
  stop/down [services...]   Stop processes (graceful)
  kill [services...]        Force kill with SIGKILL
  restart [services...]     Restart processes
  ps/list [-f|--full]       Show status (add -f for mem/cpu/ports)
  top                       Live dashboard with auto-refreshing metrics
  logs/tail [-f] [--prev] [service]  Show logs (--prev for last run)
  config [--raw|--json] [services...] Show config (--raw for YAML, --json for JSON)

TASKS:
  tasks [--group <name>]       List all tasks
  run/task <path> [args...]    Run a task (group.name or group.service.name)

MULTI-FILE:
  discover [--dry-run] [--yes] [path]  Scan for rig files and update imports

OTHER:
  version                   Show version
  help                      Show this help

OPTIONS:
  -g, --group <name>        Target entire group(s) instead of services
  -v, --verbose             Enable verbose logging for debugging

EXAMPLES:
  rig up                    Start all processes (all groups)
  rig up -d                 Start all in background
  rig start api worker      Start specific services
  rig start -g backend      Start all services in backend group
  rig down                  Stop all processes (graceful)
  rig stop -g backend       Stop all services in backend group
  rig kill                  Force kill all processes
  rig restart -g backend    Restart entire group
  rig ps                    Show status
  rig logs -f               Follow all logs
  rig logs --prev api       Show previous logs for api
  rig tasks                 List all tasks
  rig run backend.deploy    Run a group-level task
  rig run backend.api.build Run a service-level task
  rig run backend.api.test --watch  Pass args to a task
  rig config --json         Show raw JSON config
  rig discover              Scan for rig files and update imports
  rig discover --dry-run    Show what would be imported

CONFIG:
  Searches upward from current directory for rig.yaml, rig.yml, or *.rig.yaml.
  Supports imports to compose configs from multiple files:

    imports:
      - db/rig.yaml
      - backend/rig.yaml

  All imported files are merged into a flat namespace. Service names must be
  unique across all files. Circular imports are detected and reported.
`);
}

async function main(): Promise<void> {
  // Handle 'tasks' - list all tasks
  if (Deno.args[0] === "tasks") {
    const listArgs = parseArgs(Deno.args.slice(1), {
      boolean: ["help", "h", "verbose", "v"],
      string: ["g", "group"],
      collect: ["g", "group"],
      alias: { h: "help", v: "verbose", g: "group" },
    });

    VERBOSE = listArgs.verbose;

    if (listArgs.help) {
      printUsage();
      Deno.exit(0);
    }

    try {
      const { config } = await loadConfig();
      const groupFilter = (listArgs.group as string[] ?? [])[0];
      cmdTaskList(config, groupFilter);
      Deno.exit(0);
    } catch (err) {
      if (err instanceof Error) {
        logError(err.message);
      } else {
        throw err;
      }
      Deno.exit(1);
    }
    return;
  }

  // Handle 'discover' - scan for rig files and update imports
  if (Deno.args[0] === "discover") {
    const discoverArgs = parseArgs(Deno.args.slice(1), {
      boolean: ["help", "h", "dry-run", "yes", "y", "verbose", "v"],
      alias: { h: "help", y: "yes", v: "verbose" },
    });

    VERBOSE = discoverArgs.verbose;

    if (discoverArgs.help) {
      print(`
rig discover - scan for rig files and update imports

USAGE:
  rig discover [options] [path]

OPTIONS:
  --dry-run    Show what would be imported without making changes
  -y, --yes    Auto-accept changes without prompting
  -v, --verbose Enable verbose logging
  -h, --help   Show this help

DESCRIPTION:
  Scans for rig.yaml, rig.yml, and *.rig.yaml files starting from the
  specified path (or current directory). Shows which files are not yet
  imported in the root config and offers to add them.

  Uses 'fd' for fast, gitignore-aware scanning. Requires fd to be installed.
`);
      Deno.exit(0);
    }

    const path = (discoverArgs._.map(String)[0]) ?? Deno.cwd();
    await cmdDiscover(path, discoverArgs["dry-run"], discoverArgs.yes);
    Deno.exit(0);
  }

  // Handle 'run' or 'task' (singular) - execute a task
  if (Deno.args[0] === "run" || Deno.args[0] === "task") {
    // Parse flags only (not stopEarly) to detect -l/--list, -g, etc.
    const runArgs = parseArgs(Deno.args.slice(1), { // Skip "run"/"task"
      boolean: ["l", "list", "help", "h", "verbose", "v"],
      string: ["g", "group"],
      collect: ["g", "group"],
      alias: { l: "list", h: "help", v: "verbose", g: "group" },
      "--": true, // Collect everything after -- in a separate array
    });

    VERBOSE = runArgs.verbose;

    if (runArgs.help) {
      printUsage();
      Deno.exit(0);
    }

    try {
      const { config } = await loadConfig();
      const groupFilter = (runArgs.group as string[] ?? [])[0];

      if (runArgs.list) {
        cmdTaskList(config, groupFilter);
        Deno.exit(0);
      }

      // Find the task path (first positional after flags)
      // Then everything after it should pass through
      const positionals = runArgs._.map(String);
      const taskPath = positionals[0];

      if (!taskPath) {
        logError("Usage: rig run <path> [args...] or rig tasks");
        Deno.exit(1);
      }

      // Pass-through args: remaining positionals + anything after --
      const passArgs = [
        ...positionals.slice(1),
        ...(runArgs["--"] as string[] ?? []),
      ];

      const resolved = resolveTask(taskPath, config);
      await cmdTask(resolved, passArgs);
    } catch (err) {
      if (err instanceof Error) {
        logError(err.message);
      } else {
        throw err;
      }
      Deno.exit(1);
    }
    return;
  }

  // Check tmux is installed (only needed for service commands, not run)
  if (!(await checkTmux())) {
    printTmuxInstallGuide();
    Deno.exit(1);
  }

  // Parse args with @std/cli
  const args = parseArgs(Deno.args, {
    boolean: ["d", "f", "full", "help", "h", "V", "version", "raw", "json", "verbose", "v", "prev"],
    string: ["g", "group"],
    collect: ["g", "group"],
    alias: { f: "full", h: "help", V: "version", v: "verbose", g: "group" },
  });

  // Set global verbose flag
  VERBOSE = args.verbose;

  const [command, ...servicesRaw] = args._.map(String);
  const services = servicesRaw.flatMap((s) => s.split(",").map((n) => n.trim()).filter((n) => n.length > 0));

  // Collect -g/--group flags into array
  const groupFlags: string[] = (args.group as string[] ?? []).filter((g) => g.length > 0);

  if (args.version || command === "version") {
    print(VERSION);
    Deno.exit(0);
  }

  if (!command || args.help || command === "help") {
    printUsage();
    Deno.exit(0);
  }

  // Commands that need config
  if (["start", "up", "stop", "down", "kill", "restart", "ps", "list", "top", "config", "logs", "tail"].includes(command)) {
    try {
      const { config, configDir } = await loadConfig();
      const lookup = buildServiceLookup(config);
      const allServices = getAllServices(config);

      // Resolve targets based on -g flags or service names
      const targets = resolveTargets(config, lookup, services, groupFlags);
      const managers = createManagers(targets, configDir);

      switch (command) {
        case "start":
        case "up":
          await cmdStart(managers, targets, allServices, args.d);
          break;
        case "stop":
        case "down":
          await cmdStop(managers, targets);
          break;
        case "kill":
          await cmdKill(managers, targets);
          break;
        case "restart":
          await cmdRestart(managers, targets);
          break;
        case "ps":
        case "list":
          await cmdPs(managers, targets, args.full);
          break;
        case "top":
          await cmdTop(managers, targets);
          break;
        case "config":
          cmdConfig(config, targets, allServices, args.raw, args.json);
          break;
        case "logs":
        case "tail":
          await cmdLogs(managers, targets, allServices, args.f, args.prev);
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
  } else {
    logError(`Unknown command: ${command}`);
    printUsage();
    Deno.exit(1);
  }
}

main();
