#!/bin/sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
build=${1:-$here/../target/release}
prefix=${PREFIX:-/usr}
d=${DESTDIR:-}

install -Dm755 "$build/xara" "$d$prefix/bin/xara"
install -Dm755 "$build/xara-monitor" "$d$prefix/bin/xara-monitor"
install -Dm644 "$here/man/xara.1" "$d$prefix/share/man/man1/xara.1"
install -Dm644 "$here/man/xara-monitor.1" "$d$prefix/share/man/man1/xara-monitor.1"
for u in system/xara@.service user/xara.service; do
	install -d "$d/usr/lib/systemd/${u%/*}"
	sed "s#/usr/bin/xara#$prefix/bin/xara#" "$here/systemd/${u#*/}" >"$d/usr/lib/systemd/$u"
	chmod 644 "$d/usr/lib/systemd/$u"
done
install -Dm644 "$here/sysusers.d/xara.conf" "$d/usr/lib/sysusers.d/xara.conf"
install -Dm644 "$here/udev/60-xara-ptt.rules" "$d/usr/lib/udev/rules.d/60-xara-ptt.rules"
install -Dm644 "$here/xara.conf.example" "$d$prefix/share/doc/xara/xara.conf.example"
install -d "$d/etc/xara"
