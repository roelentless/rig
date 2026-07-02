<div align="center">
  <img src="https://raw.githubusercontent.com/roelentless/rig/develop/assets/rig.png" width="600"/>
</div>

# rig

A lightweight compose-like dev workflow tool for services and tasks.

<div align="center">
  <img src="https://raw.githubusercontent.com/roelentless/rig/develop/assets/cli.gif" width="600"/>
</div>

## What it does

**Services** - long-running processes via tmux:
```bash
rig up        # Start all services, stream logs (Ctrl+C stops all)
rig up -d     # Start in background (detached)
rig down      # Stop all services (graceful)
rig ps        # Show status
rig logs -f   # Follow logs
```

**Tasks** - one-off commands:
```bash
rig tasks                           # List all tasks
rig run backend.build               # Run a group-level task
rig run backend.api.test            # Run a service-level task
rig run backend.api.test -- --ci    # Pass args to task via --
rig run api.test web.test db.test   # Run multiple tasks sequentially
rig run api.test web.test -p        # Run tasks in parallel
```

Services run in tmux sessions - they survive terminal close and can be reattached.
Tasks execute directly - they pass through stdin/stdout and exit codes.

### Why rig?

Smoother dev workflow when working with many services, apps, and commands - without having to delegate everything to docker.

- **Zero-config Makefiles** - a folder with a Makefile just works, no rig.yaml needed
- **Simple config** - one yaml, or a folder tree that composes automatically
- **No state, no runtime** - tmux is the source of truth, no daemon running
- **Survives terminal close** - tmux keeps services running, come back anytime
- **Optional file watching** - services auto-restart when code changes
- **Supports monorepo workflows** - nested folders and Makefiles compose into one group tree
- **Runs anything** - make, npm, deno, cargo, docker, scripts - doesn't matter
- **Greppable file logs** - logs persist to disk, easy to search for you or agents
- **Quick inspection** - see resource usage and ports at a glance
- **Fast project switching** - spin up/down entire setups when switching between projects
- **Deploy-aligned env vars** - in config or external files, no dotenv in code, same pattern as production

## Install

**Install or upgrade** (macOS, Linux):

```bash
curl -fsSL https://raw.githubusercontent.com/roelentless/rig/develop/install.sh | sh
```

Downloads a prebuilt binary from GitHub Releases. Checks prerequisites, shows the plan, asks before running. Safe to re-run for upgrades.

The installer will offer to install [watchexec](https://github.com/watchexec/watchexec) (optional, for file watching). To include it non-interactively:

```bash
curl -fsSL https://raw.githubusercontent.com/roelentless/rig/develop/install.sh | sh -s -- --with-watchexec
```

### Manual install

**Prerequisites:** tmux (required), watchexec (optional, for file watching)

```bash
# macOS
brew install tmux watchexec

# Linux (Debian/Ubuntu)
sudo apt install tmux

# Linux (Fedora)
sudo dnf install tmux

# Linux (Arch)
sudo pacman -S tmux

# watchexec (all Linux distros, optional)
curl https://webi.sh/watchexec | sh
```

**Download the binary** from [GitHub Releases](https://github.com/roelentless/rig/releases) and place it in your PATH:

```bash
# Example for Linux amd64:
curl -fsSL https://github.com/roelentless/rig/releases/latest/download/rig-linux-amd64.tar.gz | tar xz
mv rig ~/.local/bin/

# Example for macOS arm64:
curl -fsSL https://github.com/roelentless/rig/releases/latest/download/rig-macos-arm64.tar.gz | tar xz
mv rig /usr/local/bin/
```

## Configuration

**No config needed for Makefiles.** Drop rig into any folder with a `Makefile` and
`rig tasks` / `rig run <target>` work immediately — see [Makefile support](#makefile-support).

For services (and richer tasks), add a `rig.yaml`, `rig.yml`, or `*.rig.yaml`. Top-level
`tasks`, `services`, `environment`, and `env_file` need no wrapper — they attach to the
root (the current directory), so their names are bare. Use `groups:` to namespace and
compose:

```yaml
groups:
  backend:
    services:
      db:
        command: docker run --rm -p 5432:5432 postgres:16
        working_dir: .
        healthcheck:
          grace_ms: 1000

      api:
        command: deno run -A server.ts
        working_dir: ./backend
        environment:
          PORT: 3000
        depends_on: [db]
        tasks:                           # Service-level tasks
          test:
            command: deno test -A
          build:
            command: deno compile -A -o dist/api server.ts
            description: Compile to binary

    tasks:                               # Group-level tasks
      deploy:
        command: ./scripts/deploy.sh
        # working_dir: .               # Optional if group has working_dir
        description: Deploy backend

  frontend:
    services:
      web:
        command: npm run dev
        working_dir: ./frontend
```

See [example/](example/) for a working config with all features, and [example/cli-log.md](example/cli-log.md) for real command output.

## Commands

```
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
  tasks [--group <name>]           List all tasks (→ marks a Makefile default goal)
  run/task <task...> [-- args...]  Run task(s): name, group.name, or group.service.name
    -p, --parallel                 Run tasks in parallel
    -l, --list                     List tasks instead of running them

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
  rig start api,worker      Same, comma-separated
  rig start backend.api     Start by dotted path (bare names must be unique)
  rig start -g backend      Start all services in backend group
  rig down                  Stop all processes (graceful)
  rig stop -g backend       Stop all services in backend group
  rig kill                  Force kill all processes
  rig restart -g backend    Restart entire group
  rig ps                    Show status
  rig logs -f               Follow all logs
  rig logs --prev api       Show previous logs for api
  rig tasks                 List all tasks
  rig run build             Run a task by name (Makefile target or rig task)
  rig run backend.deploy    Run a namespaced task
  rig run backend.api.test -- --coverage  Pass args to task
  rig run api.test web.test Run multiple tasks sequentially
  rig run api.test web.test -p  Run tasks in parallel
  rig config --json         Show raw JSON config

CONFIG:
  Zero-config: a folder with a Makefile just works — `rig tasks` lists its
  targets and `rig run <target>` runs them via make. Root Makefile targets are
  bare names, ./sub/Makefile targets become sub.<target>, nested folders dot
  deeper. Each Makefile's default goal is marked with → in listings.

  For services (and richer tasks) add rig.yaml, rig.yml, or *.rig.yaml. Config
  is a folder-aware group tree: rig searches upward for the nearest directory
  holding a rig file or Makefile (the project root) and builds the tree downward
  from there (gitignore-aware). Multiple rig files in one directory compose at
  the same level. Top-level tasks/services/environment/env_file need no
  wrapper (root = CWD → bare names). Any subfolder holding a Makefile or rig
  file auto-becomes a child group named by its folder. Authored `groups:`
  reshape the tree: a group is backed by `dir:` (adopt/rename a folder with its
  Makefile and/or rig file), `paths:` (explicit rig files), and/or inline
  units and child groups. `environment` and `env_file` cascade ancestor-wins —
  a higher group wraps a project and injects env from above; env files load at
  run/start, not at list time.

```

## Config reference

### Services

```yaml
groups:
  backend:                         # Group name (alphanumeric, hyphens, underscores)
    services:
      api:
        command: deno run -A app.ts  # Required. Command to run
        working_dir: ./backend       # Required. Working directory
        environment:                 # Optional. Environment variables
          PORT: 3000
          DEBUG: true
        env_file: ./api.env          # Optional. Load env from file
        color: cyan                  # Optional. Log color
        depends_on: [db, cache]      # Optional. Start after these services
        healthcheck:                 # Optional. Health check settings
          grace_ms: 500              # Wait before starting dependents
        requirements:                # Optional. Pre-start checks
          - check: pg_isready        # Command to test (exit 0 = met)
            command: docker start pg # Remediation if check fails
        tasks:                       # Optional. Service-level tasks
          test:
            command: deno test -A
```

Service names may repeat across groups — a service's full name is its dotted path
(`backend.api`). Bare names work whenever they are unambiguous; if two groups define
the same service name, rig errors and asks for the qualified path.

### Tasks

Tasks can be defined at the group level, service level, or sourced from a Makefile:

```yaml
groups:
  backend:
    working_dir: ./backend           # Optional. Default working_dir for all group tasks

    services:
      api:
        # ... service config ...
        tasks:                       # Service-level: inherits service env/working_dir
          test:
            command: deno test -A
          build:
            command: deno compile -A -o dist/api server.ts
            environment:             # Optional: overrides/extends service env
              NODE_ENV: production
            description: Compile API

    tasks:                           # Group-level: standalone
      deploy:
        command: ./scripts/deploy.sh
        # working_dir omitted — inherits from group's working_dir
        description: Deploy backend
```

- **Service tasks** inherit `working_dir` and `environment` from their parent service
- **Group tasks** inherit `working_dir` from the group if not set explicitly
- Tasks execute directly (not via tmux) - stdin/stdout pass through
- Exit codes propagate - `rig run backend.build && rig run backend.deploy`
- Cancelling (Ctrl+C / SIGTERM) forwards the signal to the task's process tree first —
  `make` gets to delete partial targets — then force-kills anything that remains

### Makefile support

Makefiles are first-class. Any folder in the tree that contains a standard `Makefile`
(`Makefile`, `makefile`, or `GNUmakefile`) contributes its targets as rig tasks — with or
without a `rig.yaml`.

**Zero-config.** In a folder with just a `Makefile`:

```bash
rig tasks              # lists the Makefile's targets
rig run build          # runs: make build
rig run test -- -v     # runs: make test -v   (args pass through after --)
```

**Folder namespacing.** Targets are named by the folder they live in, relative to where you
run `rig`:

- `./Makefile` targets are bare: `build`, `test`
- `./sub/Makefile` targets become `sub.<target>`: `sub.lint`
- deeper folders dot further: `sub/api.build`

Each Makefile's **default goal** (its `.DEFAULT_GOAL`, else its first target) is marked with
`→` in `rig tasks`.

**Which targets are exposed.** Every real rule target. Excluded: pattern rules (`%.o`),
special targets (anything starting with `.`, e.g. `.PHONY`), variable-expanded targets
(`$(GEN)`), and assignments. `include` / `-include` directives are followed and their
targets merged in file order.

**Descriptions** come from an inline `## doc` comment on the target's rule line:

```makefile
build: ## Compile for the current platform
	go build -o bin/api .
```

**Adopting a Makefile into a named group.** A `dir:` group backs itself with a folder and
adopts that folder's `Makefile` (and `rig.yaml`, if present). This also renames the folder:

```yaml
groups:
  relay:
    dir: ./lcm-relay      # ./lcm-relay/Makefile targets → relay.<target>
```

**Rig tasks override Makefile targets.** If a `rig.yaml` task and a Makefile target share a
name in the same group, the rig task wins.

**Make targets are never services.** A long-lived target (a dev server, a watcher) stays a
plain task until you define it as a service in a sibling rig file — that's what puts rig's
runtime around it: tmux lifecycle, logs, env layering, working dir:

```yaml
# rig.yaml next to the Makefile
services:
  dev-server:
    command: make serve
    working_dir: .
    environment:
      PORT: 4000
```

For a non-standard Makefile name (e.g. `ci.mk`), `include` it from a standard `Makefile` —
rig follows includes and picks up its targets.

### env_file

Load environment variables from external files. Supports string or array format:

```yaml
# Simple form
env_file: ./app.env

# Array form with required flag
env_file:
  - path: ./default.env
    required: true   # default - error if missing
  - path: ./override.env
    required: false  # skip if missing
```

Files are processed in order. Later files override earlier. Inline `environment` values override `env_file` values.

The `env_file` path is relative to the config file location. Values inside the env file that contain relative paths will resolve relative to `working_dir` at runtime (where the service executes).

Great for keeping development `.env` files outside of your repo - reduces secret sharing with LLM agents in common workflows.

Available colors: cyan, yellow, magenta, green, blue, orange, red, lavender, pink, teal, lime, coral, sky, gold, violet

### Watch (auto-restart)

Services can automatically restart when files change using [watchexec](https://github.com/watchexec/watchexec):

```yaml
groups:
  docs:
    services:
      mkdocs:
        command: mkdocs serve
        working_dir: ./docs
        watch:
          paths: ['./docs', './mkdocs.yml']  # Directories to watch (relative to working_dir)
          extensions: [md, yml, yaml]        # File extensions to watch
          patterns: ['**/*.md']              # Include glob patterns
          ignore: ['**/site/**']             # Exclude patterns
          debounce: 500ms                    # Wait before restarting
```

All options map directly to watchexec flags - no remapping or rig-specific defaults. If `paths` is omitted, watchexec watches the service's `working_dir` by default.

| Option | Type | watchexec flag | Description |
|--------|------|----------------|-------------|
| `paths` | string[] | `-w` | Directories/files to watch |
| `extensions` | string[] | `-e` | File extensions (e.g., `ts`, `tsx`) |
| `patterns` | string[] | `--filter` | Include glob patterns |
| `ignore` | string[] | `--ignore` | Exclude glob patterns |
| `debounce` | string | `--debounce` | Debounce duration (e.g., `500ms`) |

### Requirements (pre-start checks)

Services can declare prerequisites that are checked (and optionally remediated) before starting:

```yaml
groups:
  backend:
    services:
      api:
        command: deno run -A server.ts
        working_dir: .
        requirements:
          - check: test -S /var/run/docker.sock
            command: open -a Docker && sleep 10
          - check: pg_isready -h localhost
            command: docker start postgres
```

Each requirement has a `check` command and a remediation `command`:
- **check**: Runs first. If exit code is 0, the requirement is met — skip to next
- **command**: Runs if check fails. If remediation also fails, the service start is aborted

Requirements are evaluated in order before the service starts. The same check command is only remediated once per `rig` invocation, even if multiple services share the same requirement. Requirements do not apply to tasks.

### Multi-file and folder composition

rig builds a single **group tree**. It searches upward for the nearest directory
holding a rig file or Makefile (the project root) and builds the tree downward
from there (gitignore-aware), so you can run rig from any subdirectory. There is
nothing to import — folders compose automatically, and multiple rig files in one
directory compose at the same level:

- Any subfolder holding a `Makefile` or a rig file (`rig.yaml`, `rig.yml`, `*.rig.yaml`)
  auto-becomes a child group named by its folder.
- Nested folders nest as dotted groups. Folder names containing a dot (`app.v2`) can't
  be groups — rig skips them with a warning; adopt one under a clean name via `dir:`.
- The root file's top-level `tasks`/`services` attach to the root, so their names are bare.
- Conflicts are errors, not silent picks: an authored group named like a config-bearing
  folder (adopt it with `dir:` or rename), or the same task/service defined twice on one
  path. The only override is a rig task replacing a same-named Makefile target.

Reshape or extend the tree with authored `groups:`:

```yaml
# rig.yaml at the repo root
environment:
  REGION: us-east-1         # cascades to every group below (ancestor-wins)

services:
  gateway:
    command: ./gateway
    working_dir: .

groups:
  relay:
    dir: ./lcm-relay         # adopt + rename a folder (its Makefile and/or rig file)
    environment:
      RELAY_MODE: primary

  infra:
    paths:                   # pull explicit files into this group
      - ./infra/db.rig.yaml
      - ./infra/cache.rig.yaml
```

- **`dir:`** backs a group with a directory — discovered like the root (Makefile and/or
  rig file) — and renames it to the group name.
- **`paths:`** pulls explicit rig files into a group, wherever they live.
- **inline** `tasks` / `services` / child `groups:` layer on top of whatever `dir:` / `paths:`
  brought in.

Composition is folders plus `dir:` / `paths:`. rig finds the project root by searching
upward from the current directory to the nearest ancestor holding a rig file or Makefile,
then discovers the tree downward from there — so `rig` works from any subdirectory.

**Environment cascade (ancestor-wins).** `environment` and `env_file` set on a group apply
to everything beneath it, and a higher (nearer-root) group overrides a lower one. This lets
you wrap a vendored project and inject env from above without editing it. Env files are read
at run/start time, not when listing tasks.

See [example/](example/) for a working multi-folder setup.

## Log files

Rig stores logs in `.rig/logs/{group}/{service}/`:

```
.rig/
  logs/
    backend/
      db/
        current.log    # Current run
        previous.log   # Previous run (rotated on restart)
      api/
        current.log
        previous.log
    frontend/
      web/
        current.log
        previous.log
```

Logs persist after processes stop - useful for debugging crashes. The `--prev` flag shows logs from the last run before the current one.

Rig automatically adds `.rig/` to your `.gitignore`.

## Uninstall

```bash
# Check the path first, then remove
which rig        # e.g. /usr/local/bin/rig
rm /usr/local/bin/rig
```

## How it works

tmux is the source of truth for services - no state files.

- Start: `tmux new-session -d -s {group}-{name} -c {working_dir} '{command}'`
- Logs: `tmux pipe-pane` streams output to `.rig/logs/` files
- Stop: `tmux kill-session -t {group}-{name}` (sends SIGHUP)
- Kill: `SIGKILL` to process tree, then cleanup tmux session
- Status: `tmux list-sessions` filtered by group prefix

Tasks execute directly via `sh -c` with inherited stdin/stdout/stderr.

## Comparison with other tools

rig is not a build tool or task runner replacement. It's a dev workflow orchestrator.

| Tool | Focus | rig's approach |
|------|-------|----------------|
| **Make** | Build dependency graphs | rig layers on top — Makefile targets become rig tasks automatically |
| **npm/deno scripts/tasks** | Package-level tasks | rig spans multiple packages, manages long-running services |
| **docker-compose** | Container orchestration | rig runs native processes via tmux, no containers required |

**What rig doesn't do:**
- Incremental builds or caching
- Container management
- CI/CD pipelines

**What rig does well:**
- Start your dev stack with one command
- Keep services running across terminal sessions
- Organize tasks for any build system in one place

## License

AGPL-3.0 - See LICENSE file.

---

**Alpha Software** - This project is under active development. APIs and configuration formats may change between versions.

Note: built to improve my personal workflow during development of [halebase.com](https://halebase.com) — don't take it too seriously.

Generated with some LLM assistance.
