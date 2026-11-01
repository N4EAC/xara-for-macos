# Xara for macOS

A self-contained macOS application for the XARA VARA-compatible HF/FM modem. The Rust modem engine runs inside the app, with native Core Audio input/output, an instrument panel, and an optional local web interface. No separate modem installation is needed.

Based on **[XARA by x0rro](https://github.com/x0rro/xara)**. The original project provides the modem, DSP, protocol implementation, and host interfaces. This repository provides the macOS application and its integrated interface. It is an independent distribution; the original developer's MIT license and Git history are retained.

[Download the latest release](https://github.com/N4EAC/xara-for-macos/releases/latest) · [Original repository](https://github.com/x0rro/xara) · [Command-line guide](docs/cli.md)

![XARA macOS instrument panel](docs/images/xara-main.png)

## Compatibility

| Item | Release build |
| --- | --- |
| Processor | Apple Silicon (arm64; M1 or newer) |
| Operating system | macOS 12 Monterey or newer |
| Intel Macs | Not supported by the downloadable ZIP; it is not a Universal build |
| Modem version | XARA 0.1.3; macOS application build 7 |
| Audio | Native Core Audio; 48 kHz, signed 16-bit mono PCM internally |
| Signing | Ad-hoc signed; not Developer ID signed or notarized |

Rust and Xcode are only needed to build from source. The downloadable app contains its modem engine and UI. The minimum OS is the deployment target; operation has not been tested on every macOS release or radio interface. macOS may require approval to open the unnotarized app.

## Setup

1. Download **XARA-macOS-arm64.zip** from [Releases](https://github.com/N4EAC/xara-for-macos/releases/latest), unzip it, and move **XARA.app** to Applications.
2. Open XARA and grant microphone access when prompted. This permission also covers audio received through a USB radio interface.
3. Open **Modem → Station & Radio…** (⌘,) and enter your callsign, select HF bandwidth or FM mode, and choose your radio's audio input and output. The **Setup** tab contains station, audio, and timing settings.
4. Start in receive-only mode. To transmit, explicitly select the radio audio devices and configure serial RTS/DTR, CM108 USB HID, Hamlib rigctld, or VOX PTT. Check audio levels and PTT timing with your radio before an on-air session. For CM108 USB PTT, enter `VID:PID[:GPIO]` (four hexadecimal digits per USB ID). Device-path hidraw syntax remains Linux/FreeBSD-only.
5. If using external radio software, enable **External software → Enable command/data and KISS ports** in Station & Radio, then connect to `127.0.0.1`: command port **8300**, data port **8301**, or KISS TCP **8100**. These ports are off by default, are unauthenticated for compatibility, and can be changed in Station & Radio if already in use. Enable them only for trusted local radio software.

Save and restart applies engine settings while idle. Settings are stored in `~/Library/Application Support/XARA` and macOS app preferences.

**Modem → Open in Browser** (⌘⇧B) opens the same interface in your default browser. It connects to the app's running modem using an authenticated loopback address; keep the app open. **Modem → Fit to Panels** (⌘0) fits the window to the selected tab.

## Privacy and network access

The macOS app opens one authenticated UI listener on `127.0.0.1` using an automatically assigned port. Its telemetry, received-data previews, waterfall, and controls require the session token. Command/data and KISS listeners are disabled until explicitly enabled in Station & Radio. They remain loopback-only and are unauthenticated for compatibility with radio software. Older saved configurations also default to these ports disabled after this update.

There is no analytics, background update check, cloud service, or automatic Internet request in the app code. The modem engine’s only configurable outbound TCP connection is Hamlib rigctld PTT, used when selected; its default address is `127.0.0.1:4532`. **Update → Check for Updates…** makes a user-requested HTTPS metadata request to the N4EAC GitHub releases API. It sends no callsign, audio, payload, or device settings. A remote rigctld address connects to the host you explicitly configure.

Saving settings stores callsign, mode, audio-device identifiers, drive/timing, and native PTT/port preferences locally. The app settings directory is owner-only (`0700`) and the settings file is owner-only (`0600`). Window geometry is also saved by macOS. Received payload previews and audio buffers remain in memory; they are not automatically logged or uploaded. The app automatically saves bounded operational diagnostics locally, as described below. Monitor and diagnostic exports require a user action; the native app shows a save dialog. The separate command-line modem records audio only when explicitly given `--record`.

**Radio profiles:** Use **Profiles → Save Current Profile…** to save the current station, mode, input/output device IDs, PTT method/address, external-software setting, command/data/KISS ports, TX drive, and PTT lead/tail. Choose **Profiles → Load or Delete Profile…**, select a named profile, and choose Load. Review its settings in Station & Radio and click Start modem to apply it. Loading is refused while the modem is busy. Missing devices remain selected as unavailable; they are never silently replaced by a different audio output.

Up to 32 profiles are stored locally in `~/Library/Application Support/XARA/profiles.json` with owner-only permissions. Saving an existing name offers explicit replacement. Profiles are snapshots: later changes do not alter a saved profile until you replace it. Deleting a profile leaves the running modem unchanged. The Profiles menu belongs to the macOS app; the optional browser UI continues to edit the active modem settings. Older saved station settings remain compatible.

**Automatic diagnostic logs:** The app starts logging before microphone permission or modem startup, including when initialization fails. Logs are stored in `~/Library/Application Support/XARA/Diagnostics/current.log` and `previous.log`, owner-only (`0600`) in an owner-only directory (`0700`). Each file is limited to 1 MiB; rotation or a new launch replaces the older retained file. A bounded background writer keeps disk I/O off the audio loop and reports dropped records/write errors in exports.

Diagnostics include app/modem build, macOS version, device names and UIDs, input/output query failures, channel counts, hardware/modem rates, latency and queue configuration, microphone permission, engine errors, sanitized host-command kinds/outcomes, state/PTT transitions, watchdog/stall warnings, panic location/backtrace (without its message), and 10-second numeric audio/timing summaries. Summaries flag all-zero RX, full-scale samples and slow processing; these are observations, not automatic diagnoses of radio hardware. No raw audio, received message text, raw host commands, callsign configuration dumps or UI tokens are intentionally captured. Device identifiers and paths can identify hardware; review before sharing. Nothing is uploaded automatically.

Use **Modem → Export Diagnostic Log…** even if the modem cannot start, or **Diagnostics → Export diagnostic log** in the app or authenticated web UI. **Modem → Diagnostic Logging…** explains storage and initialization errors. The Data monitor's separate Export can include received message previews. Automatic diagnostics are a native-app feature; standalone CLI logging remains explicitly requested with `--log`. Sudden process termination or a full queue can lose the last queued records, and a failed disk can prevent logging.

The embedded web view uses a nonpersistent website data store. An external browser can retain the local URL, session cookie, and display preferences according to its own settings; no payload log is saved in browser storage by the UI. These protections do not isolate XARA from software already running as your macOS user. See [the security review](SECURITY_REVIEW.md) for scope, evidence, and limitations.

## Instruments and interfaces

- BPS history; audio input/output RMS and peak levels in dBFS; clipped-sample counts; AFC and S/N meters.
- DATA, ACK, NACK, REQ, QRT, BREAK, START, RX, and TX indicators; local and connected callsigns; connection state and received-byte total.
- Link/frame levels, retry and buffer information, passband edges, processed audio time, telemetry drops, waterfall, and received-data monitor.
- Setup and Diagnostics tabs, including reported modem/protocol versions, audio backend, meter limits, and waterfall parameters.
- HF bandwidths of 500, 2300, and 2750 Hz; FM narrow and wide modes; local command/data and KISS interfaces.

TX drive ranges from **−30 to +10 dB**. The embedded app saves the last accepted drive automatically, off the DSP thread, and restores it after engine restart or relaunch. The main tab displays Applied/Saved feedback or a save error. Positive drive is digital gain relative to the modem's nominal waveform level, not RF gain or power; full-scale clipping is visible in TX CLIP. The standalone CLI retains upstream's save-on-exit drive behavior when a state directory is configured.

The BPS readout is the latest modem-reported frame bitrate, not measured payload throughput. TX audio meters use the PCM written to the audio device after muting, delay, and rate matching. The TX/PTT indicators follow the PTT controller assertion, including its tail and watchdog release; they are not measurements of RF output. S/N comes from decoded frames, AFC from the HF receiver estimator (unavailable for FM), and clipping counts identify digital full-scale samples. Calling/answering states are not marked as established connections.

The received-byte total accumulates decoded payload bytes for each radio connection and resets at the next connection. Clearing the data monitor does not reset that counter. Its text preview shows up to 256 bytes per received payload. Protocol event LEDs remain visible briefly for readability; TX follows PTT. The Live indicator reports the UI telemetry connection, not a radio connection. Unavailable or stale instrument readings are cleared.

![XARA diagnostics](docs/images/xara-diagnostics.png)

## Build from source

Install [Rust](https://www.rust-lang.org/tools/install) (stable; upstream pins 1.99.1 in `.rust-version`) and Xcode Command Line Tools (`xcode-select --install`), then:

```sh
git clone https://github.com/N4EAC/xara-for-macos.git
cd xara-for-macos
./scripts/build_macos_ui.sh
open target/macos/XARA.app
```

The script statically links the Rust engine into an AppKit/WKWebView application, builds for the Mac's current architecture with a macOS 12 deployment target, and signs the bundle locally. The published ZIP is built on Apple Silicon. An Intel source build is not provided or validated by this release.

To package the app:

```sh
ditto -c -k --sequesterRsrc --keepParent \
  target/macos/XARA.app target/macos/XARA-macOS-arm64.zip
```

Use that asset name only for an arm64 build. Higher registered speeds require a source build with the upstream `registered` feature and a registered copy of VARA; the standard app build does not enable it. See the [CLI guide](docs/cli.md) for standalone builds and protocol options.

## Host initialization

When the modem is busy, repeating `MYCALL` with the exact callsign list already configured returns `OK` without reconfiguring the modem or disturbing an active link. Different, reordered, shortened, or invalid lists keep the existing rejection/deferred-change behavior. Normal idle initialization and registration timing are unchanged. This addresses a busy-state rejection that can interrupt MacWinlink initialization; it does not force the modem idle or automatically abort a radio connection.

## Upstream alignment

The modem PHY/ARQ crates match upstream 0.1.3, including the callsign/SSID fix, DSP warm-up, audio resampling/buffering, Core Audio backend, and revised latency/PTT-tail model. The native app uses that audio/latency path while preserving its authenticated UI, owner-only settings, and default-disabled external-software ports. Existing encoded Core Audio device selections remain compatible.

The original CLI source retains explicitly requested `--log`/`--record`, digipeating, and optional registered/experimental `(tr)uSDX` features. Payload/audio recording controls, digipeater configuration, registered speeds, and the experimental `(tr)uSDX` transport are not exposed or enabled by the standard native application. The optional upstream launchd example is not installed by the app build. See [UPSTREAM_REVIEW.md](UPSTREAM_REVIEW.md) for the comparison and telemetry mapping.

**Update → Check for Updates…** compares the app version and build with this repository’s latest published macOS release. Matching versions display “Xara UI for Mac by N4EAC is up-to-date.” A newer release offers its GitHub page. The check does not download, install, rebuild, or restart anything; using the app and checking for updates require no Rust or Xcode installation. Those tools are required only for building from source.

The waterfall displays **RX input audio**; it does not mix in generated TX audio. Quiet rows during transmit are normal when the radio mutes its receive output. Use TX RMS/peak and TX CLIP to inspect the outgoing digital audio.

Setup lists audio devices by name. Refresh devices after connecting an interface; an unavailable selection is preserved until you select a replacement.

The current app and release are 0.1.3 / macOS build 7.

## Validation

```sh
cargo test --locked --release -p xara-ui -p xara-host
cargo test --locked --release -p xara --no-default-features --features macos-app --lib embedded::tests
cargo test --locked --release -p xara --no-default-features --features macos-app --test telemetry
```

Embedded tests exercise the DSP loop with simulated PCM, authenticated UI access, settings, restart, and shutdown. The browser smoke test is `scripts/test_modem_ui.cjs` and requires Playwright/Chromium with the web directory served on port 8765. Automated tests do not substitute for validation with your radio hardware.

## Credits and license

Original XARA: **[x0rro/xara](https://github.com/x0rro/xara)** by **[x0rro](https://github.com/x0rro)**, updated here through commit [`09a9201`](https://github.com/x0rro/xara/commit/09a9201381b00499ada9d9c816bd05bb8f175705) (0.1.3). The upstream Core Audio work credits NR5P. The native application is maintained by **N4EAC, Eduardo**; its About menu links to the [macOS releases](https://github.com/N4EAC/xara-for-macos/releases). Upstream reports compatibility testing against VARA HF 4.9.0 and FM 4.4.0. This macOS distribution is not affiliated with the VARA developer.

Source is available under the [MIT license](LICENSE). Copyright © 2026 x0rro; original notices are preserved.
