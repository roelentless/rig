#!/usr/bin/env -S deno run -A

/**
 * rig - A lightweight process manager using tmux.
 * Inspired by docker-compose, but not aiming for compatibility.
 * No state files - tmux is the source of truth.
 */

// deno-lint-ignore no-import-prefix
import { parseArgs } from "jsr:@std/cli@^1.0.25/parse-args";

import { VERSION } from "./lib/version.ts";
import { logError, print, setVerbose } from "./lib/output.ts";
import { buildServiceLookup, ConfigError, getAllServices, loadConfig, resolveTargets, resolveTask } from "./lib/config.ts";
import { checkTmux, createManagers, printTmuxInstallGuide } from "./lib/process.ts";
import { cmdConfig, cmdDiscover, cmdInit, cmdKill, cmdLogs, cmdPs, cmdRestart, cmdStart, cmdStop, cmdTaskList, cmdTasks, cmdTop } from "./lib/commands.ts";

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
  tasks [--group <name>]           List all tasks
  run/task <task...> [-- args...]  Run task(s) (group.name or group.service.name)
    -p, --parallel                 Run tasks in parallel

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
  rig run backend.deploy    Run a task
  rig run backend.api.test -- --coverage  Pass args to task
  rig run api.test web.test Run multiple tasks sequentially
  rig run api.test web.test -p  Run tasks in parallel
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

    setVerbose(listArgs.verbose);

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
      if (err instanceof ConfigError) {
        logError(err.message);
        Deno.exit(1);
      }
      throw err;
    }
    return;
  }

  // Handle 'discover' - scan for rig files and update imports
  if (Deno.args[0] === "discover") {
    const discoverArgs = parseArgs(Deno.args.slice(1), {
      boolean: ["help", "h", "dry-run", "yes", "y", "verbose", "v"],
      alias: { h: "help", y: "yes", v: "verbose" },
    });

    setVerbose(discoverArgs.verbose);

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
    // Note: --parallel is parsed as boolean, --parallel=N handled manually
    const runArgs = parseArgs(Deno.args.slice(1), { // Skip "run"/"task"
      boolean: ["l", "list", "help", "h", "verbose", "v", "parallel", "p"],
      string: ["g", "group"],
      collect: ["g", "group"],
      alias: { l: "list", h: "help", v: "verbose", g: "group", p: "parallel" },
      "--": true, // Collect everything after -- in a separate array
    });

    setVerbose(runArgs.verbose);

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

      // All positionals are task paths
      const taskPaths = runArgs._.map(String);

      if (taskPaths.length === 0) {
        logError("Usage: rig run <task...> [-- args...] or rig tasks");
        Deno.exit(1);
      }

      // Args only allowed via -- (required for clarity)
      const passArgs = runArgs["--"] as string[] ?? [];

      // -p or --parallel runs all tasks in parallel
      const parallel = runArgs.parallel;

      // Resolve all tasks
      const resolved = taskPaths.map((p) => resolveTask(p, config));

      const code = await cmdTasks(resolved, passArgs, parallel);
      Deno.exit(code);
    } catch (err) {
      if (err instanceof ConfigError) {
        logError(err.message);
        Deno.exit(1);
      }
      throw err;
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
  setVerbose(args.verbose);

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
      if (err instanceof ConfigError) {
        logError(err.message);
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
