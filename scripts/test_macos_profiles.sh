#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
export MACOSX_DEPLOYMENT_TARGET=12.0
cargo build --manifest-path "$root/Cargo.toml" --locked --release -p xara --lib --no-default-features --features macos-app
mkdir -p "$root/target/profile-tests"
# Compile the production declarations without launching NSApplication/the modem.
sed '/^let app = NSApplication.shared/,$d' "$root/macos/XARA.swift" > "$root/target/profile-tests/XARA.swift"
swiftc -O -warnings-as-errors -module-cache-path "${TMPDIR:-/tmp}/xara-swift-cache" \
  -target "$(uname -m)-apple-macosx12.0" -framework AppKit -framework WebKit -framework AVFoundation -framework AudioToolbox -framework CoreAudio -framework CoreFoundation -framework Security -framework IOKit -liconv \
  -import-objc-header "$root/macos/XARA.h" "$root/target/profile-tests/XARA.swift" "$root/macos/Tests/main.swift" \
  "${CARGO_TARGET_DIR:-$root/target}/release/libxara.a" -o "$root/target/profile-tests/profile-tests"
"$root/target/profile-tests/profile-tests"
