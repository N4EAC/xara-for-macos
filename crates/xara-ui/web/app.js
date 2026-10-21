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
      $("mode").textContent = m.mode.toUpperCase();
      control = m.control;
      mode = m.mode;
      $("controls").hidden = !control;
      $("locked").hidden = !m.locked;
      $("drive").min = String(m.drive_min);
      meterFloor = m.meter_floor;
      if (control) {
        send("SETTINGS ?");
        send("DEVICES ?");
      }
      snPoints = [];
      log.reset();
      wf.reset(m.span);
      break;
    case "waterfall":
      wf.config(m);
      break;
    case "status":
      status(m);
      break;
    case "host":
      if (m.kind === "sn") addSn(m.ts, m.sn);
      log.add({ ts: m.ts, cls: m.kind, text: m.line, level: NOTABLE.has(m.kind) ? 0 : 1 });
      break;
    case "notice":
      log.add({ ts: m.ts, cls: "notice", text: NOTICES[m.kind] || m.kind, level: 0 });
      break;
    case "settings":
      showSettings(m);
      break;
    case "devices":
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
  log.setOffset(Date.now() / 1000 - s.ts);

  $("calls").textContent = s.calls.join(" ");
  $("mode").textContent = `${mode.toUpperCase()} ${/^\d+$/.test(s.bw) ? s.bw + " Hz" : s.bw}`;

  const st = $("state");
  st.textContent = s.state === "Listen" ? "Idle" : s.state;
  st.className = "state " + (s.state === "Listen" ? "idle" : s.state === "Calling" ? "calling" : "linked");
  $("peer").textContent = s.peer ? `⇄ ${s.peer}` : "";

  const linked = s.state !== "Listen";
  $("level").textContent = linked ? s.level : "-";
  $("bps").textContent = s.bitrate ? s.bitrate.bps : "-";
  $("dir").textContent = s.bitrate ? (s.bitrate.tx ? "bps TX" : "bps RX") : "";
  $("sn").textContent = s.sn === null ? "-" : s.sn.toFixed(1);
  $("afc").textContent = s.afc === null ? "-" : (s.afc > 0 ? "+" : "") + s.afc.toFixed(1);
  $("tries").textContent = `${s.attempts} / ${s.retries}`;
  $("buffer").textContent = s.buffer;

  wf.ptt = s.ptt;
  wf.setPassband(s.passband[0], s.passband[1]);
  if (control) controls(s);
  $("ptt").classList.toggle("on", s.ptt);
  $("busy").classList.toggle("on", s.busy);
  $("clip").classList.toggle("on", s.rx.clip > 0);
  rx(s.rx);
  tx(s.tx);
  drawSn();
}

function controls(s) {
  tuning = s.tuning;
  const t = $("tune");
  t.classList.toggle("on", s.tuning);
  t.textContent = s.tuning ? `Stop tune (${Math.ceil(s.tune_left)} s)` : "Tune";
  t.disabled = !s.tuning && (s.state !== "Listen" || s.ptt);
  $("abort").disabled = s.state === "Listen" && !s.tuning;
  if (!driveHeld) {
    $("drive").value = String(Math.round(s.drive));
    $("drive-val").textContent = `${s.drive.toFixed(0)} dB`;
  }
}

$("tune").addEventListener("click", () => send(tuning ? "TUNE OFF" : "TUNE ON"));
$("abort").addEventListener("click", () => send("ABORT"));
$("drive").addEventListener("pointerdown", () => { driveHeld = true; });
$("drive").addEventListener("input", (e) => {
  driveHeld = true;
  $("drive-val").textContent = `${e.target.value} dB`;
});
$("drive").addEventListener("change", (e) => {
  send(`DRIVE ${e.target.value}`);
  setTimeout(() => { driveHeld = false; }, 300);
});

function setMsg(text, bad) {
  const p = $("set-msg");
  p.textContent = text;
  p.classList.toggle("bad", !!bad);
}

function showSettings(m) {
  $("settings").hidden = false;
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
  for (const el of $("set-form").elements) {
    el.value = m.saved[el.name] || "";
    if (el.tagName === "INPUT") el.placeholder = m.values[el.name] || "";
    if (el.tagName === "SELECT") el.options[0].textContent = `command line (${m.values[el.name] || "default"})`;
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
  const lines = [...$("set-form").elements]
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
}

window.addEventListener("resize", drawSn);
wf.drawAxis();
