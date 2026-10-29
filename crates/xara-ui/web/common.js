"use strict";

const $ = (id) => document.getElementById(id);
const FLOOR = -60;
let meterFloor = -Infinity;

for (const el of document.querySelectorAll(".scale")) {
  el.replaceChildren(...[0, 1, 2, 3, 4].map((i) => {
    const s = document.createElement("span");
    const db = FLOOR * (4 - i) / 4;
    s.textContent = db ? `\u2212${-db}` : "0 dBFS";
    return s;
  }));
}

const pref = {
  get(k, d) {
    try {
      const v = localStorage.getItem(k);
      return v === null ? d : v;
    } catch (e) {
      return d;
    }
  },
  set(k, v) {
    try { localStorage.setItem(k, v); } catch (e) {}
  },
};

function setConn(text, cls) {
  const c = $("conn");
  c.textContent = text;
  c.className = "conn " + cls;
}

function connection(on) {
  let ws = null, backoff = 1000, lastStatus = 0;
  const conn = {
    send(cmd) {
      if (ws && ws.readyState === WebSocket.OPEN) ws.send(cmd);
    },
    get open() {
      return !!ws && ws.readyState === WebSocket.OPEN;
    },
    alive() {
      lastStatus = performance.now();
      setConn("live", "up");
    },
  };
  function connect() {
    const proto = location.protocol === "https:" ? "wss:" : "ws:";
    ws = new WebSocket(`${proto}//${location.host}/ws`);
    ws.binaryType = "arraybuffer";
    ws.onopen = () => {
      backoff = 1000;
      on.open();
    };
    ws.onmessage = (m) => {
      if (typeof m.data !== "string") {
        on.binary(new Uint8Array(m.data));
        return;
      }
      let msg;
      try { msg = JSON.parse(m.data); } catch (e) { return; }
      on.message(msg);
    };
    ws.onclose = () => {
      setConn("offline", "");
      setTimeout(connect, backoff);
      backoff = Math.min(backoff * 2, 10000);
    };
  }
  setInterval(() => {
    if (conn.open && performance.now() - lastStatus > 2000) setConn("stale", "stale");
  }, 1000);
  connect();
  return conn;
}

function waterfallPage(key, message) {
  let wf = null;
  const conn = connection({
    open: () => wf.subscribe(),
    binary: (b) => wf.row(b),
    message,
  });
  wf = new Waterfall(conn, key);
  return { conn, wf };
}

function meter(id) {
  const el = $(id);
  const hold = { db: FLOOR, at: 0 };
  const pct = (db) => Math.max(0, Math.min(100, (db - FLOOR) / -FLOOR * 100));
  return (m) => {
    el.querySelector(".fill").style.clipPath = `inset(0 ${100 - pct(m.rms)}% 0 0)`;
    const now = performance.now();
    if (m.peak >= hold.db || now - hold.at > 1500) {
      hold.db = m.peak;
      hold.at = now;
    }
    el.querySelector(".hold").style.left = `calc(${pct(hold.db)}% - 2px)`;
    el.querySelector(".val").textContent = m.rms <= meterFloor ? "-" : `${m.rms.toFixed(1)} dBFS`;
  };
}

function palette(stops) {
  const lut = new Uint8ClampedArray(256 * 3);
  for (let i = 0; i < 256; i++) {
    const x = i / 255;
    let k = 1;
    while (stops[k][0] < x) k++;
    const [x0, ...a] = stops[k - 1], [x1, ...b] = stops[k];
    const f = (x - x0) / (x1 - x0);
    for (let c = 0; c < 3; c++) lut[i * 3 + c] = a[c] + (b[c] - a[c]) * f;
  }
  return lut;
}

const PALETTE = palette([[0, 0, 0, 0], [0.22, 0, 0, 120], [0.45, 0, 120, 220], [0.62, 0, 220, 200],
  [0.75, 240, 230, 0], [0.88, 255, 70, 0], [1, 255, 255, 255]]);

class Waterfall {
  constructor(conn, key) {
    this.conn = conn;
    this.key = key;
    this.span = null;
    this.rowType = null;
    this.step = 0.5;
    this.noise = null;
    this.passband = null;
    this.ptt = false;
    this.img = null;
    this.clock = 0;
    this.stamps = [];
    this.on = pref.get(key, "on") !== "off";
    this.canvas = $("wf");
    $("wf-on").checked = this.on;
    $("wf-on").addEventListener("change", (e) => {
      this.on = e.target.checked;
      pref.set(key, this.on ? "on" : "off");
      this.subscribe();
    });
    document.addEventListener("visibilitychange", () => this.subscribe());
    $("wf-wrap").addEventListener("mousemove", (e) => {
      if (!this.span) return;
      const r = e.currentTarget.getBoundingClientRect();
      $("wf-cursor").textContent = `${Math.round((e.clientX - r.left) / r.width * this.span)} Hz`;
    });
    $("wf-wrap").addEventListener("mouseleave", () => { $("wf-cursor").textContent = ""; });
    window.addEventListener("resize", () => this.drawAxis());
  }

  subscribe() {
    document.querySelector(".wf").classList.toggle("off", !this.on);
    this.conn.send(this.on && !document.hidden ? "WATERFALL ON" : "WATERFALL OFF");
  }

  reset(span) {
    this.noise = null;
    this.stamps = [];
    this.span = span;
    this.drawAxis();
  }

  config(m) {
    this.rowType = m.row;
    this.span = m.span;
    this.step = m.step;
    this.canvas.width = m.bins;
    this.img = null;
    this.drawAxis();
  }

  setPassband(lo, hi) {
    const p = this.passband;
    if (lo === null ? p === null : p && p[0] === lo && p[1] === hi) return;
    this.passband = lo === null ? null : [lo, hi];
    this.drawAxis();
  }

  row(b) {
    if (b[0] !== this.rowType || !this.on) return;
    const n = b[2] | (b[3] << 8);
    const v = b.subarray(4, 4 + n);
    const p20 = Uint8Array.from(v).sort()[Math.floor(n * 0.2)];
    this.noise = this.noise === null ? p20 : this.noise * 0.95 + p20 * 0.05;
    const lo = this.noise - 6 / this.step, range = 50 / this.step;
    const c = this.canvas;
    const g = c.getContext("2d");
    if (c.width !== n) c.width = n;
    if (!this.img || this.img.width !== n) this.img = g.createImageData(n, 1);
    g.drawImage(c, 0, 0, n, c.height - 1, 0, 1, n, c.height - 1);
    const d = this.img.data;
    for (let i = 0; i < n; i++) {
      const k = Math.max(0, Math.min(255, Math.round((v[i] - lo) / range * 255))) * 3;
      d[i * 4] = PALETTE[k];
      d[i * 4 + 1] = PALETTE[k + 1];
      d[i * 4 + 2] = PALETTE[k + 2];
      d[i * 4 + 3] = 255;
    }
    if (this.ptt) {
      for (let i = 0; i < 4; i++) d.set([255, 40, 40, 255], i * 4);
    }
    g.putImageData(this.img, 0, 0);
    this.stamps.unshift(this.clock);
    if (this.stamps.length > c.height) this.stamps.length = c.height;
  }

  mark(ts, loHz, hiHz, color) {
    const y = this.stamps.findIndex((s) => s <= ts);
    if (y < 0 || !this.on) return false;
    const c = this.canvas, g = c.getContext("2d");
    const x0 = Math.max(0, Math.round(loHz / this.span * c.width));
    const x1 = Math.min(c.width, Math.round(hiHz / this.span * c.width));
    g.fillStyle = color;
    g.fillRect(x0, y, Math.max(1, x1 - x0), 2);
    return true;
  }

  drawAxis() {
    if (!this.span) return;
    const c = $("wf-axis");
    const dpr = window.devicePixelRatio || 1;
    const w = c.clientWidth, h = c.clientHeight;
    c.width = Math.round(w * dpr);
    c.height = Math.round(h * dpr);
    const g = c.getContext("2d");
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    const css = getComputedStyle(document.documentElement);
    const band = $("wf-band");
    if (this.passband) {
      const [lo, hi] = this.passband;
      band.style.display = "block";
      band.style.left = `${lo / this.span * 100}%`;
      band.style.width = `${(hi - lo) / this.span * 100}%`;
      g.fillStyle = css.getPropertyValue("--band");
      g.globalAlpha = 0.35;
      g.fillRect(lo / this.span * w, 0, (hi - lo) / this.span * w, 4);
      g.globalAlpha = 1;
    } else {
      band.style.display = "none";
    }
    const step = this.span > 4000 ? 1000 : 500;
    g.font = `10px ${css.getPropertyValue("--mono").trim()}`;
    g.fillStyle = css.getPropertyValue("--dim");
    g.strokeStyle = css.getPropertyValue("--dim");
    g.textBaseline = "top";
    for (let f = 0; f <= this.span; f += step / 2) {
      const x = Math.round(f / this.span * (w - 1)) + 0.5;
      const major = f % step === 0;
      g.beginPath();
      g.moveTo(x, 0);
      g.lineTo(x, major ? 7 : 4);
      g.stroke();
      if (major) {
        g.textAlign = f === 0 ? "left" : f >= this.span ? "right" : "center";
        g.fillText(String(f), x, 9);
      }
    }
  }
}

class LogView {
  constructor(el, keep, show) {
    this.el = el;
    this.keep = keep;
    this.show = show;
    this.entries = [];
    this.offset = null;
    this.filter = () => true;
  }

  clear() {
    this.entries = [];
    this.render();
  }

  reset() {
    this.offset = null;
    this.clear();
  }

  setOffset(offset) {
    const first = this.offset === null;
    this.offset = offset;
    if (first) this.render();
  }

  clock(ts) {
    if (this.offset === null) return "";
    return new Date((ts + this.offset) * 1000).toLocaleTimeString([], { hour12: false });
  }

  row(e) {
    const li = document.createElement("li");
    li.className = e.cls;
    const t = document.createElement("time");
    t.textContent = this.clock(e.ts);
    const s = document.createElement("span");
    s.textContent = e.text;
    li.append(t, s);
    return li;
  }

  add(e) {
    this.entries.push(e);
    if (this.entries.length > this.keep) this.entries.splice(0, this.entries.length - this.keep);
    if (this.offset !== null && this.filter(e)) {
      const log = this.el;
      const stick = log.scrollTop + log.clientHeight >= log.scrollHeight - 4;
      log.appendChild(this.row(e));
      while (log.childElementCount > this.show) log.firstElementChild.remove();
      if (stick) log.scrollTop = log.scrollHeight;
    }
  }

  addOrCollapse(e) {
    const last = this.entries[this.entries.length - 1];
    if (!e.key || !last || last.key !== e.key) {
      this.add(e);
      return;
    }
    last.base ??= last.text;
    last.count = (last.count ?? 1) + 1;
    last.ts = e.ts;
    last.text = `${last.base}  ×${last.count}`;
    if (this.offset !== null && this.filter(last) && this.el.lastElementChild) {
      this.el.lastElementChild.replaceWith(this.row(last));
    }
  }

  render() {
    const log = this.el;
    log.replaceChildren();
    if (this.offset === null) return;
    const shown = this.entries.filter(this.filter).slice(-this.show);
    log.append(...shown.map((e) => this.row(e)));
    log.scrollTop = log.scrollHeight;
  }
}

function lineChart(c, points, now, span, lo, hi, tick = 10) {
  const dpr = window.devicePixelRatio || 1;
  const w = c.clientWidth, h = c.clientHeight;
  if (c.width !== Math.round(w * dpr) || c.height !== Math.round(h * dpr)) {
    c.width = Math.round(w * dpr);
    c.height = Math.round(h * dpr);
  }
  const g = c.getContext("2d");
  g.setTransform(dpr, 0, 0, dpr, 0, 0);
  g.clearRect(0, 0, w, h);
  const css = getComputedStyle(document.documentElement);
  const dim = css.getPropertyValue("--dim"), line = css.getPropertyValue("--line"), accent = css.getPropertyValue("--accent");
  lo = Math.min(lo, ...points.map((p) => Math.floor(p.v / 10) * 10));
  hi = Math.max(hi, ...points.map((p) => Math.ceil(p.v / 10) * 10));
  g.font = `10px ${css.getPropertyValue("--mono").trim()}`;
  const left = Math.max(30, Math.ceil(Math.max(g.measureText(String(lo)).width, g.measureText(String(hi)).width)) + 6), bottom = h - 4;
  const x = (ts) => left + (w - left - 4) * (1 - (now - ts) / span);
  const y = (v) => bottom - (bottom - 6) * (v - lo) / (hi - lo);
  g.textBaseline = "middle";
  for (let v = lo; v <= hi; v += tick) {
    g.strokeStyle = line;
    g.beginPath();
    g.moveTo(left, y(v));
    g.lineTo(w, y(v));
    g.stroke();
    g.fillStyle = dim;
    g.fillText(String(v), 2, y(v));
  }
  if (!points.length) return;
  g.strokeStyle = accent;
  g.fillStyle = accent;
  g.lineWidth = 1.5;
  g.beginPath();
  points.forEach((p, i) => (i ? g.lineTo(x(p.ts), y(p.v)) : g.moveTo(x(p.ts), y(p.v))));
  g.stroke();
  for (const p of points) {
    g.beginPath();
    g.arc(x(p.ts), y(p.v), 2.5, 0, 2 * Math.PI);
    g.fill();
  }
}
