# Upstream alignment and telemetry review

Reviewed October 31, 2026 against original XARA **0.1.3**, commit [`09a9201381b00499ada9d9c816bd05bb8f175705`](https://github.com/x0rro/xara/commit/09a9201381b00499ada9d9c816bd05bb8f175705). The previous macOS distribution used upstream 0.1.1. The current app is **0.1.3 / macOS build 8**.

## Upstream integration

The contents of `crates/xara-phy` and `crates/xara-arq` match upstream 0.1.3 exactly. The original developer's MIT license and attribution remain intact. The native Core Audio work in upstream credits NR5P.

Integrated upstream changes include:

- Callsign/SSID packing and decoding corrections, including the updated vectors. Native station-entry validation delegates directly to the modem, including its supported letter SSIDs (T/R/X/L).
- HF/FM DSP changes and decoder/FFT warm-up.
- Revised FM timing and TX scheduler behavior.
- Native Rust Core Audio backend and its Core Foundation/AudioToolbox bindings, replacing this fork's earlier C adapter.
- Capture/output buffering, resampling, clock-rate matching, and dynamic latency budgets. The default TX queue target is 40 ms; scheduler timing accounts for measured capture/output paths and PTT tail, rather than the previous fixed 30 ms assumption.
- CLI device enumeration, optional timestamped logs, optional experimental `(tr)uSDX` transport, updated platform/build documentation, and macOS CM108 USB HID PTT.

Intentional differences from upstream:

| Area | macOS distribution behavior |
| --- | --- |
| Native application | AppKit/WKWebView shell with the modem linked into the same process |
| Host listeners | Explicit shutdown/join and restart lifecycle, automatic cleanup on drop, and an in-process mode without command/data sockets |
| Network defaults | Authenticated native telemetry/controls; command/data/KISS ports off until explicitly enabled; loopback-only native listeners |
| Repeated host initialization | An unchanged full `MYCALL` list is acknowledged while busy without reconfiguration; different/invalid lists and idle initialization retain the original behavior |
| Saved settings | Owner-only directory/files; preserved encoded Core Audio UIDs; automatic TX-drive persistence |
| Drive range | −30 to +10 dB in the native UI and host controls; the +10 maximum is an intentional extension to upstream's 0 dB maximum |
| Radio profiles | Private named snapshots of all native station/radio settings, including drive/timing; explicit load review and idle guard |
| Diagnostic logging | Automatic local operational logs, bounded to two 1 MiB files; authenticated/manual export; no payload/audio capture |
| Main UI | Additional instrument panels, per-connection bytes, protocol indicators, diagnostics, and accurate output/PTT telemetry |
| CLI-only features | Explicit `--record`/`--log`, digipeater configuration, optional registered speeds, and experimental `(tr)uSDX` remain available in source/CLI configurations; the standard native app does not expose or enable them |
| Startup installation | The upstream launchd example is retained as documentation; the app build does not install a service or login item |
| Updates | User-selected metadata check and GitHub release-page link only; no automatic checks, downloads, installation, rebuilding, tool installation, or restart |

There is no claim that the native UI exposes every CLI flag. Core HF/FM functionality and the current upstream modem implementation are included; the differences above are explicit integration choices.

## TX drive

The native app automatically remembers the last accepted value, including values above 0 dB through +10 dB. Changes from the UI or enabled host interfaces enter the same modem drive path. A bounded/coalescing worker writes the setting off the DSP thread and flushes the final pending value before shutdown or engine restart. Other saved settings are preserved. The UI reports Applied/Saved feedback and displays save errors.

Positive drive is digital waveform gain relative to the modem's nominal level. It is not a measurement of RF gain or power, and higher settings can cause digital clipping. The UI exposes full-scale sample counts and TX CLIP. It does not bypass PCM limits or radio hardware controls.

## Main-tab telemetry mapping

| Instrument | Source and representation |
| --- | --- |
| BPS and frame level | `Report::Bitrate` / host bitrate response from the modem; latest reported frame bitrate and record level, not measured user-payload throughput |
| BPS history | History of available reported frame rates; missing reports are not fabricated as zero-rate samples |
| RX RMS/peak | Selected Core Audio input PCM at the 48 kHz modem boundary, before modem demodulation |
| TX RMS/peak | Quantized, rate-matched PCM submitted to Core Audio after muting and PTT-lead delay; receive-only mode does not meter unwritten generated audio |
| Clip counts/LEDs | Digital full-scale samples per telemetry interval; they do not detect analog/RF distortion elsewhere in the radio |
| TX/PTT LEDs | PTT controller assertion state, including configured tail and watchdog release; not sensed RF power |
| RX LED | Receive-state/decoder activity; successful receive-side control detections also pulse RX |
| ACK/NACK/REQ/QRT/BREAK/START | Actual decoded or scheduled/keyed protocol reports; mappings checked against upstream HF/FM control constants and FM START records |
| DATA LED | Frame/payload activity reports, rather than ambient audio energy |
| S/N and history | S/N reports from decoded modem frames; missing or disconnected values remain unavailable |
| AFC | HF receiver frequency-offset estimate in hertz; unavailable in FM |
| Link status/callsigns | Modem state and configured/peer callsigns; established-connection flag comes from CONNECTED/DISCONNECTED reports, so calling/answering is not falsely styled as connected |
| Received bytes | Full decoded connection payload byte count, reset on CONNECTED and retained after disconnect; UI reconnection or monitor Clear does not reset it |
| Link level, attempts/retries, buffer | Station/session state and modem host configuration, not UI estimates |
| RX waterfall/passband | Actual receive input audio and mode passband; transmit output is not mixed into this display; published FFT/span/floor/quantization parameters are shown in Diagnostics |
| DSP time/drops | Processed modem sample time and telemetry queue drops; neither is network latency or packet error rate |

The display clears stale live instruments and preserves the documented connection byte-total semantics. Audio meters show digital PCM levels in dBFS, not RF watts.

## GitHub issue review and device selection

Reviewed the fork's [audio-device issue #1](https://github.com/N4EAC/xara-for-macos/issues/1), filed against build 2. The current build uses upstream's newer Rust Core Audio backend. Build 6 additionally preserves a device's usable direction when its driver rejects the other stream-configuration scope. Regression tests cover input-only/output-only errors and multi-buffer aggregate channel counts. The app's inventory was compared with an independent Core Audio implementation on the development Mac: all 10 inputs and 2 outputs matched. Jeremy's November 1 build 7 attachment confirmed a separate native selection bug: three same-named USB inputs and three same-named USB outputs were present. `NSPopUpButton.addItems(withTitles:)` collapses duplicate titles, shifting later selection indices against the original device array; selecting IC-7100 therefore resolved to Jump Desktop Audio. This exact mapping was reproduced with a synthetic version of his device ordering. Build 8 creates explicit menu items carrying their stable device IDs, gives duplicate names distinct labels, and retrieves selection from the item instead of an external array index. Tests cover every device, duplicate labels, reordered enumeration and unavailable devices. His Setup-tab workaround successfully selected the IC-7100 aggregate at 48 kHz and later established radio sessions; the physical setup still needs build 8 field confirmation.

Shared native/web Setup now uses device-name dropdowns rather than datalist autocomplete, which could show encoded UIDs and filter suggestions by the existing value. Identifiers remain stable in settings; unavailable selections are preserved across refresh/hot-plug instead of silently selecting another output. Native Station & Radio already uses named popup menus.

The waterfall is explicitly labelled **RX waterfall / Live RX spectrum**. It receives only the selected input, including any audio the radio supplies during transmit. It does not display internally generated TX audio; quiet rows during transmit are expected. TX RMS/peak/clip meters remain the output instrumentation.

The original repository remains at the reviewed 0.1.3 commit. Its two open issues, [wider xara-monitor bandwidth #32](https://github.com/x0rro/xara/issues/32) and [Raspberry Pi GPIO PTT #27](https://github.com/x0rro/xara/issues/27), are feature requests outside this native macOS build. Closed reports concerning SSIDs, HF PTT-tail timing, FM timing and host descriptor cleanup were checked against the incorporated upstream changes and this fork's lifecycle tests. No new PHY/ARQ modifications were needed.

## Named radio profiles

The native Profiles menu saves/replaces/loads/deletes up to 32 named configurations. A profile includes callsign, HF/FM mode, stable input/output UIDs, PTT method and endpoint, external listener enablement and ports, TX drive, and PTT lead/tail. Saving merges active persisted web/drive overrides with native-only settings. Loading opens Station & Radio for review; it does not key or start a radio automatically. Both the opening and application paths check modem-idle state. Device menus preserve unavailable UIDs. Applying a profile replaces all active saved radio overrides, so the previous radio's drive/timing cannot override the loaded profile.

Profiles use a versioned, size-bounded JSON file with 0600 permissions in the 0700 app-state directory. Read/write reject symlinks, hard links, nonregular files, incorrect ownership, unsupported schema, malformed configurations, duplicate names/IDs, and invalid or excessive names. Writes use an exclusive temporary file, synchronization and atomic rename. New optional station fields retain compatibility with older saved native settings. Deleting a snapshot does not mutate active configuration; replacements and deletions are explicit.

The native profile regression script checks two different radio configurations end-to-end through serialization, saved-setting merges, legacy decoding, invalid settings/names, duplicates, permissions and unsafe/malformed-file rejection. It runs in macOS CI and starts no audio or PTT.

## Automatic operational diagnostics

Build 7 initializes an owner-only rotating log before audio permission is requested. The background writer uses a 256-entry nonblocking queue with per-record length limits, UTC timestamps, a 1 MiB current file and a 1 MiB previous file. File paths reject symlinks/nonregular files/hard links; the app state and diagnostic directories are private. Rotations and launch retain only the newest two files. Export reports queue drops and the last write failure; pending records are synchronized before the snapshot. An unresponsive writer times out rather than blocking the UI indefinitely.

Records cover native OS/build/permission state, complete available audio inventory and failed property operations/scopes, input/output selection, rates/channels/latency, initial/current timing budgets, PTT configuration and safety release, engine errors, sanitized command kinds/results, modem/PTT transitions and 10-second signal/timing summaries. Internal panic location/backtrace is included without the panic payload. Audio buffers, payload previews, raw host commands, callsign configuration dumps and authenticated UI URLs/tokens do not enter the log. This is deliberate allowlisted instrumentation, not a copy of console/monitor text. Device IDs/names and diagnostic paths remain potentially identifying and are disclosed before export.

The macOS Modem menu exports without a running engine. The Diagnostics tab exports through an authenticated same-origin `/diagnostics.txt` request and a local blob download. The route requires the control cookie even outside private mode, rejects foreign Origin/cross-site navigation, uses no-store headers and returns explicit unavailable/error responses. No new listener or outbound request was introduced. Tests cover retention/restart/permissions, symlink/hard-link rejection, worker snapshots, control-character limits, panic payload exclusion, HTTP authentication and export failure handling. Browser tests cover successful and failed exports without disabling later retries. The original modem PHY/ARQ remains unchanged.

## About and Check for Updates

The macOS application menu's About panel identifies **Xara modem for Mac — By N4EAC, Eduardo**, displays the bundle's version/build, credits x0rro, and offers GitHub Releases and the original repository.

**Update → Check for Updates…** reads public metadata from `https://api.github.com/repos/N4EAC/xara-for-macos/releases/latest` only when selected. The request uses an ephemeral URLSession without shared cookies, credentials, or a disk cache. No callsign, audio, payload, device settings, or installed-version identifier is sent. Comparison is performed locally using the app version and macOS build number.

- Equal versions show exactly: **“Xara UI for Mac by N4EAC is up-to-date.”**
- A newer release offers its N4EAC GitHub release page.
- A local build newer than the published release is identified as such; it is not downgraded.
- Failed requests, unsupported tags, drafts, and prereleases are not reported as up-to-date.

There is no app/tool installer, automatic updater, or automatic rebuild/restart. Users do not need Rust or Xcode to run the prebuilt application or use this check.

## Validation

- **459 workspace tests passed; 8 ignored** because they require physical loopback or external VARA captures. This includes simulated HF/FM audio links, DSP/protocol vectors, byte counters, host interfaces, private UI access, drive persistence across a full engine restart, +10 dB limits, PTT tail/controller state, and device-output meter behavior.
- Additional MYCALL regressions verify the exact `N4EAC` retry, full-list matching, live HF/FM payload transfer, unchanged registration timing, and preserved rejection/deferred changes. This confirms the modem behavior; the reported MacWinlink incident still requires field confirmation.
- Clippy for macOS app features/all targets passed with warnings denied; rustfmt and whitespace checks passed.
- Browser regression checks passed for the +10 dB slider command, Saved feedback, diagnostics, waveform display, safe payload text, setup, export, responsive layout, stale values, and reconnection behavior.
- Native regression checks reproduce the old IC-7100-to-Jump mapping and verify stable device-ID selection, distinct duplicate names, reordered lists and missing-device preservation.
- Native profile persistence and compatibility checks passed using `scripts/test_macos_profiles.sh`; the final native app compiled and was signed.
- Native About construction and release-version parsing/comparison were tested with active preconditions. The user-requested GitHub metadata endpoint was checked live. Malformed tags and non-public release metadata were rejected.
- The local macOS app was rebuilt with hardened runtime, its signature verified, and the ZIP checked against every app-bundle file.
- No on-air session or physical audio/USB/PTT hardware validation was performed for this revision. The eight ignored tests remain explicit validation limits.

Repeat the main test suite with:

```sh
cargo test --locked --release --workspace --no-default-features --features xara/macos-app
```

The October 30 [security review](SECURITY_REVIEW.md) remains evidence for build 2; its artifact hashes and test counts are historical. Build 8 retains the private-listener and settings-permission defaults. The manual GitHub metadata check remains the disclosed network addition; automatic operational diagnostic files are the newly requested storage addition.

## Build 8 package

App: `target/macos/XARA.app`. ZIP: `target/release-assets/v0.1.3-macos.8/XARA-macOS-arm64.zip`. Apple Silicon arm64, macOS 12+; ad-hoc signed with hardened runtime, not notarized.

```text
0c8f7db2cacf9630ba94dcc221c11b1cde4a9afcce9d07f18f890905bcb22eb2  XARA-macOS-arm64.zip
```
