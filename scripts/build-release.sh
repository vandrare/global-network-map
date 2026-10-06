#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.."
[[ $(uname -m) == x86_64 ]] || { echo 'This release recipe targets x86_64.' >&2; exit 1; }
version=$(sed -n 's/^version = "\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
cargo build --release --locked --features gui --bins -j "${CARGO_BUILD_JOBS:-2}"
bundle="global-network-map-${version}-linux-x86_64"
staging=$(mktemp -d)
trap 'rm -rf "$staging"' EXIT
mkdir -p "dist" "$staging/$bundle/bin" "$staging/$bundle/assets"
for binary in global-network-map global-network-map-capture global-network-map-gui; do
  install -m 755 "target/release/$binary" "$staging/$bundle/bin/$binary"
done
install -m 755 install-gui.sh "$staging/$bundle/install.sh"
install -m 644 README.md LICENSE ATTRIBUTION.md graphical-preview.png "$staging/$bundle/"
install -m 644 assets/global-network-map.svg "$staging/$bundle/assets/"
tar -czf "dist/$bundle.tar.gz" -C "$staging" "$bundle"
printf 'Built dist/%s.tar.gz\n' "$bundle"
