"use strict";
/* Kitsune page: the voice notebook, driven by /api/kitsune on this core. */
const $ = (id) => document.getElementById(id);
const COPPER = "#e48a46";
const MOOD = {
  idle: { pulse_hz: 0.25, turbulence: 0.3, glow: 0.2 },
  listening: { pulse_hz: 0.9, turbulence: 0.8, glow: 0.45 },
  working: { pulse_hz: 1.6, turbulence: 1.4, glow: 0.6 },
};
const state = { settings: null, asrReady: false, ttsReady: false, notes: 0, triggers: 0, mascot: false };
let foxFrame = null, pendingSay = "", orbCtx = null, sayTimer = 0;
const P = { colour: COPPER, pulse_hz: 0.25, turbulence: 0.3, glow: 0.2 };
const cur = Object.assign({}, P);

function toast(text) {
  const e = $("toast");
  e.textContent = text;
  e.style.display = "block";
  clearTimeout(toast.h);
  toast.h = setTimeout(() => (e.style.display = "none"), 4200);
}

function legacySecret() {
  try { return JSON.parse(localStorage.getItem("inari-pass")) || sessionStorage.getItem("inari-token") || ""; } catch (e) { return ""; }
}
function forgetLegacy() {
  try { localStorage.removeItem("inari-pass"); sessionStorage.removeItem("inari-token"); } catch (e) {}
}
function showLogin(why) {
  $("kapp").hidden = true;
  $("login").hidden = false;
  $("loginWhy").textContent = why || "";
  $("tokenInput").focus();
}

async function signIn(secret) {
  const r = await fetch("/api/session", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ secret }) }).catch(() => null);
  if (!r) return { ok: false, why: "The core cannot be reached. Check the connection and try again." };
  if (r.status === 429) { const d = await r.json().catch(() => ({})); return { ok: false, why: "Too many attempts. Try again in " + (d.retry_after || 60) + " s." }; }
  return r.ok ? { ok: true } : { ok: false, why: "This token or pass is not known by this core." };
}

async function api(path, body) {
  const r = await fetch(path, body === undefined
    ? { headers: { Accept: "application/json" } }
    : { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body) }).catch(() => null);
  if (!r) { toast("The core cannot be reached."); return { error: "offline" }; }
  if (r.status === 401) { showLogin(""); return { error: "unauthorized" }; }
  const data = await r.json().catch(() => ({}));
  if (!r.ok) { toast(data.error || "The core refused this request."); return Object.assign({ error: data.error || "http " + r.status }, data); }
  return data;
}

/* FoxBot: the mascot served under /fox/, and the canvas orb when the core serves no mascot. */
function fox(method, a) {
  try {
    const f = foxFrame && foxFrame.contentWindow && foxFrame.contentWindow.fox;
    if (!f) return false;
    if (method === "activity") { if (!f.setActivity) return false; f.setActivity(a); }
    else if (method === "mood") { if (!f.setMood) return false; f.setMood(a); }
    else if (method === "say") { if (!f.say) return false; f.say(a); }
    return true;
  } catch (e) { return false; }
}

function activity(name) {
  Object.assign(P, MOOD[name] || MOOD.idle);
  fox("activity", name);
}

function say(text) {
  $("ksay").textContent = text;
  if (!fox("say", text)) pendingSay = text;
  clearTimeout(sayTimer);
  sayTimer = setTimeout(() => { $("ksay").textContent = ""; }, 7000);
}

function mountFox() {
  const host = $("kfox");
  if (!state.mascot) return orbInit(host);
  const fr = document.createElement("iframe");
  fr.title = "FoxBot";
  fr.src = "/fox/index.html";
  fr.setAttribute("scrolling", "no");
  fr.addEventListener("load", () => {
    foxFrame = fr;
    let f = null;
    try { f = fr.contentWindow.fox; } catch (e) { f = null; }
    if (!f) { foxFrame = null; return orbInit(host); }
    if (f.setReducedMotion) { try { f.setReducedMotion(matchMedia("(prefers-reduced-motion: reduce)").matches); } catch (e) {} }
    activity("idle");
    if (pendingSay) { fox("say", pendingSay); pendingSay = ""; }
  });
  host.appendChild(fr);
}

function orbInit(host) {
  host.textContent = "";
  const c = document.createElement("canvas");
  c.width = 300;
  c.height = 300;
  c.setAttribute("role", "img");
  c.setAttribute("aria-label", "FoxBot");
  host.appendChild(c);
  orbCtx = c.getContext("2d");
  requestAnimationFrame(orbFrame);
}

function orbFrame(t) {
  if (!orbCtx) return;
  const c = orbCtx.canvas, w = c.width, m = w / 2, s = t / 1000;
  for (const k of ["pulse_hz", "turbulence", "glow"]) cur[k] += (P[k] - cur[k]) * 0.05;
  const still = matchMedia("(prefers-reduced-motion: reduce)").matches;
  const r0 = w * 0.27 * (still ? 1 : 1 + 0.06 * Math.sin(s * 2 * Math.PI * cur.pulse_hz));
  orbCtx.clearRect(0, 0, w, w);
  const halo = orbCtx.createRadialGradient(m, m, r0 * 0.6, m, m, w * 0.5);
  halo.addColorStop(0, cur.colour + "3a");
  halo.addColorStop(1, cur.colour + "00");
  orbCtx.fillStyle = halo;
  orbCtx.fillRect(0, 0, w, w);
  orbCtx.beginPath();
  for (let i = 0; i <= 120; i++) {
    const a = (i / 120) * Math.PI * 2;
    const n = still ? 0 : cur.turbulence * w * 0.035 * (Math.sin(a * 3 + s * 1.7) + Math.sin(a * 5 - s * 2.3) * 0.6);
    const r = r0 + n, x = m + Math.cos(a) * r, y = m + Math.sin(a) * r;
    if (i === 0) orbCtx.moveTo(x, y); else orbCtx.lineTo(x, y);
  }
  orbCtx.closePath();
  const fill = orbCtx.createRadialGradient(m, m, r0 * 0.1, m, m, r0 * 1.1);
  fill.addColorStop(0, "#fff8f0cc");
  fill.addColorStop(0.55, cur.colour);
  fill.addColorStop(1, cur.colour + "55");
  orbCtx.fillStyle = fill;
  orbCtx.fill();
  requestAnimationFrame(orbFrame);
}

function chip(el, ready, label) {
  el.className = "state " + (ready ? "ok live" : "off");
  el.textContent = label + (ready ? " ready" : " off");
}

function text(el, value) {
  const v = value || "";
  el.textContent = v || "not set";
  el.className = v ? "" : "none";
}

function render() {
  const s = state.settings || {}, asr = s.asr || {}, tts = s.tts || {};
  chip($("kAsr"), state.asrReady, "ASR");
  chip($("kTts"), state.ttsReady, "TTS");
  $("kNotes").textContent = String(state.notes);
  $("kTriggers").textContent = String(state.triggers);
  text($("kAsrUrl"), asr.url);
  text($("kTtsUrl"), tts.url);
  text($("kEmbed"), s.embed_model);
}

async function load() {
  const d = await api("/api/kitsune");
  if (d.error) {
    if (d.error !== "unauthorized" && d.error !== "offline") say("This core holds no Kitsune yet.");
    return;
  }
  state.settings = d.settings || {};
  state.asrReady = !!d.asr_ready;
  state.ttsReady = !!d.tts_ready;
  state.notes = Number(d.notes) || 0;
  state.triggers = Number(d.triggers) || 0;
  state.mascot = !!d.mascot;
  render();
  const missing = [];
  if (!state.asrReady) missing.push("speech to text");
  if (!state.ttsReady) missing.push("text to speech");
  say(missing.length ? "Local " + missing.join(" and ") + " not set yet." : "Ready. Notes: " + state.notes + ".");
}

async function connect() {
  const hash = new URLSearchParams(location.hash.slice(1));
  const linked = hash.get("pass") || hash.get("token");
  const kept = legacySecret();
  if (location.hash) history.replaceState(null, "", location.pathname);
  if (linked) { const r = await signIn(linked); if (!r.ok) return showLogin(r.why); }
  else if (kept) await signIn(kept);
  forgetLegacy();
  const who = await fetch("/api/whoami").then((r) => (r.ok ? r.json() : null)).catch(() => null);
  if (!who) return showLogin("");
  $("login").hidden = true;
  $("kapp").hidden = false;
  await load();
  mountFox();
}

$("tokenGo").addEventListener("click", async () => {
  const secret = $("tokenInput").value.trim();
  if (!secret) return;
  const r = await signIn(secret);
  if (!r.ok) { $("loginWhy").textContent = r.why; return; }
  $("tokenInput").value = "";
  await connect();
});
$("tokenInput").addEventListener("keydown", (e) => { if (e.key === "Enter") $("tokenGo").click(); });
connect();
