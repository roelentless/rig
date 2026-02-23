/**
 * Process management via tmux: SessionManager, process inspection,
 * log streaming, and pre-start helpers.
 */

import { c, log, logSystem, logVerbose, print, SERVICE_COLORS, stripControlCodes } from "./output.ts";
import type { Config, RequirementDef, ResolvedService, ServiceDef, WatchDef } from "./config.ts";
import { LOG_DIR } from "./config.ts";

// ============================================================================
// TYPES
// ============================================================================

export interface SessionStatus {
  name: string;
  running: boolean;
  pid?: number;
  exitCode?: number;
  created?: number;
}

export interface ProcessMetrics {
  memoryMB: number;
  cpuPercent: number;
  ports: number[];
  processCount: number;
}

// ============================================================================
// PROCESS INSPECTION
// ============================================================================

export async function getProcessTree(rootPid: number): Promise<number[]> {
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

export async function getProcessMetrics(rootPid: number): Promise<ProcessMetrics> {
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

export async function checkTmux(): Promise<boolean> {
  try {
    const cmd = new Deno.Command("which", { args: ["tmux"] });
    const { code } = await cmd.output();
    return code === 0;
  } catch {
    return false;
  }
}

export function printTmuxInstallGuide(): void {
  print(`
${c("red")}Error: tmux is not installed${c("reset")}

tmux is required to manage background processes.

Install via the rig installer or manually:

  Installer:     curl -fsSL https://raw.githubusercontent.com/roelentless/rig/develop/install.sh | sh
  macOS:         brew install tmux
  Ubuntu/Debian: sudo apt install tmux
  Fedora:        sudo dnf install tmux
  Arch:          sudo pacman -S tmux
`);
}

// ============================================================================
// COMMAND BUILDING HELPERS
// ============================================================================

/**
 * Quote a string for shell if it contains special characters.
 */
function shellQuote(s: string): string {
  // If string contains shell special chars, wrap in single quotes
  // and escape any single quotes within
  if (/[*?[\]{}$`"'\\!<>|;&() \t\n]/.test(s)) {
    return `'${s.replace(/'/g, "'\\''")}'`;
  }
  return s;
}

/**
 * Build watchexec command wrapper for a service with watch config.
 * Returns the wrapped command string.
 */
function buildWatchexecCommand(command: string, watch: WatchDef, workingDir: string): string {
  const args: string[] = [];

  // -w for each path (default to working_dir if no paths specified)
  const paths = watch.paths?.length ? watch.paths : [workingDir];
  for (const p of paths) {
    args.push("-w", shellQuote(p));
  }

  // -e extensions (comma-separated)
  if (watch.extensions?.length) {
    args.push("-e", watch.extensions.join(","));
  }

  // --filter patterns (need quoting to prevent shell glob expansion)
  if (watch.patterns?.length) {
    for (const pattern of watch.patterns) {
      args.push("--filter", shellQuote(pattern));
    }
  }

  // --ignore patterns (need quoting to prevent shell glob expansion)
  if (watch.ignore?.length) {
    for (const pattern of watch.ignore) {
      args.push("--ignore", shellQuote(pattern));
    }
  }

  // --debounce
  if (watch.debounce) {
    args.push("--debounce", watch.debounce);
  }

  // Always use --restart to kill and restart on changes
  args.push("--restart");

  // Add command separator and the actual command
  args.push("--", command);

  return `watchexec ${args.join(" ")}`;
}

function buildEnvString(env: Record<string, string>): string {
  return Object.entries(env)
    .map(([k, v]) => `${k}=${JSON.stringify(v)}`)
    .join(" ");
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
// PRE-START CHECKS
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

// Track which check commands have already been remediated in this invocation
const remediatedChecks = new Set<string>();

/**
 * Run pre-start requirement checks for a service.
 * Each requirement has a `check` command (must exit 0) and a `command` (remediation).
 * If the check fails and hasn't been remediated yet, run the remediation command.
 * If remediation fails, throw to abort service start.
 */
async function checkRequirements(
  service: string,
  requirements: RequirementDef[],
  working_dir: string,
  environment?: Record<string, string>,
): Promise<void> {
  const env = environment ? { ...Deno.env.toObject(), ...environment } : undefined;

  for (const req of requirements) {
    // Run the check command
    const check = new Deno.Command("sh", {
      args: ["-c", req.check],
      cwd: working_dir,
      env,
      stdout: "null",
      stderr: "null",
    });
    const checkResult = await check.output();

    if (checkResult.code === 0) {
      continue; // Requirement already met
    }

    // Check failed - if already remediated, re-run check only
    if (remediatedChecks.has(req.check)) {
      // Already ran remediation for this check in another service
      // Re-check in case it now passes
      const recheck = new Deno.Command("sh", {
        args: ["-c", req.check],
        cwd: working_dir,
        env,
        stdout: "null",
        stderr: "null",
      });
      const recheckResult = await recheck.output();
      if (recheckResult.code === 0) {
        continue;
      }
      throw new Error(`Requirement check failed for ${service}: '${req.check}' (already remediated, still failing)`);
    }

    // Run remediation
    logSystem(`${service}: requirement '${req.check}' not met, running '${req.command}'`);
    const remediate = new Deno.Command("sh", {
      args: ["-c", req.command],
      cwd: working_dir,
      env,
      stdout: "inherit",
      stderr: "inherit",
    });
    const remediateResult = await remediate.output();

    if (remediateResult.code !== 0) {
      throw new Error(`Requirement remediation failed for ${service}: '${req.command}' exited with code ${remediateResult.code}`);
    }

    remediatedChecks.add(req.check);
  }
}

/**
 * Check if watchexec is installed, exit with helpful message if not.
 */
async function requireWatchexec(): Promise<void> {
  if (await commandExists("watchexec")) {
    return;
  }

  print(`
${c("red")}Error: watchexec is not installed${c("reset")}

watchexec is required for file watching (auto-restart on changes).

Install via the rig installer or manually:

  Installer:     curl -fsSL https://raw.githubusercontent.com/roelentless/rig/develop/install.sh | sh
  macOS:         brew install watchexec
  Arch:          sudo pacman -S watchexec
  Other Linux:   https://github.com/watchexec/watchexec/releases
`);
  Deno.exit(1);
}

// ============================================================================
// SESSION MANAGER
// ============================================================================

export class SessionManager {
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

    // Check requirements before starting
    if (def.requirements?.length) {
      await checkRequirements(service, def.requirements, def.working_dir, def.environment);
    }

    // Check for watchexec if watch is configured
    if (def.watch) {
      await requireWatchexec();
    }

    // Build command - wrap with watchexec if watch is configured
    let finalCommand = def.command;
    if (def.watch) {
      finalCommand = buildWatchexecCommand(def.command, def.watch, def.working_dir);
    }

    // Build command with environment vars
    const envStr = def.environment ? buildEnvString(def.environment) + " " : "";
    const cmd = `${envStr}exec ${finalCommand}`;

    logVerbose(`command=${finalCommand}`);
    logVerbose(`working_dir=${def.working_dir}`);

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
export function streamLogs(
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
// FACTORY HELPERS
// ============================================================================

/**
 * Create SessionManagers for all groups that have services in the target list.
 */
export function createManagers(targets: ResolvedService[], configDir: string): Map<string, SessionManager> {
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
export function createAllManagers(config: Config, configDir: string): Map<string, SessionManager> {
  const managers = new Map<string, SessionManager>();
  for (const groupName of Object.keys(config.groups)) {
    managers.set(groupName, new SessionManager(groupName, configDir));
  }
  return managers;
}

// ============================================================================
// SERVICE COLOR
// ============================================================================

export function getServiceColor(allServices: ResolvedService[], serviceName: string): string {
  // Allow config override, otherwise use deterministic color by index
  const idx = allServices.findIndex((s) => s.name === serviceName);
  if (idx === -1) return SERVICE_COLORS[0];

  const def = allServices[idx].def;
  if (def?.color) return def.color;

  return SERVICE_COLORS[idx % SERVICE_COLORS.length];
}
