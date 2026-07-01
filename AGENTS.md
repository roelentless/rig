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

- Session naming convention `{group}-{name}` enables discovery via prefix filtering; dotted group paths are flattened with `-` — tmux mangles `.` to `_` in session names and parses `.` in `-t` targets as window.pane, so a dotted session name can be created but never found again
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

### Task cancellation

- `rig run`/`task` supervises the task subprocess: on SIGINT/SIGTERM it delegates cancellation to the task provider (`TaskProvider::cancel`) before exiting 130/143. `RigProvider` forwards the received signal to the whole descendant tree (sysinfo snapshot, graceful — lets `make` run its delete-partial-target cleanup), polls with re-snapshots until the tree drains (5s cap), then SIGKILLs stragglers — so a cancelled `make` run leaves no orphaned recipe. Task-run path only; tmux-managed services stop via `kill-session`, so this must not touch the service path

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

### Group Tree

- The root is the nearest ancestor of CWD (incl. CWD) directly holding a rig file or Makefile — found by an **upward** search (direct reads, gitignore-independent), so rig run from any subdir finds the project root. From that root, discovery walks **downward** (or from a `dir:`'s directory), gitignore-aware (via `ignore` crate)
- All rig files directly in one directory (`rig.yaml`, `rig.yml`, `*.rig.yaml`) compose into that group at the same level; a duplicate task/service/child-group name across siblings is a hard error
- One tree: rig-authored units and discovered Makefile targets share the same `Group` nodes; a single tree-backed provider carries both
- Path expansion is per-file: each rig file's `working_dir`/`env_file` resolve relative to its own directory before folding into the tree
- Folder-auto child groups: a subdir holding a Makefile or a rig file becomes a group named by its folder. Dotted folder names (`app.v2`) are not supported as groups — the subtree is skipped with an always-on warning
- `dir:` adopts + renames a folder; `paths:` pulls explicit files; both mark their targets as adopted so folder-auto discovery doesn't add the same folder/file twice
- Conflicts are hard errors, never silent: an authored group named like an unadopted config-bearing folder, or a rig task/service defined twice on one fully-qualified path (dir file + inline, or two `paths:` files). Rig-over-make displacement is the only sanctioned override
- `environment`/`env_file` cascade ancestor-wins; env files load at run/start, never at list time
- A `visited` set of normalized dirs guards against `dir:` cycles

## Config Structure

Config is a folder-aware **group tree**. A group is backed by any of: a directory
(`dir:`, discovered — Makefile and/or rig file), explicit files (`paths:`), inline
`tasks`/`services`, and/or child `groups:`. Top-level `tasks`/`services`/`environment`/
`env_file` need no wrapper — they attach to the root (CWD → bare names).

```yaml
environment:
  REGION: us-east-1          # cascades ancestor-wins to every group below

services:
  gateway: { command: ./gateway, working_dir: . }

groups:
  relay:
    dir: ./lcm-relay         # adopt + rename a folder (its Makefile and/or rig file)
  infra:
    paths: [./infra/db.rig.yaml]
    services:
      cache: { command: ..., working_dir: ... }
```

Key rules:
- Names are folder-namespaced dotted paths: `relay.build`, `infra.cache`
- A subfolder with a Makefile or rig file auto-becomes a child group; `dir:`/`paths:` reshape or rename
- Service names may repeat across groups; a service's identity is its fully-qualified dotted path. Bare-name targeting resolves unique-or-error (like tasks); `depends_on` resolves same-group-first, then tree-wide unique-or-error
- Groups are targeted with `-g/--group` using the dotted path: `rig start -g relay`
- Default CLI targets are services: `rig start gateway`
- Tasks run via `rig run <name>`, `rig run group.task`, or `rig run group.service.task`; a short name works when unambiguous
- Rig-authored tasks win over Makefile targets on a name clash
- One SessionManager instance per group (tmux sessions: `{group}-{service}`, dotted group paths flattened with `-`)

Discovery, per-file path resolution, sibling composition, and folder-auto grouping are
detailed under Key Learnings → Group Tree above.

## Code Structure

```
src/
  main.rs       → Entry point: CLI parsing (clap derive) and command dispatch
  lib.rs        → Library root, re-exports modules
  output.rs     → Terminal output: colors, logging, display helpers
  config.rs     → Types, schema validation, group-tree loading/parsing/querying, env cascade
  providers/    → Task providers over the group tree
    mod.rs      → `TaskProvider` trait, `provider()` builder (one tree-backed provider)
    rig.rs      → Tree-backed provider: rig-authored + Makefile tasks
    makefile.rs → Makefile parsing (targets, docs, default goal, includes) + `make` commands
  process.rs    → SessionManager, process tree/metrics, tmux checks, log streaming
  commands.rs   → CLI command implementations (start/stop/ps/top/logs/tasks/run)

Module dependency graph:
  output ← config ← process ← commands ← main
                 ← providers ← main

install.sh:
  Platform detection     → OS, arch
  Dependency check       → tmux (required), watchexec (optional)
  Binary download        → GitHub Releases tarball
  PATH verification      → check install dir is in PATH
```

## Common Pitfalls

1. **Orphan processes**: Old processes from different systems won't be in tmux. Port-based cleanup was removed - tmux handles lifecycle properly now.

2. **Path resolution**: `working_dir` in config is relative to config file location, not CWD. Across the tree, each rig file expands its paths relative to its own location.

3. **Raw mode stdin**: Intercepts Ctrl+C. Must check for byte 3 explicitly.

4. **lsof on macOS**: `-p` flag doesn't filter with `-i`. Parse output and filter by PID.

5. **Test session cleanup**: When testing configs that span folders, sessions may be created in different groups. The `cleanupSessions()` helper must track all possible test group prefixes.

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
  e2e_makefile.rs     # Makefile target discovery + folder namespacing
  e2e_make_compat.rs  # Makefile runtime contract: exit codes, signals, env forwarding
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

### Makefile Support

Makefiles are first-class citizens of the group tree, not a bolted-on provider keyed off
`working_dir`. Any folder backing a group (the root, a folder-auto child, or a `dir:` group)
that contains a standard Makefile contributes its targets as tasks in that group's namespace.
Discovery is folded into the group tree during `try_load_config`; a single tree-backed
provider carries both rig-authored and make-sourced tasks.

- Standard names: `Makefile`, `makefile`, `GNUmakefile` (picked in that priority per dir).
- Target discovery: every real rule target. Excluded: `.`-prefixed specials (`.PHONY`), pattern rules (`%`), variable-expanded targets (`$`), and assignments. `include`/`-include` files are followed and merged in file order.
- Descriptions: inline `## doc` on the target's rule line.
- Default goal: `.DEFAULT_GOAL` if set, else the first target; carried on `ResolvedTask.default_goal` and marked `→` in listings.
- Commands: `make <target>` for a standard Makefile; `make -f <file> <target>` for a non-standard filename (reached via `include`).
- Rig-authored tasks win over Makefile targets on a name clash. Precedence keys off `TaskDef.source` (`Rig` vs `Make`), not provider order, since both live in one tree-backed provider.
- Non-standard standalone Makefiles are not auto-discovered — `include` them from a standard Makefile.
- The runtime contract (exit-code passthrough, env/`-- VAR=val` forwarding into recipes, stdout/stderr, working dir, cancellation) is pinned by `tests/e2e_make_compat.rs` + `tests/fixtures/make/`.

## Future Considerations

- Additional task providers: npm scripts, Justfile, etc. — feed the same group tree via the `TaskProvider` trait.
