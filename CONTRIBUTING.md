# Contributing to rig

Thank you for your interest in contributing to rig!

## Local Development Setup

### Prerequisites

- [Deno](https://deno.land/) installed
- [tmux](https://github.com/tmux/tmux) installed
- A terminal that supports tmux

### Setting Up Local Development

1. **Clone the repository:**
   ```sh
   git clone <repository-url>
   cd rig
   ```

2. **Install locally using deno install:**
   Install your local development copy as the global `rig` command:
   ```sh
   deno install -Agf -n rig rig.ts
   ```
   
   This creates a wrapper script at `~/.deno/bin/rig` that points to your local `rig.ts` file. The `-f` flag forces reinstall, so you can run this again whenever you want to update the link.
   
   **Note:** Ensure `~/.deno/bin` is in your PATH (it should be by default with Deno).

3. **Test your changes:**
   Now you can use `rig` directly and it will run your local development version:
   ```sh
   rig -h
   rig up
   ```

4. **Alternative: Run directly (without installing):**
   You can also test without installing by running rig.ts directly:
   ```sh
   deno run -A rig.ts -h
   deno run -A rig.ts up
   ```

## Running Tests

See [AGENTS.md](AGENTS.md) for detailed testing guidelines. Quick summary:

```sh
# Fast Mac-only tests (for quick iteration)
deno task test:darwin

# Full cross-platform validation (required before PR)
deno task test:darwin  # Mac validation
deno task test:linux   # Docker-based Linux validation
```

## Code Style

- Follow the existing code style in `rig.ts`
- Keep functions focused and single-purpose
- See [AGENTS.md](AGENTS.md) for architecture and coding guidelines

## Submitting Changes

1. Make your changes
2. Run tests on both platforms (see Testing section)
3. Ensure README.md is updated if commands change (run `rig -h` and sync output)
4. Submit a pull request with a clear description of changes
