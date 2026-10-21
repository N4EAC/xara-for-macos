"use strict";

const BEHIND_S = 5;
const PAYLOAD_KEEP = 65536;
const COLORS = Object.fromEntries(["caller", "answerer", "ok", "bad", "unproto"].map((k) =>
  [k, getComputedStyle(document.documentElement).getPropertyValue(`--k-${k}`).trim()]));
const GROUP = {
  ctrl: "ctrl", data: "data", payload: "payload", header: "payload", unproto: "unproto", kiss: "unproto",
  cq: "unproto", learned: "notes", dropped: "notes", link_end: "notes", blind: "notes", summary: "notes",
};

const log = new LogView($("log"), 3000, 800);
const rx = meter("rx");
const GROUPS = ["ctrl", "data", "payload", "unproto", "notes"];
const groups = new Set(savedGroups());
let collapse = pref.get("xara-monitor.collapse", "on") !== "off";
let hex = pref.get("xara-monitor.hex", "off") === "on";
let passband = null;
let unproto = null;
let linkStart = null;
let linkName = "";
let owner = null;
let heardSeen = null;
const payload = [[], []];

function savedGroups() {
  try {
    const g = JSON.parse(pref.get("xara-monitor.groups", JSON.stringify(GROUPS)));
    if (Array.isArray(g)) return g;
  } catch (e) {}
  return GROUPS;
}

log.filter = (e) => groups.has(e.group);

for (const box of $("chips").querySelectorAll("input[value]")) {
  box.checked = groups.has(box.value);
  box.addEventListener("change", () => {
    if (box.checked) groups.add(box.value); else groups.delete(box.value);
    pref.set("xara-monitor.groups", JSON.stringify([...groups]));
    log.render();
  });
}
$("collapse").checked = collapse;
$("collapse").addEventListener("change", (e) => {
  collapse = e.target.checked;
  pref.set("xara-monitor.collapse", collapse ? "on" : "off");
});
$("pl-hex").checked = hex;
$("pl-hex").addEventListener("change", (e) => {
  hex = e.target.checked;
  pref.set("xara-monitor.hex", hex ? "on" : "off");
  showPayload(0);
  showPayload(1);
});

const { conn, wf } = waterfallPage("xara-monitor.waterfall", handle);

function handle(m) {
  switch (m.t) {
    case "hello":
      meterFloor = m.meter_floor;
      log.reset();
      wf.reset(m.span);
      clearPayload();
      linkStart = null;
      owner = null;
      break;
    case "waterfall":
      wf.config(m);
      break;
    case "status":
      status(m);
      break;
    default:
      if (GROUP[m.t]) record(m);
  }
}

function record(r) {
  const e = { ts: r.ts, group: GROUP[r.t], cls: r.t, text: r.text.trim(), key: null };
  if (r.t === "ctrl") {
    e.cls += " " + r.sender;
    e.key = `c|${r.sender}|${r.code}|${r.key}`;
    if (passband) wf.mark(r.ts, passband[0] + r.df, passband[1] + r.df, COLORS[r.sender]);
  } else if (r.t === "unproto") {
    e.key = `u|${r.code}`;
    if (unproto) wf.mark(r.ts, unproto[0], unproto[1], COLORS.unproto);
  } else if (r.t === "data") {
    const ok = r.frames.includes("+");
    if (!ok) e.cls += " bad";
    if (passband) wf.mark(r.ts, passband[0] + r.afc, passband[1] + r.afc, ok ? COLORS.ok : COLORS.bad);
  } else if (r.t === "payload") {
    if (r.status !== "data") e.cls += r.status === "repeat" ? "" : " bad";
    else if (r.sender === "caller" || r.sender === "answerer") addPayload(r.sender === "caller" ? 0 : 1, r.data);
  }
  if (collapse) log.addOrCollapse(e); else log.add(e);
}

function status(s) {
  conn.alive();
  wf.clock = s.ts;
  log.setOffset(s.epoch ?? Date.now() / 1000 - s.ts);
  $("mode").textContent = `HF ${s.bw} Hz`;
  $("calls").textContent = s.source + (s.epoch ? ` since ${new Date(s.epoch * 1000).toLocaleTimeString([], { hour12: false })}` : "");
  passband = s.passband;
  unproto = s.unproto;
  wf.setPassband(s.passband[0], s.passband[1]);
  rx(s.rx);
  $("clip").classList.toggle("on", s.rx.clip > 0);
  $("behind").classList.toggle("on", s.lag > BEHIND_S);
  $("ended").classList.toggle("on", s.ended);
  $("lag").textContent = s.lag.toFixed(1);
  const d = s.decoder;
  if (!d) return;
  const st = d.stats;
  $("frames").textContent = `${st.frames_ok} / ${st.frames}`;
  $("ctrl").textContent = `${st.ctrl} (${st.ctrl_blind} blind)`;
  $("searches").textContent = String(st.searches);
  link(d.link, d.decoded);
  keys(d.keys);
  const same = heardSeen !== null && d.heard.length === heardSeen.length &&
    d.heard.every((h, i) => ["name", "last", "count"].every((k) => h[k] === heardSeen[i][k]));
  if (!same) {
    heardSeen = d.heard;
    heard(d.heard);
  }
}

function link(l, now) {
  const state = $("state");
  if (!l) {
    state.textContent = "Quiet";
    state.className = "state idle";
    $("since").textContent = "";
    $("bytes-ca").textContent = "-";
    $("bytes-ac").textContent = "-";
    return;
  }
  linkStart = l.start;
  const name = `${l.caller ?? "?"} → ${l.answerer ?? "?"}`;
  linkName = name;
  if (owner === linkStart) $("pl-link").textContent = name;
  state.textContent = name;
  state.className = "state linked";
  $("since").textContent = `for ${Math.max(0, Math.round(now - l.start))} s`;
  $("bytes-ca").textContent = String(l.bytes[0]);
  $("bytes-ac").textContent = String(l.bytes[1]);
}

function cell(text, cls) {
  const td = document.createElement("td");
  td.textContent = text;
  if (cls) td.className = cls;
  return td;
}

function heard(list) {
  $("heard-none").hidden = list.length > 0;
  $("heard").replaceChildren(...list.map((s) => {
    const tr = document.createElement("tr");
    const as = s.kinds.join(", ") + (s.bw ? ` ${s.bw}` : "");
    tr.append(cell(s.name, "mono"), cell(as), cell(log.clock(s.last)), cell(String(s.count), "n"));
    return tr;
  }));
}

function keys(list) {
  $("keys-none").hidden = list.length > 0;
  $("keys").replaceChildren(...list.map((k) => {
    const tr = document.createElement("tr");
    const exp = k.expires === null ? "-" : `${Math.ceil(k.expires / 60)} min`;
    const last = k.origin === "configured" && k.last === 0 ? "" : log.clock(k.last);
    tr.append(cell(k.key_name, "mono"), cell(k.key_kind), cell(k.origin), cell(last), cell(exp, "n"));
    return tr;
  }));
}

function clearPayload() {
  payload[0] = [];
  payload[1] = [];
  $("pl-link").textContent = "";
  showPayload(0);
  showPayload(1);
}

function addPayload(i, data) {
  if (owner !== linkStart) {
    clearPayload();
    owner = linkStart;
    $("pl-link").textContent = linkName;
  }
  payload[i].push(data);
  let n = payload[i].reduce((a, s) => a + s.length, 0);
  while (n > PAYLOAD_KEEP && payload[i].length > 1) n -= payload[i].shift().length;
  showPayload(i);
}

function asText(s) {
  return s.replace(/\r\n?/g, "\n").replace(/[^\n\t\x20-\x7e\xa0-\xff]/g, "·");
}

function asHex(s) {
  const lines = [];
  for (let o = 0; o < s.length; o += 16) {
    const chunk = s.slice(o, o + 16);
    const hx = [...chunk].map((c) => c.charCodeAt(0).toString(16).padStart(2, "0")).join(" ");
    const tx = [...chunk].map((c) => (c >= " " && c <= "~" ? c : ".")).join("");
    lines.push(`${o.toString(16).padStart(6, "0")}  ${hx.padEnd(47)}  ${tx}`);
  }
  return lines.join("\n");
}

function showPayload(i) {
  const s = payload[i].join("");
  const pre = $(`pl${i}`);
  const stick = pre.scrollTop + pre.clientHeight >= pre.scrollHeight - 4;
  pre.textContent = hex ? asHex(s) : asText(s);
  $(`pl-n${i}`).textContent = s.length ? `${s.length} B` : "";
  if (stick) pre.scrollTop = pre.scrollHeight;
}

wf.drawAxis();
