# Test rig installer in Debian
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    curl \
    git \
    unzip \
    ca-certificates \
    tmux \
    && rm -rf /var/lib/apt/lists/*

# Install deno
RUN curl -fsSL https://deno.land/install.sh | sh
ENV DENO_INSTALL="/root/.deno"
ENV PATH="${DENO_INSTALL}/bin:${PATH}"

# Copy local repo for testing (simulates clone to ~/.rig/repo)
COPY . /root/.rig/repo
WORKDIR /root/.rig/repo

# Install rig from local copy
RUN deno install -A -g -n rig --force rig.ts

# Verify installation
RUN rig version
RUN which rig

# Test rig init in a new directory
WORKDIR /test
RUN rig init && cat rig.yaml

# Test that help works
RUN rig --help

CMD ["echo", "All tests passed!"]
