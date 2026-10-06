#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "${BASH_SOURCE[0]}")"
if [[ -d bin ]]; then
  binary_dir=bin
else
  cargo build --release --locked --features gui --bins -j "${CARGO_BUILD_JOBS:-2}"
  binary_dir=target/release
fi
install_prefix="$HOME/.local"
if [[ $# -gt 0 ]]; then
  [[ $# == 2 && $1 == --prefix && $2 == /* ]] || { echo "Usage: $0 [--prefix /absolute/path]" >&2; exit 1; }
  install_prefix=$2
  app_data="$install_prefix/share"
else
  app_data="${XDG_DATA_HOME:-$HOME/.local/share}"
fi
install -d "$install_prefix/bin"
for binary in global-network-map global-network-map-capture global-network-map-gui; do
  install -m 755 "$binary_dir/$binary" "$install_prefix/bin/$binary"
done
install -d "$app_data/applications" "$app_data/icons/hicolor/scalable/apps"
install -m 644 assets/global-network-map.svg "$app_data/icons/hicolor/scalable/apps/global-network-map.svg"
# Escape the executable path according to desktop-entry string and Exec rules.
python3 - "$install_prefix/bin/global-network-map-gui" "$app_data/applications/global-network-map.desktop" <<'PY'
from pathlib import Path
import sys
executable = sys.argv[1]
if '\n' in executable or '\r' in executable:
    raise SystemExit('Unsupported newline in installation path')
escaped = ''.join('\\' + c if c in '\\"`$' else c for c in executable)
escaped = escaped.replace('\\', '\\\\').replace('%', '%%')
Path(sys.argv[2]).write_text(f'''[Desktop Entry]
Type=Application
Name=Global Network Map
Comment=Live local network traffic on a world map
Exec="{escaped}"
Icon=global-network-map
Terminal=false
Categories=Network;Monitor;
Keywords=network;traffic;map;bandwidth;
StartupNotify=true
StartupWMClass=global-network-map
Actions=Demo;

[Desktop Action Demo]
Name=Preview with demo traffic
Exec="{escaped}" --demo
''')
PY
if command -v desktop-file-validate >/dev/null; then
  desktop-file-validate "$app_data/applications/global-network-map.desktop"
fi
if command -v update-desktop-database >/dev/null; then
  update-desktop-database "$app_data/applications"
fi
printf 'Installed Global Network Map and its desktop launcher.\nPreview: global-network-map-gui --demo\nLaunch: global-network-map-gui\n'
