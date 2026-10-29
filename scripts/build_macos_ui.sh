#!/bin/sh
set -eu
root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
output=${1:-"$root/target/macos/XARA.app"}
mkdir -p "$output/Contents/MacOS" "$output/Contents/Resources"
export MACOSX_DEPLOYMENT_TARGET=12.0
cargo build --manifest-path "$root/Cargo.toml" --locked --release -p xara --lib --no-default-features --features macos-app
static_lib="${CARGO_TARGET_DIR:-$root/target}/release/libxara.a"

CLANG_MODULE_CACHE_PATH="${TMPDIR:-/tmp}/xara-clang-cache" \
  swiftc -O -module-cache-path "${TMPDIR:-/tmp}/xara-swift-cache" \
  -target "$(uname -m)-apple-macosx12.0" -framework AppKit -framework WebKit -framework AVFoundation -framework AudioToolbox -framework CoreAudio -framework CoreFoundation -framework Security -liconv \
  -import-objc-header "$root/macos/XARA.h" "$root/macos/XARA.swift" "$static_lib" -o "$output/Contents/MacOS/XARA"
swift -module-cache-path "${TMPDIR:-/tmp}/xara-swift-cache" "$root/macos/Icon.swift" "$output/Contents/Resources/XARA.icns"
cp "$root/macos/Info.plist" "$output/Contents/Info.plist"
codesign --force --sign - --entitlements "$root/macos/Entitlements.plist" "$output"
printf 'Built %s\n' "$output"
