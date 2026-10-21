#!/bin/sh
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
ver=$(sed -n 's/^version = "\(.*\)"$/\1/p' "$root/Cargo.toml" | head -1)
check=${1:-}
stale=0
for src in "$root"/dist/man/*.1.md; do
	out=${src%.md}
	new=$(pandoc -s -f markdown-smart-subscript-superscript-tex_math_dollars-pipe_tables -t man \
		-M footer="xara $ver" -V header-includes=.nh -V header-includes='.ad l' "$src")
	if [ "$check" = --check ]; then
		if [ "$new" != "$(cat "$out")" ]; then
			echo "${out#$root/} is stale" >&2
			stale=1
		fi
	else
		printf '%s\n' "$new" >"$out"
	fi
done
exit $stale
