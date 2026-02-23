# Dockerfile for running rig tests on Linux
FROM rust:bookworm

# Install tmux, procps (for ps, pgrep), lsof, fd-find (for discover), and watchexec (for watch)
RUN apt-get update && apt-get install -y \
    tmux \
    procps \
    lsof \
    fd-find \
    curl \
    && rm -rf /var/lib/apt/lists/* \
    && ln -s /usr/bin/fdfind /usr/bin/fd \
    && ARCH=$(dpkg --print-architecture) \
    && if [ "$ARCH" = "amd64" ]; then WATCHEXEC_ARCH="x86_64"; else WATCHEXEC_ARCH="aarch64"; fi \
    && curl -fsSL -o /tmp/watchexec.deb "https://github.com/watchexec/watchexec/releases/download/v2.2.1/watchexec-2.2.1-${WATCHEXEC_ARCH}-unknown-linux-gnu.deb" \
    && dpkg -i /tmp/watchexec.deb \
    && rm /tmp/watchexec.deb

WORKDIR /app

# Copy manifests first for dependency caching
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs && cargo build --tests 2>/dev/null || true && rm -rf src

# Copy real source
COPY src/ ./src/
COPY tests/ ./tests/

# Build
RUN cargo build --tests

# Run tests
CMD ["cargo", "test", "--", "--test-threads=1"]
