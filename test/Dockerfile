# Dockerfile for running rig tests on Linux
FROM denoland/deno:debian

# Install tmux, procps (for ps, pgrep), fd-find (for discover), and watchexec (for watch)
# watchexec: download correct architecture DEB from GitHub releases
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

# Copy source files (skip lockfile for cross-version compatibility)
COPY rig.ts version.ts deno.json ./
COPY test/ ./test/

# Cache dependencies
RUN deno cache rig.ts test/rig.test.ts

# Run tests
CMD ["deno", "test", "-A", "test/rig.test.ts"]
