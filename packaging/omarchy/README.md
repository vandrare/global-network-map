# Arch / Omarchy package

This directory is ready to copy to `pkgbuilds/global-network-map/` in a fork of
https://github.com/omacom/omarchy-pkgs. Official acceptance and signing are handled
by that repository's maintainers. `.omarchy/package.json` marks this as a local
recipe; version updates are manual until an upstream watch is configured there.

The recipe packages the upstream release archive, verifies its SHA-256 checksum,
and installs normal unprivileged executables into `/usr/bin`. It grants no
persistent capabilities, installs no service or polkit policy, and does not
modify desktop or terminal configuration. Capture uses the existing desktop
polkit agent to authorize its helper, which drops privileges after opening pcap.

Build with `makepkg --cleanbuild`. The desktop file and licence/attribution are
installed in the standard Arch locations. Runtime libraries and capture tools
are declared in `depends`; users need no compiler for a release installation.
This initial release supports x86_64. It was built on Arch Linux with the system
Rust toolchain. No ARM binary or compatibility with older distribution libraries
is claimed.

For another release:

1. Update Cargo.toml/Cargo.lock and README versioned installation commands.
2. Run the checks documented in the root README and render a fresh demo preview.
3. Run `scripts/build-release.sh` once for the final archive.
4. Update PKGBUILD's version and SHA-256 checksum from that exact archive; regenerate
   `.SRCINFO` with `makepkg --printsrcinfo > .SRCINFO` from this directory.
5. Copy the archive here and run `makepkg --cleanbuild` to validate and build the package.
6. Commit/tag, then publish the exact archive, package, source archive and SHA256SUMS.

Do not regenerate the release archive after fixing its checksum in PKGBUILD:
tarball metadata can change. Keep the verified artifact for publication.

Initial validation: Rust unit and pcap replay tests, GUI clippy, formatting,
terminal PTY smoke tests, isolated user installer and desktop-entry validation,
makepkg build and file manifest checks, and a Wayland synthetic demo screenshot.
Live GUI authentication and VPN capture still require manual verification.
