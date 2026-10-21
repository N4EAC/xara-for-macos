#!/bin/bash
set -euo pipefail
target=$1
root=$(cd "$(dirname "$0")/.." && pwd)
out=$(mkdir -p "${2:-$root/target/dist}" && cd "${2:-$root/target/dist}" && pwd)
ver=$(sed -n 's/^version = "\(.*\)"$/\1/p;T;q' "$root/Cargo.toml")
name=xara-$ver-$target
cargo build --manifest-path "$root/Cargo.toml" --locked --release --target "$target" -p xara
stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
mkdir "$stage/$name"
cp "$root/target/$target/release/xara" "$root/target/$target/release/xara-monitor" \
    "$root/LICENSE" "$root/README.md" "$stage/$name/"
cp -r "$root/dist" "$stage/$name/"
tar -C "$stage" --sort=name --owner=0 --group=0 --numeric-owner -cf - "$name" | xz -9 > "$out/$name.tar.xz"
ls -l "$out/$name.tar.xz"
