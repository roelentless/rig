# Contributing to rig

Thank you for your interest in contributing to rig!

## Local Development Setup

### Prerequisites

- [Rust](https://www.rust-lang.org/tools/install) toolchain (stable)
- [tmux](https://github.com/tmux/tmux) installed
- A terminal that supports tmux

### Setting Up Local Development

1. **Clone the repository:**
   ```sh
   git clone <repository-url>
   cd rig
   ```

2. **Build:**
   ```sh
   cargo build
   ```

3. **Install locally:**
   ```sh
   cargo install --path .
   ```

   Or use the dev wrapper script (builds and runs from source):
   ```sh
   ./rig-dev ps
   ```

   To make the dev wrapper available system-wide:
   ```sh
   ln -sf "$(pwd)/rig-dev" ~/.local/bin/rig
   ```

4. **Test your changes:**
   ```sh
   rig -h
   rig up
   ```

## Running Tests

See [AGENTS.md](AGENTS.md) for detailed testing guidelines. Quick summary:

```sh
# Run all tests (single-threaded for tmux safety)
cargo test -- --test-threads=1

# Full cross-platform validation via Docker
docker build -f test/Dockerfile -t rig-test . && docker run --rm rig-test
```

## Code Quality

```sh
# Lint
cargo clippy -- -W clippy::all

# Format
cargo fmt --check    # check
cargo fmt            # fix
```

## Code Style

- Follow the existing code style in `src/`
- Keep functions focused and single-purpose
- See [AGENTS.md](AGENTS.md) for architecture and coding guidelines

## Submitting Changes

1. Make your changes
2. Run tests locally and via Docker
3. Ensure README.md is updated if commands change (run `rig -h` and sync output)
4. Run `cargo clippy` and `cargo fmt`
5. Submit a pull request with a clear description of changes
