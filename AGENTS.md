# Agent Guidelines for rig

**Compose Spec Alignment**: Where it makes sense, we align with the [Compose Specification](https://github.com/compose-spec/compose-spec/blob/main/spec.md) for naming and behavior of new features.

## Off-Limits Directories

- `rfc/` - Contains work tasks and planning documents. Do not read unless specifically asked.

## Core Philosophy

**Stateless over stateful**: tmux IS the state. Don't add state files. Query reality.

**Idempotent operations**: Every command should be safe to run multiple times. `stop` loops through all processes even if none are running. `start` checks if already running.

**Let processes be independent**: When one process dies, don't kill others. User decides lifecycle (like docker-compose).

## Key Learnings

### tmux

- Session naming convention `{group}-{name}` enables discovery via prefix filtering
- `remain-on-exit on` preserves crash output for debugging
- `kill-session` sends SIGHUP → process exits → port released
- In raw mode, Ctrl+C is byte 3, not SIGINT - must handle explicitly

### Process Metrics

- `lsof -p PID -i` doesn't filter properly on macOS with `-i` flag - must filter output by PID column
- Process trees need recursive `pgrep -P` to find all children
- Sum RSS across tree for accurate memory (child processes matter)

### Performance

- `rig ps` should be instant (~30ms) - don't collect metrics by default
- `rig ps -f` for full metrics (~800ms acceptable)
- `rig top` uses smart refresh: high CPU processes refresh more often

### CLI

- Uses clap with derive macros for argument parsing
- Custom help text (not clap's auto-generated help)

### Installer (`install.sh`)

- POSIX `sh` only, no bashisms — the script is piped via `curl | sh`
- `curl` is required and checked immediately — hard error if missing (curl must exist to fetch the installer anyway)
- Downloads prebuilt binary from GitHub Releases (`rig-{platform}-{arch}.tar.gz`)
- tmux is **required** (hard error if missing, no auto-install)
- watchexec is **optional** — installed via `curl https://webi.sh/watchexec | sh` (their own installer handles all platform/arch logic); macOS prefers `brew`, falls back to webi
- Do not use `.tar.xz` for anything — watchexec has no `.tar.gz` release so we delegate to webi instead of handling archives ourselves
- Transparency pattern: check deps → show plan → ask user → download binary
- Read user input when stdin is a pipe: redirect from `/dev/tty`
- Install location: `~/.local/bin` on Linux, `/usr/local/bin` on macOS
- Version check: skips download if already up to date
- The installer doubles as upgrader — safe to re-run
- Supported platforms: macOS (arm64, x86_64), Linux (amd64, arm64)

### Watch (auto-restart)

- Watch wraps service commands with watchexec for file-triggered restarts
- Paths in `watch.paths` resolve relative to `working_dir` (same as other path fields)
- Glob patterns need shell quoting (`shellQuote()`) to prevent shell expansion
- Full delegation to watchexec - no rig-specific defaults or remapping
- Check watchexec installed before starting service (`requireWatchexec()`)
- All runtime dependency errors (tmux, watchexec) point to the installer URL as first option

### Requirements (pre-start checks)

- Services can declare `requirements` — check/command pairs evaluated before start
- Each requirement runs a `check` command (exit 0 = met). If check fails, runs `command` (remediation)
- If remediation fails (non-zero exit), service start is aborted with an error
- Module-level `remediatedChecks` Set deduplicates remediation across services in one `rig` invocation
- Check commands run with `stdout/stderr: "null"` (only exit code matters)
- Remediation commands run with `stdout/stderr: "inherit"` (user sees output)
- Commands run in the service's `working_dir` with its `environment`
- Requirements do not apply to tasks

### Config Display

- Simple text output (`rig config`) skips complex nested fields (`tasks`, `watch`) to avoid `[object Object]` serialization
- JSON output (`rig config --json`) includes full details
- Use `skip` Set to control which fields appear in text output

### Multi-File Config

- Path normalization uses URL class: `new URL(path, "file:///").pathname` - cleaner than manual string manipulation
- Circular import detection needs normalized absolute paths to work correctly
- File discovery respects .gitignore (via `ignore` crate) to avoid pulling in rig files from dependencies
- Deduplication must happen by absolute normalized path, not relative path
- Path expansion must happen per-file before merging (each config's paths relative to its own location)
- `LoadContext` pattern works well: track `configPath`, `configDir`, `loaded` Set, and `importChain` for recursion

## Config Structure

Config uses multi-group format with services and tasks nested under groups:

```yaml
imports:
  - db/rig.yaml
  - backend/rig.yaml

groups:
  backend:
    services:
      api: { command: ..., working_dir: ... }
      db:  { command: ..., working_dir: ... }
    tasks:
      deploy: { command: ..., working_dir: ... }
  frontend:
    services:
      web: { command: ..., working_dir: ... }
```

Key rules:
- Service names must be unique across all groups (even across imported files)
- Groups can have services, tasks, or both
- Groups are targeted with `-g/--group` flag: `rig start -g backend`
- Default CLI targets are services: `rig start api db`
- Tasks are run via `rig run group.task` or `rig run group.service.task`
- One SessionManager instance per group (tmux sessions: `{group}-{service}`)

### Multi-File Config

Configs can import other configs to compose a service graph:

```yaml
imports:
  - shared/db/rig.yaml      # Relative path from this config's location
  - ../common/rig.yaml      # Parent directory traversal supported
  - infra.rig.yaml          # *.rig.yaml naming pattern supported
```

Key rules:
- `rig` searches upward from CWD to find the nearest config file
- Each file's paths (`working_dir`, `env_file`) are expanded relative to its own location
- Same file imported multiple times = loaded once (deduped by absolute path)
- Circular imports = error
- Duplicate group or service names = error
- `rig discover` scans for rig files and suggests imports

## Code Structure

```
src/
  main.rs       → Entry point: CLI parsing (clap derive) and command dispatch
  lib.rs        → Library root, re-exports modules
  output.rs     → Terminal output: colors, logging, display helpers
  config.rs     → Types, schema validation, config loading/parsing/querying
  process.rs    → SessionManager, process tree/metrics, tmux checks, log streaming
  commands.rs   → CLI command implementations (start/stop/ps/top/logs/tasks/discover)

Module dependency graph (strict DAG):
  output ← config ← process ← commands ← main

install.sh:
  Platform detection     → OS, arch
  Dependency check       → tmux (required), watchexec (optional)
  Binary download        → GitHub Releases tarball
  PATH verification      → check install dir is in PATH
```

## Common Pitfalls

1. **Orphan processes**: Old processes from different systems won't be in tmux. Port-based cleanup was removed - tmux handles lifecycle properly now.

2. **Path resolution**: `working_dir` in config is relative to config file location, not CWD. With multi-file imports, each config expands paths relative to its own location.

3. **Raw mode stdin**: Intercepts Ctrl+C. Must check for byte 3 explicitly.

4. **lsof on macOS**: `-p` flag doesn't filter with `-i`. Parse output and filter by PID.

5. **Test session cleanup**: When testing multi-file configs, sessions may be created in different groups. The `cleanupSessions()` helper must track all possible test group prefixes.

## Local Development

Developers should set up local development as described in [CONTRIBUTING.md](CONTRIBUTING.md).

Build and run from source:
```sh
cargo build                    # Build debug binary
cargo install --path .         # Install to ~/.cargo/bin
./rig-dev ps                   # Build + run in one step (dev wrapper)
```

**Before committing**, run `cargo fmt` to auto-format all source files. CI enforces `cargo fmt --check` and will reject unformatted code.

The `rig-dev` script builds from the source tree and runs the debug binary, so your working directory's `rig.yaml` is used while the binary comes from the source checkout.

## Changelog

`CHANGELOG.md` at project root.

**Granularity: one entry per feature, not per commit or internal refactor.** A new command, a new config key, a bugfix, an install improvement — those get entries. Internal restructuring that users can't see does not.

**Scope: user-visible impact only.** Describe what changed in experience, not how it was built. No internal identifiers, struct names, or implementation details.

**Style rules:**
- Max ~80 chars per entry
- Prefix with type: `feature:`, `bugfix:`, `improvement:`
- Date-based sections, no "Unreleased": `## YYYY-MM-DD`
- Latest date on top; latest entry on top within a date
- If today's section doesn't exist yet, add it above the previous one

```markdown
## 2026-03-16

- feature: short task names — `rig run deploy` works when the name is unambiguous
- bugfix: PATH entry uses `~/.zshenv` — works in editors and non-interactive shells
```

## Documentation

**Keep README.md in sync with command output**: When modifying commands or their output, always run `rig -h` and update the Commands section in README.md to match the exact output. The README should reflect what users see when they run the help command.

**Keep install.sh in sync with dependencies**: When adding new runtime dependencies, update `install.sh` (detection, plan display), the README install section, and the error messages in `src/main.rs` that guide users when a tool is missing.

## Testing

Tests are end-to-end Rust integration tests that spawn the rig binary and interact with tmux.

### Quick feedback loop

During development, test locally:

```sh
cargo test -- --test-threads=1
```

Tests must run single-threaded because they share tmux state.

### Validate all platforms after task completion

After completing any task, **always** validate on Linux via Docker:

```sh
cargo test -- --test-threads=1                                        # Local
docker build -f test/Dockerfile -t rig-test . && docker run --rm rig-test  # Docker
```

Both must pass before considering the task complete.

### Test structure

```
tests/
  common/mod.rs       # TestContext: temp dirs, rig binary, tmux helpers, cleanup
  e2e_help.rs         # Help/version output tests
  e2e_tasks.rs        # Task execution tests
  e2e_env.rs          # Environment variable tests
  e2e_deps.rs         # depends_on tests
  e2e_watch.rs        # File watching tests
  e2e_services.rs     # Service lifecycle tests
  e2e_multifile.rs    # Multi-file config tests
  e2e_requirements.rs # Pre-start requirement tests
  e2e_schema.rs       # Config validation tests
test/
  Dockerfile          # Linux test environment for Docker
```

Each test file uses `TestContext` from `common/mod.rs` which handles temp directory creation, writing test configs, running the rig binary, and tmux session cleanup.

### Makefile Provider

Groups support a `working_dir` field. When set, a `Makefile` in that directory is automatically discovered and its `.PHONY` targets become tasks under the group namespace.

- Target discovery: `.PHONY` targets only. Falls back to `## target: description` documented targets when no `.PHONY` is declared.
- Descriptions: extracted from `## target: description` comment lines (must be non-indented, at column 0).
- Commands: `make <target>` for standard `Makefile`; `make -f <filename> <target>` for non-standard filenames.
- Rigfile tasks always override Makefile targets on name collision (rigfile is authoritative).
- Explicit extra Makefiles via `makefiles:` list (deduplicated against auto-discovered).
- `working_dir` also serves as the default `working_dir` for group-level tasks that don't specify one.

```yaml
groups:
  backend:
    working_dir: ./backend   # auto-discovers ./backend/Makefile; tasks inherit this dir
    makefiles:
      - ./tools/ci.mk        # additional explicit Makefile
    services:
      api: { command: go run ., working_dir: ./backend }
    tasks:
      deploy: { command: ./scripts/deploy.sh }   # inherits working_dir from group
```

The provider model is designed for extension: `services`, `tasks`, and `makefiles` are named providers within a group. Future providers (e.g., `npm-scripts`, `justfile`) follow the same pattern.

## Future Considerations

- Import globs: `imports: ["services/*/rig.yaml"]`
- `rig discover --watch` for continuous import updates
- Additional task providers: npm scripts, Justfile, etc.
