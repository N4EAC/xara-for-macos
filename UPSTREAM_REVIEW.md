# Upstream alignment and telemetry review

Reviewed October 31, 2026 against original XARA **0.1.3**, commit [`09a9201381b00499ada9d9c816bd05bb8f175705`](https://github.com/x0rro/xara/commit/09a9201381b00499ada9d9c816bd05bb8f175705). The previous macOS distribution used upstream 0.1.1. The current app is **0.1.3 / macOS build 4**.

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
| Saved settings | Owner-only directory/files; preserved encoded Core Audio UIDs; automatic TX-drive persistence |
| Drive range | −30 to +10 dB in the native UI and host controls; the +10 maximum is an intentional extension to upstream's 0 dB maximum |
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
| Waterfall/passband | Actual input audio and mode passband; published FFT/span/floor/quantization parameters are shown in Diagnostics |
| DSP time/drops | Processed modem sample time and telemetry queue drops; neither is network latency or packet error rate |

The display clears stale live instruments and preserves the documented connection byte-total semantics. Audio meters show digital PCM levels in dBFS, not RF watts.

## About and Check for Updates

The macOS application menu's About panel identifies **Xara modem for Mac — By N4EAC, Eduardo**, displays the bundle's version/build, credits x0rro, and offers GitHub Releases and the original repository.

**Update → Check for Updates…** reads public metadata from `https://api.github.com/repos/N4EAC/xara-for-macos/releases/latest` only when selected. The request uses an ephemeral URLSession without shared cookies, credentials, or a disk cache. No callsign, audio, payload, device settings, or installed-version identifier is sent. Comparison is performed locally using the app version and macOS build number.

- Equal versions show exactly: **“Xara UI for Mac by N4EAC is up-to-date.”**
- A newer release offers its N4EAC GitHub release page.
- A local build newer than the published release is identified as such; it is not downgraded.
- Failed requests, unsupported tags, drafts, and prereleases are not reported as up-to-date.

There is no app/tool installer, automatic updater, or automatic rebuild/restart. Users do not need Rust or Xcode to run the prebuilt application or use this check.

## Validation

- **450 workspace tests passed; 8 ignored** because they require physical loopback or external VARA captures. This includes simulated HF/FM audio links, DSP/protocol vectors, byte counters, host interfaces, private UI access, drive persistence across a full engine restart, +10 dB limits, PTT tail/controller state, and device-output meter behavior.
- Clippy for macOS app features/all targets passed with warnings denied; rustfmt and whitespace checks passed.
- Browser regression checks passed for the +10 dB slider command, Saved feedback, diagnostics, waveform display, safe payload text, setup, export, responsive layout, stale values, and reconnection behavior.
- Native About construction and release-version parsing/comparison were tested with active preconditions. The user-requested GitHub metadata endpoint was checked live. Malformed tags and non-public release metadata were rejected.
- The local macOS app was rebuilt with hardened runtime, its signature verified, and the ZIP checked against every app-bundle file.
- No on-air session or physical audio/USB/PTT hardware validation was performed for this revision. The eight ignored tests remain explicit validation limits.

Repeat the main test suite with:

```sh
cargo test --locked --release --workspace --no-default-features --features xara/macos-app
```

The October 30 [security review](SECURITY_REVIEW.md) remains evidence for build 2; its artifact hashes and test counts are historical. Build 4 retains the private-listener and settings-permission defaults. The new user-requested GitHub metadata check is the explicitly disclosed network addition.

## Build 4 package

App: `target/macos/XARA.app`. ZIP: `target/release-assets/v0.1.3-macos.4/XARA-macOS-arm64.zip`. Apple Silicon arm64, macOS 12+ deployment target; ad-hoc signed with hardened runtime, not notarized.

```text
13801f43c0c547b82d9e328237386380b7455b1e192630d15642c5ddb8c99b06  XARA-macOS-arm64.zip
```
