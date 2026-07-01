# Changelog

## 2026-07-02

- improvement: services resolve like tasks — `group.service` paths, ambiguity errors
- bugfix: services in nested groups get valid tmux session names
- improvement: cancelling `rig run` signals gracefully first, then force-kills
- improvement: authored group named like a config-bearing folder is an error
- improvement: task/service defined twice on one path is an error
- improvement: dotted folder names are skipped with a warning, not half-supported
- bugfix: `rig run` cancellation on Linux no longer orphans the make process tree
- bugfix: empty arguments after `--` are preserved
- bugfix: task listing no longer crashes on non-ASCII commands

## 2026-07-01

- bugfix: cancelling `rig run` no longer orphans the `make`/recipe process tree
- improvement: tasks default `working_dir` to their config's folder
- improvement: `rig config --json/--raw` shows the real group tree (rig + make)
- improvement: `-g/--group` scopes the whole subtree, not just one level
- improvement: `rig init` scaffolds the wrapper-free top-level format
- feature: multiple rig files in one folder compose at the same level
- feature: `rig` works from any subdirectory — searches upward to the project root
- feature: first-class Makefile support — zero-config, folder-namespaced tasks
- feature: Makefile default goal marked with `→` in `rig tasks`
- feature: group-tree config — subfolders auto-compose into namespaced groups
- improvement: compose with `dir:`/`paths:` and folders — `imports:` removed
- feature: `environment`/`env_file` cascade ancestor-wins down the group tree

## 2026-03-16

- feature: Makefile tasks — set group `working_dir` and Makefile targets become rig tasks
- improvement: Group `working_dir` — group tasks inherit it when not explicitly set
- improvement: `make install` — builds release binary and installs to `~/.local/bin`
- improvement: installer uses `~/.local/bin` on all platforms, no sudo required
- bugfix: PATH entry uses `~/.zshenv` — works in editors and non-interactive shells
- improvement: installer warns when `/usr/local/bin/rig` would shadow the install
- feature: short task names — `rig run deploy` works when the name is unambiguous
- feature: `rig run` with no arguments lists available tasks
