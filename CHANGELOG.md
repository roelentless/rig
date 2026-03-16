# Changelog

## 2026-03-16

- feature: Makefile tasks — set group `working_dir` and Makefile targets become rig tasks
- improvement: Group `working_dir` — group tasks inherit it when not explicitly set
- improvement: `make install` — builds release binary and installs to `~/.local/bin`
- improvement: installer uses `~/.local/bin` on all platforms, no sudo required
- bugfix: PATH entry uses `~/.zshenv` — works in editors and non-interactive shells
- improvement: installer warns when `/usr/local/bin/rig` would shadow the install
- feature: short task names — `rig run deploy` works when the name is unambiguous
- feature: `rig run` with no arguments lists available tasks
