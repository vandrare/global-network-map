#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
cargo build --release --locked
install -d "$HOME/.local/bin"
install -m 755 target/release/global-network-map "$HOME/.local/bin/global-network-map"
install -m 755 target/release/global-network-map-capture "$HOME/.local/bin/global-network-map-capture"
printf 'Installed global-network-map and its capture helper in %s/.local/bin\n' "$HOME"
printf 'Preview: global-network-map --demo\nDatabase: global-network-map --update-db\nLaunch: global-network-map\n'
