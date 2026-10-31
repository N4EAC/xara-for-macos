"use strict";

const SN_WINDOW = 300;
const NOTABLE = new Set(["connected", "disconnected", "pending", "cancel_pending", "link_registered",
  "registered", "unencrypted_link", "line"]);
const NOTICES = { timed_out: "Link timed out", unable_to_connect: "Unable to connect" };

let control = false;
let tuning = false;
let driveHeld = false;
let mode = "hf";
let restartAfterSave = false;
let settingsDirty = false;
let latestTs = 0;
let snPoints = [];
let bpsPoints = [];
let lastStatusAt = 0;
let reportedPanelReady = false;
let settingsAvailable = false;
let settingsWritable = false;
const ledUntil = new Map();
let verbosity = Number(pref.get("xara.verbosity", "0"));
const log = new LogView($("log"), 2000, 600);
log.filter = (e) => e.level <= verbosity;
const rx = meter("rx"), tx = meter("tx");

$("verbosity").value = String(verbosity);
$("verbosity").addEventListener("change", (e) => {
  verbosity = Number(e.target.value);
  pref.set("xara.verbosity", String(verbosity));
  log.render();
});

const { conn, wf } = waterfallPage("xara.waterfall", handle);
const send = (cmd) => conn.send(cmd);

function handle(m) {
  switch (m.t) {
    case "hello":
      clearLiveDiagnostics();
      setText("diag-version", typeof m.version === "string" ? m.version : "—");
      setText("diag-protocol", count(m.v));
      setText("diag-replay", count(m.replay));
      setText("diag-meter-floor", fixed(m.meter_floor));
      setText("diag-drive-min", fixed(m.drive_min));
      setText("diag-drive-max", fixed(m.drive_max));
      setText("diag-tune-limit", fixed(m.tune_limit, 0));
      setText("diag-span", fixed(m.span, 0));
      for (const id of ["diag-backend", "diag-bins", "diag-rate", "diag-wf-floor", "diag-step"]) setText(id, "—");
      $("mode").textContent = m.mode.toUpperCase();
      settingsAvailable = false;
      $("set-fields").disabled = true;
      $("set-access").hidden = false;
      control = m.control;
      mode = m.mode;
      $("controls").hidden = !control;
      $("locked").hidden = !m.locked;
      $("drive").min = String(m.drive_min);
      $("drive").max = String(Number.isFinite(m.drive_max) ? m.drive_max : 0);
      meterFloor = m.meter_floor;
      if (control) {
        send("SETTINGS ?");
        send("DEVICES ?");
      }
      snPoints = [];
      bpsPoints = [];
      ledUntil.clear();
      log.reset();
      wf.reset(m.span);
      break;
    case "signal":
      pulse(m.name);
      break;
    case "data":
      pulse("DATA");
      pulse("RX");
      log.add({ ts: m.ts, cls: "data", text: `RX ${m.bytes} bytes · ${m.preview}${m.bytes > 256 ? " … [preview truncated]" : ""}`, level: 0 });
      break;
    case "waterfall":
      setText("diag-span", fixed(m.span, 0));
      setText("diag-bins", count(m.bins));
      setText("diag-rate", fixed(m.rate, 2));
      setText("diag-wf-floor", fixed(m.floor));
      setText("diag-step", fixed(m.step, 2));
      wf.config(m);
      break;
    case "status":
      status(m);
      break;
    case "host":
      if (m.kind === "sn") addSn(m.ts, m.sn);
      if (m.kind === "bitrate") { pulse("DATA"); if (!m.tx) pulse("RX"); }
      log.add({ ts: m.ts, cls: m.kind, text: m.line, level: NOTABLE.has(m.kind) ? 0 : 1 });
      break;
    case "notice":
      log.add({ ts: m.ts, cls: "notice", text: NOTICES[m.kind] || m.kind, level: 0 });
      break;
    case "settings":
      showSettings(m);
      break;
    case "devices":
      setText("diag-backend", typeof m.backend === "string" ? m.backend : "—");
      fillDevices("dev-capture", m.capture);
      fillDevices("dev-playback", m.playback);
      break;
    case "settings_result":
      settingsResult(m);
      break;
    case "control":
      if (m.what === "restart") {
        setMsg(m.ok ? "Restarting…" : "Restart refused: the modem is busy.", !m.ok);
      }
      if (m.what.startsWith("drive ")) $("drive-save").textContent = m.ok ? "Applied" : "Drive refused";
      if (m.what.startsWith("save drive")) $("drive-save").textContent = m.ok ? "Saved" : m.what;
      log.add({
        ts: m.ts ?? latestTs,
        cls: m.ok ? "control" : "notice",
        text: `UI ${m.what}${m.ok ? "" : " refused" + (m.reason ? `: ${m.reason}` : "")}`,
        level: 0,
      });
      break;
    case "trace":
      log.add({ ts: m.ts, cls: "trace", text: m.text, level: 2 });
      break;
  }
}

function status(s) {
  conn.alive();
  latestTs = s.ts;
  lastStatusAt = performance.now();
  wf.clock = s.ts;
  updateDiagnostics(s);
  $("received").textContent = Number.isSafeInteger(s.received_bytes) ? s.received_bytes.toLocaleString() : "—";
  log.setOffset(Date.now() / 1000 - s.ts);

  $("calls").textContent = s.calls.join(" ") || "—";
  $("mode").textContent = `${mode.toUpperCase()} ${/^\d+$/.test(s.bw) ? s.bw + " Hz" : s.bw}`;

  const st = $("state");
  st.textContent = s.state === "Listen" ? "Idle" : s.state;
  const linked = typeof s.connected === "boolean" ? s.connected : s.state !== "Listen";
  st.className = "state " + (s.state === "Listen" ? "idle" : linked ? "linked" : "calling");
  $("peer").textContent = s.peer || "—";
  document.querySelector(".session").classList.toggle("linked", linked);

  $("level").textContent = linked ? s.level : "-";
  $("bps").textContent = s.bitrate ? s.bitrate.bps : "-";
  $("dir").textContent = s.bitrate ? (s.bitrate.tx ? "bps TX" : "bps RX") : "";
  $("sn").textContent = s.sn === null ? "-" : s.sn.toFixed(1);
  $("afc").textContent = s.afc === null ? "-" : (s.afc > 0 ? "+" : "") + s.afc.toFixed(1);
  $("tries").textContent = `${s.attempts} / ${s.retries}`;
  $("buffer").textContent = s.buffer;

  wf.ptt = s.ptt;
  wf.setPassband(s.passband[0], s.passband[1]);
  $("controls").hidden = !control;
  if (control) controls(s);
  if (control && settingsAvailable) {
    $("set-fields").disabled = false;
    $("set-devices").disabled = false;
    $("set-save").disabled = !settingsWritable;
    $("set-reset").disabled = !settingsWritable;
  }
  $("ptt").classList.toggle("on", s.ptt);
  $("busy").classList.toggle("on", s.busy);
  for (const [id, samples, name] of [["clip", s.rx.clip, "RX"], ["tx-clip", s.tx.clip, "TX"]]) {
    $(id).classList.toggle("on", samples > 0);
    $(id).setAttribute("aria-label", `${name} clipping ${samples > 0 ? "active" : "inactive"}`);
  }
  rx(s.rx);
  tx(s.tx);
  if (s.bitrate && Number.isFinite(s.bitrate.bps)) bpsPoints.push({ ts: s.ts, v: s.bitrate.bps });
  while (bpsPoints.length && bpsPoints[0].ts < s.ts - SN_WINDOW) bpsPoints.shift();
  setQuality("sn-meter", s.sn, -10, 30);
  setQuality("afc-meter", s.afc, -100, 100);
  // PTT is authoritative; RX means receive-side decoder activity, not ambient audio.
  setLed("TX", s.ptt);
  setLed("RX", !s.ptt && (s.state === "Receiving" || (ledUntil.get("RX") || 0) > performance.now()));
  drawSn();
  if (!reportedPanelReady && window.webkit?.messageHandlers?.panelReady) {
    reportedPanelReady = true;
    requestAnimationFrame(() => window.webkit?.messageHandlers?.panelReady?.postMessage("ready"));
  }
}

function controls(s) {
  tuning = s.tuning;
  const t = $("tune");
  t.classList.toggle("on", s.tuning);
  t.textContent = s.tuning ? `Stop tune (${Math.ceil(s.tune_left)} s)` : "Tune";
  t.disabled = window.xaraNativeTransmit === false || (!s.tuning && (s.state !== "Listen" || s.ptt));
  $("abort").disabled = s.state === "Listen" && !s.tuning;
  if (!driveHeld) {
    $("drive").value = String(Math.round(s.drive));
    $("drive-val").textContent = `${s.drive > 0 ? "+" : ""}${s.drive.toFixed(Number.isInteger(s.drive) ? 0 : 1)} dB`;
  }
}

$("tune").addEventListener("click", () => send(tuning ? "TUNE OFF" : "TUNE ON"));
$("abort").addEventListener("click", () => send("ABORT"));
$("drive").addEventListener("pointerdown", () => { driveHeld = true; });
$("drive").addEventListener("input", (e) => {
  driveHeld = true;
  $("drive-val").textContent = `${Number(e.target.value) > 0 ? "+" : ""}${e.target.value} dB`;
});
$("drive").addEventListener("change", (e) => {
  $("drive-save").textContent = "Saving…";
  send(`DRIVE ${e.target.value}`);
  setTimeout(() => { driveHeld = false; }, 300);
});

function setMsg(text, bad) {
  const p = $("set-msg");
  p.textContent = text;
  p.classList.toggle("bad", !!bad);
}

function showSettings(m) {
  settingsAvailable = true;
  settingsWritable = m.available;
  $("set-fields").disabled = !control;
  $("set-devices").disabled = !control;
  $("set-access").hidden = true;
  $("set-path").textContent = m.available ? m.path : "";
  const err = $("set-error");
  err.hidden = !m.error;
  err.textContent = m.error || "";
  $("set-hf").hidden = mode !== "hf";
  $("set-fm").hidden = mode !== "fm";
  $("set-save").disabled = !m.available;
  $("set-reset").disabled = !m.available;
  if (!m.available) setMsg("Saving settings needs --state-dir, or running as a systemd service.", false);
  if (settingsDirty) return;
  for (const el of $("set-form").querySelectorAll("input, select")) {
    el.value = m.saved[el.name] || "";
    if (el.tagName === "INPUT") el.placeholder = m.values[el.name] || "";
    if (el.tagName === "SELECT") el.options[0].textContent = `${window.xaraEmbedded ? "app default" : "command line"} (${m.values[el.name] || "default"})`;
  }
}

function fillDevices(id, list) {
  $(id).replaceChildren(...list.map((d) => {
    const o = document.createElement("option");
    o.value = d.name;
    o.label = d.desc;
    return o;
  }));
}

function settingsResult(m) {
  setMsg(m.ok ? (m.what === "save" ? "Saved." : m.text) : `${m.what}: ${m.text}`, !m.ok);
  if (m.what !== "save" && m.what !== "reset") return;
  if (m.ok) {
    settingsDirty = false;
    if (restartAfterSave) send("RESTART");
  }
  restartAfterSave = false;
}

$("set-form").addEventListener("input", () => { settingsDirty = true; });
$("set-save").addEventListener("click", () => {
  if (!$("set-form").reportValidity() || !conn.open || !control) return;
  const lines = [...$("set-form").querySelectorAll("input, select")]
    .filter((el) => !el.closest("[hidden]"))
    .map((el) => `${el.name} ${el.value.trim()}`);
  restartAfterSave = true;
  send(["SETTINGS SAVE", ...lines].join("\n"));
});
$("set-reset").addEventListener("click", () => {
  restartAfterSave = true;
  send("SETTINGS RESET");
});
$("set-devices").addEventListener("click", () => send("DEVICES ?"));

function addSn(ts, sn) {
  snPoints.push({ ts, v: sn });
  while (snPoints.length && snPoints[0].ts < ts - SN_WINDOW) snPoints.shift();
}

function drawSn() {
  lineChart($("sn-chart"), snPoints, latestTs, SN_WINDOW, -10, 30);
  lineChart($("bps-chart"), bpsPoints, latestTs, SN_WINDOW, 0, Math.max(1000, ...bpsPoints.map(p => p.v)), 1000);
}

window.addEventListener("resize", drawSn);
wf.drawAxis();


function setLed(name, active) {
  const el = $("led-" + name.toLowerCase());
  if (!el) return;
  el.classList.toggle("on", active);
  el.setAttribute("aria-label", `${name} ${active ? "active" : "inactive"}`);
}
function pulse(name) {
  ledUntil.set(name, performance.now() + 750);
  setLed(name, true);
}
function setQuality(id, value, min, max) {
  const el = $(id), valid = Number.isFinite(value);
  const pct = valid ? Math.max(0, Math.min(100, (value - min) / (max - min) * 100)) : 0;
  el.setAttribute("aria-valuetext", valid ? String(value) : "Unavailable");
  if (valid) el.setAttribute("aria-valuenow", Math.max(min, Math.min(max, value)));
  else el.removeAttribute("aria-valuenow");
  const indicator = el.querySelector("i");
  if (id === "afc-meter") {
    indicator.style.left = `calc(${pct}% - 1px)`;
    indicator.style.opacity = valid ? "1" : "0";
  } else indicator.style.width = `${pct}%`;
}

const tabs = [...document.querySelectorAll('.tabs [role="tab"]')];
function selectTab(index) {
  tabs.forEach((tab, i) => {
    tab.setAttribute("aria-selected", String(i === index));
    tab.tabIndex = i === index ? 0 : -1;
    $(tab.getAttribute("aria-controls")).hidden = i !== index;
  });
  if (index === 0) { drawSn(); wf.drawAxis(); }
}
tabs.forEach((tab, i) => {
  tab.addEventListener("click", () => selectTab(i));
  tab.addEventListener("keydown", e => {
    if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(e.key)) return;
    e.preventDefault();
    const next = e.key === "Home" ? 0 : e.key === "End" ? tabs.length - 1 :
      (i + (e.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length;
    selectTab(next); tabs[next].focus();
  });
});
$("log-clear").addEventListener("click", () => log.clear());
$("log-export").addEventListener("click", () => {
  const content = log.entries.filter(log.filter).map(e => `${log.clock(e.ts)} ${e.text}`).join("\n");
  const url = URL.createObjectURL(new Blob([content], { type: "text/plain" }));
  const a = document.createElement("a"); a.href = url; a.download = "xara-monitor.txt"; a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
});
new MutationObserver(() => { $("log-empty").hidden = $("log").childElementCount > 0; })
  .observe($("log"), { childList: true });
const originalRow = wf.row.bind(wf);
wf.row = b => { originalRow(b); if (b[0] === wf.rowType && wf.on) $("wf-empty").hidden = true; };
setInterval(() => {
  for (const [name, until] of ledUntil) if (performance.now() > until) { setLed(name, false); ledUntil.delete(name); }
  if (conn.open && lastStatusAt && performance.now() - lastStatusAt < 2500) return;
  for (const name of ["DATA", "ACK", "NACK", "REQ", "QRT", "BREAK", "START", "RX", "TX"]) setLed(name, false);
  $("state").textContent = "Telemetry offline";
  $("state").className = "state idle";
  document.querySelector(".session").classList.remove("linked");
  for (const id of ["ptt", "busy", "clip", "tx-clip"]) $(id).classList.remove("on");
  clearLiveDiagnostics();
  for (const id of ["bps", "sn", "afc"]) $(id).textContent = "—";
  $("dir").textContent = "bits / second";
  setQuality("sn-meter", null, -10, 30);
  setQuality("afc-meter", null, -100, 100);
  rx({ rms: -120, peak: -120 }); tx({ rms: -120, peak: -120 });
  for (const value of document.querySelectorAll(".meter .val")) value.textContent = "—";
  $("controls").hidden = true;
  $("set-fields").disabled = true;
  for (const id of ["set-save", "set-reset", "set-devices"]) $(id).disabled = true;
}, 250);

if (window.xaraEmbedded) {
  $("set-reset").textContent = "Use app defaults";
  document.querySelector(".settings-bottom .hint").textContent = "Empty fields inherit the app configuration. Changes restart the built-in modem while idle. Audio devices are connected to this Mac.";
}


function setText(ids, value) {
  for (const id of Array.isArray(ids) ? ids : [ids]) $(id).textContent = value;
}
function fixed(value, digits = 1) {
  return Number.isFinite(value) ? value.toLocaleString(undefined, { minimumFractionDigits: digits, maximumFractionDigits: digits }) : "—";
}
function count(value) {
  return Number.isSafeInteger(value) && value >= 0 ? value.toLocaleString() : "—";
}
function updateDiagnostics(s) {
  setText(["dsp-time", "diag-time"], fixed(s.ts, 2));
  setText(["telemetry-drops", "diag-drops"], count(s.dropped));
  for (const id of ["telemetry-drops", "diag-drops"]) $(id).classList.toggle("metric-warning", s.dropped > 0);
  setText("frame-level", count(s.bitrate?.level));
  const band = Array.isArray(s.passband) ? s.passband : [];
  setText(["passband-low", "diag-passband-low"], fixed(band[0], 1));
  setText(["passband-high", "diag-passband-high"], fixed(band[1], 1));
  for (const [prefix, meter] of [["rx", s.rx], ["tx", s.tx]]) {
    const peak = Number.isFinite(meter?.peak) && meter.peak <= meterFloor ? `≤ ${fixed(meterFloor)}` : fixed(meter?.peak);
    setText(`${prefix}-peak`, peak);
    setText(`${prefix}-clips`, count(meter?.clip));
    $(`${prefix}-clips`).classList.toggle("metric-warning", meter?.clip > 0);
  }
}
function clearLiveDiagnostics() {
  for (const id of ["dsp-time", "diag-time", "telemetry-drops", "diag-drops", "frame-level", "passband-low", "passband-high", "diag-passband-low", "diag-passband-high", "rx-peak", "tx-peak", "rx-clips", "tx-clips"]) {
    setText(id, "—");
    $(id).classList.remove("metric-warning");
  }
  $("clip").setAttribute("aria-label", "RX clipping inactive");
  $("tx-clip").setAttribute("aria-label", "TX clipping inactive");
}
