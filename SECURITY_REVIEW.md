# Xara for macOS security and privacy review

Review date: October 30, 2026. Baseline: commit `85783fb` / published release `v0.1.1-macos.1`. This report covers the local security changes made after that release, the rebuilt local macOS app, and the evidence below.

**Result:** No evidence of malware, analytics, credential harvesting, hidden startup persistence, automatic updates, or unsolicited Internet calls was found in the reviewed application source. Concrete local-access and privacy weaknesses were identified and corrected. This is a source review with targeted tests and dependency checks, not a guarantee that every possible vulnerability has been eliminated.

**Publication status:** Released as `v0.1.1-macos.2` (macOS application build 2). The superseded `v0.1.1-macos.1` release and its downloadable assets have been removed from this repository. The original Git source history is retained.

## Scope and method

Reviewed the native Swift application and permissions, Rust embedded lifecycle, Core Audio C/Rust boundary, UI server and WebSocket parser, UI JavaScript and rendering, settings persistence, host/KISS listeners, PTT transports, compression bounds, build/install scripts, and dependency manifest/lockfile. Followed network, file-write, process-launch, unsafe-code, and export paths. Checked compiled-library dependencies and signature metadata. Searched application source for credential patterns and unexpected network/storage APIs.

Validated with Rust tests, Clippy, rustfmt, Clang static analysis, Chromium browser checks, a native WebKit test bundle, archive/source checksums, and socket inspection of an isolated UI probe. The probe used simulated telemetry and never opened microphone input, radio output, or PTT. The test used the actual native navigation delegate and bundled ATS configuration. No running XARA app was found during initial inspection; the live socket check therefore describes the isolated test process, not an on-air production session.

No binaries or user data were submitted to an external malware-scanning service. The official RustSec database was fetched to check known dependency advisories. This was not an exhaustive independent audit of every third-party dependency or Apple framework.

## Findings and local corrections

| Finding | Assessment | Correction |
| --- | --- | --- |
| UI token protected controls, but unauthenticated local clients could receive telemetry, payload previews, history, or waterfall through WebSocket/NDJSON endpoints | Local confidentiality weakness; loopback binding limited network exposure but did not authenticate readers | Native app now requires its random session token for both `/ws` and `/stream`; unauthenticated reads return 403 |
| Command/data and KISS ports opened whenever the native modem started | Unnecessary exposure for standalone app use; protocols intentionally lack authentication | All three listeners are off by default, including when loading older native preferences; user must enable external-software ports in Station & Radio |
| Saved settings used mode 0644 | Defense-in-depth issue: the existing 0700 directory already blocked traversal by other users, but file permissions were broader than needed | Rust and native settings writes now create owner-only files (0600), atomically replace settings, and use an owner-only state directory (0700); startup repairs existing permissions and refuses symlink settings/state paths |
| CSP allowed WebSocket connections to arbitrary hosts via `ws:` / `wss:` source schemes | Excess permission if a separate script-injection defect ever occurred; no such injection was found | Connections restricted to the UI's own origin; browser tests verify external HTTP and WebSocket requests are blocked |
| Native WebKit had a general arbitrary-web-content network exception and an overly broad download path | Permissions broader than the current app needs | ATS exception narrowed to local networking; native navigation restricted to its modem endpoint; downloads restricted to locally generated monitor-export blobs |
| Configured rigctld response used an unbounded line read | A faulty or malicious user-selected rigctld server could grow memory with an oversized response | Response capped at 1024 bytes, with existing connection/read/write timeouts retained; regression test added |
| App signing did not request hardened runtime | Missing platform defense in depth | Build script now signs with hardened runtime; signing is still ad-hoc, not notarized or Developer ID authenticated |
| Saved-data behavior was insufficiently visible | Users should know what settings and exports persist | Added notices to native configuration, web Setup, README, and this review |

The source changes are included in the commit tagged `v0.1.1-macos.2`. The release ZIP is built from that source with macOS application build number 2; the upstream modem version remains 0.1.1.

## Network exposure after hardening

| Interface | Default state | Binding/destination | Authentication/purpose |
| --- | --- | --- | --- |
| Native UI HTTP/WebSocket/NDJSON | On while modem runs | One ephemeral TCP port on `127.0.0.1` | Telemetry and controls require a random 128-bit session token; static UI assets contain no user data |
| Command TCP | Off | `127.0.0.1:8300` when enabled; configurable | Unauthenticated compatibility protocol for trusted external radio software |
| Data TCP | Off | Command port + 1, normally `127.0.0.1:8301` | Unauthenticated compatibility protocol |
| KISS TCP | Off | `127.0.0.1:8100` when enabled; configurable or disabled separately | Unauthenticated compatibility protocol |
| Hamlib rigctld | No connection unless selected as PTT method | User-specified host/port, default `127.0.0.1:4532` | Explicit PTT control, not an analytics or payload-upload connection |
| Analytics, update checks, cloud uploads, advertising | Absent from reviewed app code | None | No runtime phone-home endpoint found |

The native app has no wildcard (`0.0.0.0` / `::`) listeners, UDP discovery service, UPnP/NAT port mapping, or automatic firewall rule changes. Enabling external-software ports does not expose them to the LAN through the app itself.

The private UI also requires the exact bound IP/port in the Host header, checks WebSocket Origin, rejects cross-site subresource requests, uses an HttpOnly/SameSite=Strict session cookie, sets no-store and no-referrer headers, and limits request/frame sizes, connection counts, queues, and timeouts. The token is generated in memory and is not written to an app token file. Opening the interface in an external browser deliberately passes the token in its initial local login URL, then redirects to a clean URL.

**Residual local-access risk:** Command/data and KISS protocols do not authenticate clients. When enabled, other local processes or users may connect, read traffic, or issue commands; loopback is not per-user isolation. Browser-origin protections on the UI do not authenticate these legacy TCP protocols. Enable them only when needed. Software already running as the same macOS user may also access settings, browser state, or process data; this app is not a security boundary against an already-compromised account.

**Separate CLI modes:** The standalone command-line modem opens command/data ports as its primary interface. Its optional UI is off by default; explicitly enabled read-only CLI/monitor web interfaces retain upstream's anonymous-read behavior. CLI users can explicitly choose non-loopback addresses. The native app's private-UI and opt-in-port defaults must not be assumed to apply to every CLI configuration. No CLI configuration was changed to expose new interfaces.

## Data storage and user awareness

| Data | Where/how it persists | Trigger |
| --- | --- | --- |
| Callsign, HF/FM mode, audio-device identifiers, drive and PTT timing | `~/Library/Application Support/XARA/settings`, mode 0600 inside a 0700 directory | User saves configuration/settings |
| Native station configuration, PTT method/address, command/KISS choices and opt-in state | macOS UserDefaults for `org.xara.modem` | User saves native Station & Radio configuration |
| Window geometry | macOS application preferences | Standard window autosave behavior |
| Native web-view cookies/display preferences | Nonpersistent WebKit data store | UI session; not a persistent website store |
| External-browser display preferences | Browser localStorage (waterfall/display settings) | User changes display preferences |
| External-browser session cookie and local login URL/history | Browser-managed state | User chooses Open in Browser; retention depends on the browser |
| Received payload previews, telemetry/event history, audio buffers and waterfall | Bounded application/browser memory | Modem/UI operation; no automatic payload/audio log or upload |
| Monitor export | User-selected file in the native app; browser-managed download in external browsers | Explicit Export action; native app presents a save dialog |
| Standalone CLI audio recording | User-provided FLAC/WAV path | Explicit `--record` CLI/config option; not invoked by the native app |

The app does not search documents, contacts, mail, browser profiles, passwords, Keychain items, or unrelated user files in its reviewed runtime paths. Audio capture requires macOS microphone permission. No macOS login item, LaunchAgent, cron job, or background service is installed by the app. The repository's Linux systemd installer is a separate, explicitly invoked Linux installation path.

Clear removes the monitor view, not all server-side in-memory replay history. Quitting/restarting the engine releases that history. This is not a forensic memory-erasure guarantee: macOS controls paging, crash reports, and backups. App settings are not encrypted at rest; they contain station/hardware preferences, not account passwords.

## Dependencies and supply-chain checks

- Checked all **13 registry package names/versions in Cargo.lock** against the official [RustSec advisory database](https://github.com/RustSec/advisory-db), revision `ee1a49a2769a24a596afc1c20cb33748d2769586` fetched during this review.
- The matching advisory was [RUSTSEC-2023-0080](https://rustsec.org/advisories/RUSTSEC-2023-0080.html), a `transpose` buffer-overflow issue. Its patched range is `>= 0.2.3`; the lockfile uses **0.2.3**. No affected locked version was identified by this comparison.
- Verified the archive SHA-256 for all **11 registry dependencies compiled for macOS** against Cargo.lock. Compared every extracted source file with its archive and checked for unexpected extra source files; all matched.
- `alsa-sys` and `pkg-config` are locked Linux-only dependencies, not included in the macOS build. Their names were included in the advisory comparison; their archives were not present locally and were not checksum-verified here.
- Build uses `--locked`. Runtime dynamic-library inspection showed Apple system frameworks/libraries; the Rust modem and its dependencies are statically linked. No bundled downloader, updater, or third-party analytics framework was found.
- Build tooling may contact package registries or download Rust dependencies during development. That is distinct from runtime app behavior. A checksum match verifies package integrity relative to the lockfile, not the trustworthiness of an entire upstream supply chain.

## Validation results

**146 Rust tests passed** in the final verification:

- 20 host tests, including a disabled host without TCP endpoints.
- 55 UI tests, including unauthenticated WebSocket/NDJSON denial, authenticated telemetry, Host restrictions, and invalid private-server configuration.
- 35 core/embedded unit tests, including private file permissions, symlink rejection, default-disabled host ports, settings restart, and shutdown.
- 26 PTT tests, including bounded rigctld responses and correct macOS rejection of Linux-only CM108 operations.
- 10 telemetry integration tests.

Additional checks passed:

- `cargo fmt --all --check`.
- Clippy on macOS app features/all targets with warnings denied.
- Clang static analyzer on the Core Audio C adapter: **zero diagnostics**.
- Chromium against the actual private UI server: authenticated live telemetry, clean login redirect, HttpOnly cookie, anonymous telemetry denial, external fetch/WebSocket CSP rejection.
- Native WebKit test bundle using the actual native navigation delegate, nonpersistent data store, local-only ATS configuration, and hardened signing: authenticated live telemetry and blocked external fetch. This did not start audio or PTT.
- Existing browser smoke suite: instruments, safe payload rendering, waterfall, tabs/settings, export, responsive layout, stale/disconnected state, and session-counter reset.
- Swift legacy-preference decoding: existing station fields retained, external ports off until explicitly enabled.
- Socket inspection of the isolated UI probe: exactly one listener on `127.0.0.1`, no external established sockets in the probe process. No radio/audio session was used for this check.
- Signature verification, arm64 architecture check, and ZIP-to-bundle byte comparison passed. All temporary audit servers were stopped afterward.

Physical radio transmission, all audio hardware, every macOS version, exhaustive fuzzing, and long-duration packet capture of an on-air session were not performed. The app is **not App Sandbox-contained**; hardened runtime is an additional code-integrity defense, not a filesystem/network sandbox. It remains ad-hoc signed and not notarized.

## Rebuilt local artifacts

- App: `target/macos/XARA.app`
- Executable: `target/macos/XARA.app/Contents/MacOS/XARA`
- Release ZIP: `target/release-assets/v0.1.1-macos.2/XARA-macOS-arm64.zip`
- Local reviewed ZIP: `target/security/XARA-macOS-arm64-security-reviewed.zip`
- Updated conventional ZIP: `target/macos/XARA-macos.zip`
- Compatibility: arm64 / Apple Silicon, macOS 12+ deployment target.
- Signing: ad-hoc + hardened runtime (`adhoc,runtime`); signature verified.

SHA-256:

```text
0938f63371796ae6663aa5388351d24c65a3807d42871b71f2f279e2d31322a8  XARA executable
0b32a00fc1b6efae47768df62b2870609d60f5dd573ad1bc5490afcbb390b23f  XARA-macOS-arm64.zip
```

The previous ZIP is retained only as a local archive under `target/release-assets/v0.1.1-macos.1/`; it is no longer a downloadable GitHub release. The current release assets are under `target/release-assets/v0.1.1-macos.2/`.

## Repeating the checks

```sh
cargo fmt --all --check
cargo clippy --locked -p xara --no-default-features --features macos-app --all-targets -- -D warnings
cargo test --locked --release -p xara-ui -p xara-host
cargo test --locked --release -p xara --no-default-features --features macos-app --lib
cargo test --locked --release -p xara --no-default-features --features macos-app --test telemetry --test ptt
clang --analyze -std=c11 -Wall -Wextra crates/xara/src/audio/coreaudio.c
./scripts/build_macos_ui.sh
codesign --verify --deep --strict --verbose=2 target/macos/XARA.app
```

These tests require permission to bind local sockets and, for PTT tests, use pseudo-terminals. A filesystem/network-restricted execution sandbox may refuse them even when the code is correct. See [README.md](README.md#build-from-source) for the normal build prerequisites and instructions.
