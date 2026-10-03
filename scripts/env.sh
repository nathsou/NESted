#!/usr/bin/env bash
# Cloud-workspace toolchain fallback; ordinary installations keep their own paths.
if [ -x /workspace/.cargo/bin/cargo ]; then
  export CARGO_HOME=/workspace/.cargo
  export RUSTUP_HOME=/workspace/.rustup
  export PATH="/workspace/.cargo/bin:$PATH"
fi
export npm_config_cache="${npm_config_cache:-/tmp/nested-npm-cache}"

