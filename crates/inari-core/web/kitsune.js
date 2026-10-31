"use strict";
/* Kitsune page: the voice notebook, driven by /api/kitsune on this core. */
const $ = (id) => document.getElementById(id);
const COPPER = "#e48a46";
const MOOD = {
  idle: { pulse_hz: 0.25, turbulence: 0.3, glow: 0.2 },
  listening: { pulse_hz: 0.9, turbulence: 0.8, glow: 0.45 },
  working: { pulse_hz: 1.6, turbulence: 1.4, glow: 0.6 },
};
const state = { settings: null, asrReady: false, ttsReady: false, notes: 0, reminders: 0, mascot: false };
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
  chip($("kAsr"), state.asrReady, "ASR");
  chip($("kTts"), state.ttsReady, "TTS");
  $("kNotes").textContent = String(state.notes);
  $("kTriggers").textContent = String(state.reminders);
}

/* Settings: the servers this core talks to. Filled from the core, never overwritten under the
   caret, and sent back whole so nothing the page does not know about is lost. */
function setFill() {
  const s = state.settings || {}, asr = s.asr || {}, tts = s.tts || {};
  const slm = s.slm || {}, emb = s.embed || {}, geo = s.geocoder || {};
  const put = (id, v) => {
    const el = $(id);
    if (el && document.activeElement !== el) el.value = v || "";
  };
  put("kSetAsrUrl", asr.url);
  put("kSetAsrModel", asr.model);
  put("kSetAsrLang", asr.language);
  put("kSetTtsUrl", tts.url);
  put("kSetTtsVoice", tts.voice);
  put("kSetSlmUrl", slm.url);
  put("kSetSlmModel", slm.model);
  put("kSetEmbedUrl", emb.url);
  put("kSetEmbed", emb.model);
  put("kSetGeoUrl", geo.url);
  put("kSetGeoKind", geo.kind || "nominatim");
}

async function setSave() {
  const s = JSON.parse(JSON.stringify(state.settings || {}));
  s.asr = s.asr || {};
  s.tts = s.tts || {};
  s.asr.url = $("kSetAsrUrl").value.trim();
  s.asr.model = $("kSetAsrModel").value.trim();
  s.asr.language = $("kSetAsrLang").value.trim();
  s.tts.url = $("kSetTtsUrl").value.trim();
  s.tts.voice = $("kSetTtsVoice").value.trim();
  s.slm = s.slm || {};
  s.embed = s.embed || {};
  s.geocoder = s.geocoder || {};
  s.slm.url = $("kSetSlmUrl").value.trim();
  s.slm.model = $("kSetSlmModel").value.trim();
  s.embed.url = $("kSetEmbedUrl").value.trim();
  s.embed.model = $("kSetEmbed").value.trim();
  s.geocoder.url = $("kSetGeoUrl").value.trim();
  s.geocoder.kind = $("kSetGeoKind").value;
  $("kSetState").textContent = "Saving…";
  const d = await api("/api/kitsune", { action: "set_settings", settings: s });
  if (!d || d.error) {
    $("kSetState").textContent = d && d.error ? "The core refused it: " + d.error : "The core did not answer.";
    return;
  }
  await load();
  const on = [];
  if (s.asr.url) on.push("speech to text");
  if (s.tts.url) on.push("text to speech");
  if (s.slm.url) on.push("understanding");
  if (s.embed.url) on.push("meaning");
  if (s.geocoder.url) on.push("places");
  $("kSetState").textContent = on.length ? "Saved. " + on.join(", ") + " on." : "Saved. No server set.";
  say("Settings kept.");
}

$("kSet").addEventListener("submit", (ev) => {
  ev.preventDefault();
  setSave();
});

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
  state.reminders = Number(d.reminders) || 0;
  state.mascot = !!d.mascot;
  render();
  setFill();
  const missing = [];
  if (!state.asrReady) missing.push("speech to text");
  if (!state.ttsReady) missing.push("text to speech");
  say(missing.length ? "Local " + missing.join(" and ") + " not set yet." : "Ready. Notes: " + state.notes + ".");
  memLoad();
  remFillWhen();
  remLoad();
}

/* Talk: hold the button, record here, hand the core a 16 kHz mono wav. */
const TARGET_RATE = 16000;
const rec = { stream: null, media: null, chunks: [], t0: 0, tick: 0, busy: false, reviewId: 0 };

function clock(ms) {
  const s = Math.max(0, ms) / 1000;
  return Math.floor(s / 60) + ":" + String(Math.floor(s % 60)).padStart(2, "0");
}

function recMime() {
  if (typeof MediaRecorder === "undefined") return "";
  const kinds = ["audio/webm;codecs=opus", "audio/webm", "audio/mp4", "audio/ogg;codecs=opus"];
  for (const k of kinds) if (MediaRecorder.isTypeSupported(k)) return k;
  return "";
}

function micError(e) {
  const n = e && e.name;
  if (n === "NotAllowedError" || n === "SecurityError") return "The microphone is blocked. Allow it for this page, then try again.";
  if (n === "NotFoundError" || n === "OverconstrainedError") return "No microphone answers on this machine.";
  if (n === "NotReadableError") return "Another program holds the microphone.";
  return "The microphone did not open: " + ((e && e.message) || "unknown reason") + ".";
}

async function startTalk() {
  if (rec.media || rec.busy) return;
  const mime = recMime();
  if (!mime) return say("This browser cannot record audio. Type the note instead.");
  let stream;
  try {
    stream = await navigator.mediaDevices.getUserMedia({
      audio: { channelCount: 1, echoCancellation: true, noiseSuppression: true, autoGainControl: true },
    });
  } catch (e) { return say(micError(e)); }
  rec.stream = stream;
  rec.chunks = [];
  const m = new MediaRecorder(stream, { mimeType: mime, audioBitsPerSecond: 64000 });
  rec.media = m;
  m.addEventListener("dataavailable", (ev) => { if (ev.data && ev.data.size) rec.chunks.push(ev.data); });
  m.start(250);
  rec.t0 = performance.now();
  activity("listening");
  $("kTalk").classList.add("live");
  $("kTalkLabel").textContent = "Listening — release to keep";
  $("kTime").textContent = "0:00";
  rec.tick = setInterval(() => { $("kTime").textContent = clock(performance.now() - rec.t0); }, 200);
}

async function stopTalk() {
  const m = rec.media;
  if (!m) return;
  clearInterval(rec.tick);
  rec.media = null;
  $("kTalk").classList.remove("live");
  $("kTalkLabel").textContent = "Hold to talk";
  const stopped = new Promise((res) => m.addEventListener("stop", res, { once: true }));
  if (m.state !== "inactive") m.stop();
  if (rec.stream) rec.stream.getTracks().forEach((t) => t.stop());
  rec.stream = null;
  await stopped;
  const blob = new Blob(rec.chunks, { type: m.mimeType || "audio/webm" });
  rec.chunks = [];
  const ms = performance.now() - rec.t0;
  $("kTime").textContent = clock(ms);
  if (ms < 400) { activity("idle"); return say("Too short. Hold the button while you speak."); }
  rec.busy = true;
  $("kTalk").disabled = true;
  activity("working");
  say("Writing it down on this core…");
  try {
    const wav = await toWav(blob);
    const d = await api("/api/kitsune", { action: "ingest_voice", audio: b64(wav), place: $("kPlace").value.trim() });
    if (d.inaudible) say(d.error);
    else if (d.error) say("The capture was not kept.");
    else await keep(d.id);
  } catch (e) {
    toast("The audio could not be read: " + ((e && e.message) || e));
  } finally {
    rec.busy = false;
    $("kTalk").disabled = false;
    activity("idle");
  }
}

async function toWav(blob) {
  const AC = window.AudioContext || window.webkitAudioContext;
  if (!AC) throw new Error("this browser cannot decode audio");
  const ctx = new AC();
  let buf;
  try { buf = await ctx.decodeAudioData(await blob.arrayBuffer()); }
  finally { if (ctx.close) ctx.close(); }
  let data;
  if (buf.sampleRate === TARGET_RATE && buf.numberOfChannels === 1) data = buf.getChannelData(0);
  else {
    const frames = Math.max(1, Math.ceil(buf.duration * TARGET_RATE));
    const off = new OfflineAudioContext(1, frames, TARGET_RATE);
    const src = off.createBufferSource();
    src.buffer = buf;
    src.connect(off.destination);
    src.start();
    data = (await off.startRendering()).getChannelData(0);
  }
  return pcm16(data);
}

function pcm16(samples) {
  const out = new Uint8Array(44 + samples.length * 2);
  const v = new DataView(out.buffer);
  const tag = (at, s) => { for (let i = 0; i < s.length; i++) out[at + i] = s.charCodeAt(i); };
  tag(0, "RIFF");
  v.setUint32(4, 36 + samples.length * 2, true);
  tag(8, "WAVEfmt ");
  v.setUint32(16, 16, true);
  v.setUint16(20, 1, true);
  v.setUint16(22, 1, true);
  v.setUint32(24, TARGET_RATE, true);
  v.setUint32(28, TARGET_RATE * 2, true);
  v.setUint16(32, 2, true);
  v.setUint16(34, 16, true);
  tag(36, "data");
  v.setUint32(40, samples.length * 2, true);
  for (let i = 0, at = 44; i < samples.length; i++, at += 2) {
    const s = Math.max(-1, Math.min(1, samples[i]));
    v.setInt16(at, s < 0 ? s * 0x8000 : s * 0x7fff, true);
  }
  return out;
}

function b64(bytes) {
  const step = 0x8000;
  let s = "";
  for (let i = 0; i < bytes.length; i += step) s += String.fromCharCode.apply(null, bytes.subarray(i, i + step));
  return btoa(s);
}

// what happens next to a kept note, said in one short sentence
function comesBack(n) {
  const where = n.location_raw || "that place";
  if (n.status === "pending_geocode") return ". I will place " + where + " when the map answers.";
  if (n.status === "failed_geocode") return ". I could not find " + where + ", so no place reminder.";
  if (n.status === "ambiguous_geocode") return ". Several places are called " + where + ": set it by hand.";
  if (n.trigger_type === "time") return ". It comes back " + stamp(n.due_date) + ".";
  if (n.trigger_type === "geofence") return ". It comes back at " + where + ".";
  return ".";
}

async function keep(id) {
  const d = await api("/api/kitsune", { action: "get_note", id });
  if (d.error) return;
  const n = d.note || {};
  await load();
  if (n.status === "pending_transcription") return showReview(n);
  hideReview();
  say(words(n) ? "Kept: " + words(n) + comesBack(n) : "Kept, with the audio.");
}

function showReview(n) {
  rec.reviewId = n.id;
  $("kReview").hidden = false;
  $("kReviewText").value = words(n);
  $("kReviewText").focus();
  say("I kept the audio but could not write it out. Type what you said.");
}

function hideReview() {
  $("kReview").hidden = true;
  rec.reviewId = 0;
}

async function saveReview() {
  const id = rec.reviewId, words = $("kReviewText").value.trim();
  if (!id) return;
  if (!words) { toast("Write what you said first."); $("kReviewText").focus(); return; }
  activity("working");
  const d = await api("/api/kitsune", { action: "correct_note", id, text: words });
  activity("idle");
  if (d.error) return;
  hideReview();
  await load();
  say("Corrected: " + words);
}

async function keepTyped() {
  const words = $("kText").value.trim();
  if (!words) return $("kText").focus();
  $("kTextGo").disabled = true;
  activity("working");
  const d = await api("/api/kitsune", { action: "capture_text", text: words, tags: [] });
  activity("idle");
  $("kTextGo").disabled = false;
  if (d.error) return;
  $("kText").value = "";
  await keep(d.id);
}

/* Memory: find what was said, read it back, correct it, forget it. */
const mem = { query: "", url: "" };

// what a note says: its clean summary, else the words as they were said
function words(n) {
  return n.clean_summary || n.raw_transcript || "";
}

// an ISO 8601 UTC time (or seconds) as the person's local time
function stamp(t) {
  const at = typeof t === "string" ? Date.parse(t) / 1000 : Number(t) || 0;
  if (!at) return "";
  return new Date(at * 1000).toLocaleString(undefined, { day: "2-digit", month: "short", hour: "2-digit", minute: "2-digit" });
}

function actButton(act, label, cls) {
  const b = document.createElement("button");
  b.type = "button";
  b.className = cls || "btn small";
  b.dataset.act = act;
  b.textContent = label;
  return b;
}

function noteLi(n) {
  const li = document.createElement("li");
  li.className = "knote";
  li.dataset.id = String(n.id);
  const said = document.createElement("p");
  said.className = "knote-text" + (words(n) ? "" : " waiting");
  said.textContent = words(n) || (n.status === "pending_transcription" ? "Audio kept, words missing." : "Nothing written in this note.");
  li.appendChild(said);
  const meta = document.createElement("p");
  meta.className = "kmeta";
  const bits = [stamp(n.created_at), n.category !== "memo" ? n.category : "", n.location_raw, n.source];
  for (const t of n.tags || []) bits.push("#" + t);
  meta.textContent = bits.filter(Boolean).join(" \u00b7 ");
  li.appendChild(meta);
  const acts = document.createElement("div");
  acts.className = "knote-actions";
  acts.append(actButton("read", "Read aloud"), actButton("fix", "Correct"), actButton("drop", "Delete", "btn small danger"));
  li.appendChild(acts);
  const fix = document.createElement("div");
  fix.className = "knote-fix";
  fix.hidden = true;
  const ta = document.createElement("textarea");
  ta.className = "kfield";
  ta.value = words(n);
  ta.setAttribute("aria-label", "Correct the note");
  fix.appendChild(ta);
  fix.append(actButton("save", "Save", "btn small go"), actButton("cancel", "Cancel"));
  li.appendChild(fix);
  return li;
}

function memState(line) {
  $("kMemState").textContent = line || "";
}

function renderNotes(notes) {
  const list = $("kNotesList");
  list.textContent = "";
  for (const n of notes) list.appendChild(noteLi(n));
}

async function memLoad() {
  const q = mem.query.trim();
  const d = q
    ? await api("/api/kitsune", { action: "search", query: q, limit: 20 })
    : await api("/api/kitsune", { action: "list_notes", limit: 20 });
  if (d.error) return memState("");
  const notes = q ? (d.hits || []).map((h) => h.note) : (d.notes || []);
  $("kFindClear").hidden = !q;
  renderNotes(notes);
  if (!notes.length) return memState(q ? "Nothing matches " + q + "." : "Nothing kept yet.");
  memState(q ? notes.length + " note(s) for " + q + "." : "The " + notes.length + " newest notes.");
}

function playAudio(src, revoke) {
  const p = $("kPlayer");
  if (mem.url) URL.revokeObjectURL(mem.url);
  mem.url = revoke ? src : "";
  p.src = src;
  p.play().catch(() => {});
}

async function readNote(id) {
  const d = await api("/api/kitsune", { action: "get_note", id });
  if (d.error) return;
  const n = d.note || {};
  const kept = (d.audio || [])[0];
  if (kept && kept.path) {
    activity("working");
    const r = await fetch("/api/kitsune/audio?path=" + encodeURIComponent(kept.path)).catch(() => null);
    activity("idle");
    if (!r || !r.ok) { toast("That audio cannot be read back."); return; }
    playAudio(URL.createObjectURL(await r.blob()), true);
    return say("Here is what you said.");
  }
  if (!words(n)) { toast("There are no words to read back yet."); return; }
  activity("working");
  const s = await api("/api/kitsune", { action: "speak", text: words(n), note_id: n.id });
  activity("idle");
  if (s.error) return;
  if (!s.audio) { toast("The core answered without audio."); return; }
  const bytes = Uint8Array.from(atob(s.audio), (c) => c.charCodeAt(0));
  playAudio(URL.createObjectURL(new Blob([bytes], { type: s.mime || "audio/wav" })), true);
  say("Reading it back.");
}

async function saveFix(li) {
  const words = li.querySelector(".knote-fix .kfield").value.trim();
  if (!words) { toast("Write the words first."); return; }
  activity("working");
  const d = await api("/api/kitsune", { action: "correct_note", id: Number(li.dataset.id), text: words });
  activity("idle");
  if (d.error) return;
  await load();
  say("Corrected: " + words);
}

async function dropNote(li) {
  if (!confirm("Forget this note and its audio?")) return;
  activity("working");
  const d = await api("/api/kitsune", { action: "delete_note", id: Number(li.dataset.id) });
  activity("idle");
  if (d.error) return;
  await load();
  say("Forgotten.");
}

$("kNotesList").addEventListener("click", (ev) => {
  const b = ev.target.closest("button[data-act]");
  const li = b && b.closest(".knote");
  if (!li) return;
  const act = b.dataset.act;
  if (act === "read") return readNote(Number(li.dataset.id));
  if (act === "fix") { const f = li.querySelector(".knote-fix"); f.hidden = false; f.querySelector(".kfield").focus(); return; }
  if (act === "cancel") { li.querySelector(".knote-fix").hidden = true; return; }
  if (act === "save") return saveFix(li);
  if (act === "drop") return dropNote(li);
});
$("kFind").addEventListener("submit", (ev) => { ev.preventDefault(); mem.query = $("kQuery").value; memLoad(); });
$("kFindClear").addEventListener("click", () => { $("kQuery").value = ""; mem.query = ""; memLoad(); });

/* Reminders: a note that comes back at a time or at a place, until it is done. */

function whenText(secs) {
  return new Date(secs * 1000).toLocaleString(undefined, {
    year: "numeric", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit",
  });
}

function whenField(secs) {
  const d = new Date(secs * 1000);
  const p = (n) => String(n).padStart(2, "0");
  return d.getFullYear() + "-" + p(d.getMonth() + 1) + "-" + p(d.getDate()) + "T" + p(d.getHours()) + ":" + p(d.getMinutes());
}

// seconds since the epoch to the brain's ISO 8601 UTC form
function isoUtc(secs) {
  return new Date(secs * 1000).toISOString().slice(0, 19) + "Z";
}

function remFillWhen() {
  const f = $("kRemWhen");
  if (f.value) return;
  f.value = whenField(Math.ceil(Date.now() / 1000 / 60) * 60 + 3600);
}

function remLi(n) {
  const li = document.createElement("li");
  li.className = "ktri";
  li.dataset.id = String(n.id);
  const body = document.createElement("div");
  body.className = "ktri-body";
  const p = document.createElement("p");
  p.className = "ktri-text";
  p.textContent = words(n) || "Reminder #" + n.id;
  const meta = document.createElement("p");
  meta.className = "kmeta";
  const bits = [];
  if (n.trigger_type === "time") {
    bits.push(stamp(n.due_date), rel(n.due_date));
    if (n.notified_at) bits.push("rang");
  } else {
    bits.push((n.location_raw || "a place") + ", within " + n.radius_meters + " m");
  }
  meta.textContent = bits.filter(Boolean).join(" \u00b7 ");
  body.append(p, meta);
  li.append(body);
  if (n.trigger_type === "time") li.append(actButton("later", "In 1 h", "btn small"));
  li.append(actButton("done", "Done", "btn small go"));
  return li;
}

function remState(line) {
  text($("kRemState"), line);
}

async function remLoad() {
  const d = await api("/api/kitsune", { action: "reminders" });
  const list = $("kTrigList");
  list.textContent = "";
  if (d.error) return remState("");
  const all = d.reminders || [];
  phoneSync(all);
  for (const n of all) list.appendChild(remLi(n));
  remState(!all.length ? "Nothing set to come back." : all.length + " still to come.");
}

async function keepReminder(ev) {
  ev.preventDefault();
  const what = $("kRemWhat").value.trim();
  const when = $("kRemWhen").value;
  if (!what) { toast("Say what should come back."); return; }
  if (!when) { toast("Pick a time first."); return; }
  const at = Math.round(new Date(when).getTime() / 1000);
  if (!Number.isFinite(at)) { toast("That time cannot be read."); return; }
  activity("working");
  const kept = await api("/api/kitsune", { action: "capture_text", text: what, tags: ["reminder"] });
  if (kept.error) { activity("idle"); return; }
  const made = await api("/api/kitsune", { action: "set_reminder", id: kept.id,
    reminder: { trigger_type: "time", due_date: isoUtc(at) } });
  activity("idle");
  if (made.error || !made.ok) { toast("The reminder was not kept."); return; }
  $("kRemWhat").value = "";
  $("kRemWhen").value = "";
  remFillWhen();
  await load();
  say("It comes back on " + whenText(at) + ".");
}

async function remAct(li, act) {
  const id = Number(li.dataset.id);
  activity("working");
  const d = act === "later"
    ? await api("/api/kitsune", { action: "snooze", id, due_date: isoUtc(Math.round(Date.now() / 1000) + 3600) })
    : await api("/api/kitsune", { action: "dismiss", id });
  activity("idle");
  if (d.error) return;
  await load();
  say(act === "later" ? "It comes back in an hour." : "Done.");
}

$("kTrigList").addEventListener("click", (ev) => {
  const b = ev.target.closest("button[data-act]");
  const li = b && b.closest(".ktri");
  if (li) remAct(li, b.dataset.act);
});
$("kRemForm").addEventListener("submit", keepReminder);
$("kRemWhen").addEventListener("focus", remFillWhen);

$("kTalk").addEventListener("pointerdown", (ev) => { ev.preventDefault(); startTalk(); });
window.addEventListener("pointerup", stopTalk);
window.addEventListener("pointercancel", stopTalk);
$("kTalk").addEventListener("keydown", (ev) => {
  if ((ev.key === " " || ev.key === "Enter") && !ev.repeat) { ev.preventDefault(); startTalk(); }
});
$("kTalk").addEventListener("keyup", (ev) => {
  if (ev.key === " " || ev.key === "Enter") { ev.preventDefault(); stopTalk(); }
});
$("kTalk").addEventListener("blur", stopTalk);
$("kTextGo").addEventListener("click", keepTyped);
$("kReviewSave").addEventListener("click", saveReview);

/* Time made visible (docs/tdah-recherche-fr.md, P6): "in 20 min" beside "09:00". */
function rel(iso) {
  const at = Date.parse(iso);
  if (!at) return "";
  const m = Math.round((at - Date.now()) / 60000);
  if (Math.abs(m) < 1) return "now";
  const span = Math.abs(m) < 60 ? Math.abs(m) + " min" : Math.abs(m) < 1440 ? Math.round(Math.abs(m) / 60) + " h" : Math.round(Math.abs(m) / 1440) + " d";
  return m > 0 ? "in " + span : span + " ago";
}

// a local time today or tomorrow (hh:mm), as the brain's ISO 8601 UTC
function localAt(dayOffset, h, m) {
  const d = new Date();
  d.setDate(d.getDate() + dayOffset);
  d.setHours(h, m || 0, 0, 0);
  return isoUtc(Math.round(d.getTime() / 1000));
}

/* The views: Talk (the stage, today, the memory), Assistants, Lists, Running, My week. */
const view = { now: "talk", timer: 0 };

function showView(name) {
  view.now = name;
  const map = { talk: "kmain", assist: "vAssist", lists: "vLists", runs: "vRuns", week: "vWeek" };
  for (const [k, id] of Object.entries(map)) $(id).hidden = k !== name;
  document.querySelectorAll("#kviews button").forEach((b) => {
    if (b.dataset.view === name) b.setAttribute("aria-current", "page"); else b.removeAttribute("aria-current");
  });
  clearInterval(view.timer);
  if (name === "assist") assistDraw();
  if (name === "lists") listsLoad();
  if (name === "runs") { runsLoad(); view.timer = setInterval(runsLoad, 5000); }
  if (name === "week") weekLoad();
  try { history.replaceState(null, "", name === "talk" ? location.pathname : "#" + name); } catch (e) {}
}
$("kviews").addEventListener("click", (ev) => {
  const b = ev.target.closest("button[data-view]");
  if (b) showView(b.dataset.view);
});

/* Today: the "Today" list's first steps, and what did not happen, to place again without blame. */
async function todayLoad() {
  const box = $("kTodayList");
  const l = await api("/api/kitsune", { action: "lists" });
  const today = (l.lists || []).find((x) => x.kind === "todo" && /^(today|aujourd)/i.test(x.name));
  box.textContent = "";
  if (today) {
    const d = await api("/api/kitsune", { action: "items", list: today.name });
    for (const it of (d.items || []).slice(0, 3)) {
      const li = document.createElement("li");
      li.textContent = it.text;
      if (it.due_date) { const m = document.createElement("span"); m.className = "kmeta"; m.textContent = " · " + rel(it.due_date); li.append(m); }
      box.append(li);
    }
  }
  $("kTodayState").textContent = box.children.length ? "" : "Nothing planned yet. The assistant can plan the day with you.";
  const o = await api("/api/kitsune", { action: "overdue" });
  const host = $("kOverdue");
  host.textContent = "";
  for (const it of (o.items || []).slice(0, 2)) {
    const row = document.createElement("div");
    row.className = "kgentle";
    const s = document.createElement("span");
    s.textContent = "“" + it.text + "” did not happen. Place it again?";
    const later = actButton("later", "In 1 h");
    const tomorrow = actButton("tomorrow", "Tomorrow 9:00");
    later.onclick = () => moveItem(it.id, isoUtc(Math.round(Date.now() / 1000) + 3600));
    tomorrow.onclick = () => moveItem(it.id, localAt(1, 9));
    row.append(s, later, tomorrow);
    host.append(row);
  }
}

async function moveItem(id, due) {
  const d = await api("/api/kitsune", { action: "move_item", id, due_date: due });
  if (d.error) return;
  say("Placed again, " + rel(due) + ".");
  todayLoad();
}

/* Assistants: one per job, each on its tier, started in a thread of its own. */
const ASSISTANTS = [
  { wf: "daily-plan", name: "Plan of the day", what: "Three priorities at most, each with its smallest first step, its duration and its cue. You agree before anything is written.", tier: "standard", where: "local" },
  { wf: "kitsune-inbox", name: "Sort my notes", what: "Each note to its list, its reminder or its specialist.", tier: "fast", where: "local" },
  { wf: "groceries", name: "Shopping & pantry", what: "What expires, what is missing, the list by aisle, meal ideas.", tier: "fast", where: "local" },
  { wf: "weekly-review", name: "Weekly review", what: "What went well, one pattern, one thing for next week. Private.", tier: "standard", where: "local" },
  { wf: "doc-sort", name: "Sort documents", what: "A name and a place for each document; nothing moves until you approve.", tier: "standard", where: "local", params: [{ id: "folder", label: "Folder" }] },
  { wf: "research-brief", name: "Research", what: "A question searched on your SearXNG, with its sources, kept as a note.", tier: "expert", where: "your expert tier", params: [{ id: "question", label: "Question" }] },
  { wf: "dev-handoff", name: "Development", what: "A plan with acceptance criteria; once you agree, a coder works on it in an Inari thread.", tier: "expert", where: "an Inari thread", params: [{ id: "request", label: "What to build" }, { id: "harness", label: "Coder harness (empty: the default)", optional: true }] },
];
const assist = { pick: null };

function assistDraw() {
  const grid = $("kAssistList");
  grid.textContent = "";
  for (const a of ASSISTANTS) {
    const b = document.createElement("button");
    b.type = "button";
    b.className = "kassistant";
    const n = document.createElement("b"); n.textContent = a.name;
    const w = document.createElement("span"); w.textContent = a.what;
    const tier = document.createElement("em"); tier.className = "ktier"; tier.textContent = a.tier;
    const where = document.createElement("em"); where.className = "ktier where"; where.textContent = a.where;
    b.append(n, w, tier, where);
    b.onclick = () => assistOpen(a);
    grid.append(b);
  }
}

function assistOpen(a) {
  assist.pick = a;
  if (!a.params) return assistStart({});
  $("kAssistForm").hidden = false;
  $("kAssistTitle").textContent = a.name;
  const host = $("kAssistParams");
  host.textContent = "";
  for (const p of a.params) {
    const lab = document.createElement("label"); lab.className = "klab"; lab.textContent = p.label; lab.htmlFor = "kp-" + p.id;
    const inp = document.createElement("input"); inp.className = "kfield"; inp.id = "kp-" + p.id; inp.dataset.param = p.id; inp.required = !p.optional;
    host.append(lab, inp);
  }
  host.querySelector("input").focus();
}

async function assistStart(params) {
  const a = assist.pick;
  activity("working");
  const d = await api("/api/kitsune", { action: "assist", workflow: a.wf, title: a.name, params });
  activity("idle");
  if (d.error) return;
  $("kAssistForm").hidden = true;
  say(a.name + ": started. Follow it in Running.");
  showView("runs");
}

$("kAssistForm").addEventListener("submit", (ev) => {
  ev.preventDefault();
  const params = {};
  for (const inp of $("kAssistParams").querySelectorAll("input")) if (inp.value.trim()) params[inp.dataset.param] = inp.value.trim();
  assistStart(params);
});
$("kAssistCancel").addEventListener("click", () => { $("kAssistForm").hidden = true; });

/* Lists: shopping by aisle, the pantry and what expires, the tasks. */
const lists = { kind: "shopping", name: "" };

async function listsLoad() {
  document.querySelectorAll("#kListKinds button").forEach((b) => b.setAttribute("aria-selected", String(b.dataset.kind === lists.kind)));
  const d = await api("/api/kitsune", { action: "lists" });
  const mine = (d.lists || []).filter((l) => l.kind === lists.kind);
  const pick = $("kListPick");
  pick.textContent = "";
  for (const l of mine) pick.append(new Option(l.name + (l.open ? " · " + l.open : ""), l.name, false, l.name === lists.name));
  const fallback = { shopping: "Courses", pantry: "Garde-manger", todo: "Today" }[lists.kind];
  if (!mine.length) pick.append(new Option(fallback + " (new)", fallback, false, true));
  lists.name = pick.value;
  $("kItemAisle").hidden = lists.kind === "todo";
  $("kExpiring").textContent = "";
  if (lists.kind === "pantry") {
    const e = await api("/api/kitsune", { action: "expiring", day: new Date(Date.now() + 3 * 86400000).toISOString().slice(0, 10) });
    if ((e.items || []).length) {
      const box = document.createElement("div");
      box.className = "kgentle";
      const s = document.createElement("span");
      s.textContent = "Soon past their date: " + e.items.map((i) => i.text + " (" + i.expires_on + ")").join(", ");
      const meals = actButton("meals", "Meal ideas", "btn small go");
      meals.onclick = () => { assist.pick = ASSISTANTS.find((a) => a.wf === "groceries"); assistStart({}); };
      box.append(s, meals);
      $("kExpiring").append(box);
    }
  }
  itemsLoad();
}

async function itemsLoad() {
  const ul = $("kItems");
  ul.textContent = "";
  if (!lists.name || $("kListPick").selectedOptions[0]?.text.endsWith("(new)")) { $("kListState").textContent = "Empty for now: add the first item."; return; }
  const d = await api("/api/kitsune", { action: "items", list: lists.name, with_done: true });
  let aisle = null;
  for (const it of d.items || []) {
    if (lists.kind !== "todo" && !it.done && it.aisle !== aisle) {
      aisle = it.aisle;
      const h = document.createElement("li"); h.className = "kaisle-h"; h.textContent = aisle || "anywhere"; ul.append(h);
    }
    const li = document.createElement("li");
    li.className = "kitem" + (it.done ? " done" : "");
    const box = document.createElement("input"); box.type = "checkbox"; box.checked = it.done; box.id = "ki-" + it.id;
    box.onchange = async () => { await api("/api/kitsune", { action: "check", ids: [it.id], done: box.checked }); itemsLoad(); if (box.checked) say("Done. One less."); };
    const lab = document.createElement("label"); lab.htmlFor = box.id; lab.textContent = it.text;
    li.append(box, lab);
    const meta = [it.qty != null ? it.qty + (it.unit ? " " + it.unit : "") : "", it.due_date ? rel(it.due_date) : "", it.expires_on ? "until " + it.expires_on : ""].filter(Boolean).join(" · ");
    if (meta) { const m = document.createElement("span"); m.className = "kmeta"; m.textContent = meta; li.append(m); }
    ul.append(li);
  }
  const open = (d.items || []).filter((i) => !i.done).length;
  $("kListState").textContent = open ? open + " to go." : "All done here.";
}

$("kListKinds").addEventListener("click", (ev) => {
  const b = ev.target.closest("button[data-kind]");
  if (!b) return;
  lists.kind = b.dataset.kind;
  lists.name = "";
  listsLoad();
});
$("kListPick").addEventListener("change", () => { lists.name = $("kListPick").value; itemsLoad(); });
$("kAddItem").addEventListener("submit", async (ev) => {
  ev.preventDefault();
  const text = $("kItemText").value.trim();
  if (!text) return;
  const d = await api("/api/kitsune", { action: "add_items", list: lists.name, kind: lists.kind, items: [{ text, aisle: $("kItemAisle").value.trim() }] });
  if (d.error) return;
  $("kItemText").value = "";
  listsLoad();
});

/* Running: the assistants' threads, and the choices of a step that waits for the person. */
async function runsLoad() {
  const d = await api("/api/kitsune", { action: "runs" });
  const ul = $("kRunsList");
  ul.textContent = "";
  for (const r of d.runs || []) {
    const li = document.createElement("li");
    li.className = "knote";
    const title = document.createElement("p"); title.className = "knote-text"; title.textContent = r.title || r.id;
    const wf = r.workflow || {};
    const meta = document.createElement("p"); meta.className = "kmeta";
    meta.textContent = [wf.status === "waiting" ? "waits for you" : wf.status, wf.step, r.phase].filter(Boolean).join(" · ");
    li.append(title, meta);
    if (r.said) { const s = document.createElement("p"); s.className = "krun-said"; s.textContent = String(r.said).slice(0, 1200); li.append(s); }
    if (wf.status === "waiting") {
      const row = document.createElement("div"); row.className = "krun-choices";
      for (const c of wf.choices || []) {
        const b = actButton("choice", c.label, "btn small go");
        b.onclick = async () => { await api("/api/message", { agent: r.id, text: "", workflow: { resume: true, choice: c.id, label: c.label } }); say(c.label + "."); runsLoad(); };
        row.append(b);
      }
      li.append(row);
    }
    const open = document.createElement("a"); open.href = "/app"; open.textContent = "Open the thread in Inari"; open.className = "kmeta";
    li.append(open);
    ul.append(li);
  }
  $("kRunsState").textContent = (d.runs || []).length ? "" : "No assistant is working. Start one in Assistants.";
}

/* My week: kept and done, privately, and the weekly review. */
async function weekLoad() {
  const d = await api("/api/kitsune", { action: "week" });
  if (d.error) return;
  $("kWeekDone").textContent = String(d.done);
  $("kWeekKept").textContent = String(d.kept);
}
$("kWeekReview").addEventListener("click", () => { assist.pick = ASSISTANTS.find((a) => a.wf === "weekly-review"); assistStart({}); });

/* The Inari app on the phone (window.InariPhone): it keeps the reminders so they ring with no network,
   on its own alarm and the system's geofencing, and hands back what was done from a notification. */
function phone() {
  try { return window.InariPhone && window.InariPhone.version() >= 1 ? window.InariPhone : null; } catch (e) { return null; }
}

function phoneSync(reminders) {
  const p = phone();
  if (!p) return;
  try {
    p.syncReminders(JSON.stringify(reminders));
    const rings = (state.settings && state.settings.rings) || {};
    if (rings.geofence_quiet_minutes != null) p.setQuietMinutes(Number(rings.geofence_quiet_minutes) || 0);
  } catch (e) {}
}

// "Done" and "In 1 h" pressed on the phone's notifications while the page was closed
async function phoneActions() {
  const p = phone();
  if (!p) return;
  let acts = [];
  try { acts = JSON.parse(p.takeActions() || "[]"); } catch (e) { acts = []; }
  for (const a of acts) {
    if (a.action === "dismiss") await api("/api/kitsune", { action: "dismiss", id: a.id });
    else if (a.action === "snooze") await api("/api/kitsune", { action: "snooze", id: a.id, due_date: a.due_date });
    else if (a.action === "entered") await api("/api/kitsune", { action: "entered", lat: a.lat, lon: a.lon });
  }
  if (acts.length) await load();
}

// the widget's one tap (and #talk): Kitsune listens now, a tap anywhere keeps the note
window.inariTalk = async function () {
  await startTalk();
  if (rec.media) $("kTalkLabel").textContent = "Listening — tap anywhere to keep";
};

/* Rings: a reminder that comes back stays on screen until it is done or put off. */
const ring = { queue: [], es: null };

function ringShow() {
  const n = ring.queue[0];
  $("kRing").hidden = !n;
  if (!n) return;
  $("kRingText").textContent = words(n) || "Reminder #" + n.id;
  $("kRingMore").textContent = n.trigger_type === "geofence" ? "You are at " + (n.location_raw || "the place") + "." : stamp(n.due_date) + " · " + rel(n.due_date);
}

function ringHeard(n) {
  if (ring.queue.some((x) => x.id === n.id)) return;
  ring.queue.push(n);
  ringShow();
  say("Reminder: " + (words(n) || "#" + n.id));
  try { if (navigator.vibrate) navigator.vibrate([200, 100, 200]); } catch (e) {}
  try {
    if (window.Notification && Notification.permission === "granted" && document.hidden) new Notification("Kitsune", { body: words(n), tag: "kitsune-" + n.id });
  } catch (e) {}
  load();
}

function ringListen() {
  if (ring.es || !window.EventSource) return;
  ring.es = new EventSource("/api/kitsune/events");
  ring.es.addEventListener("ring", (ev) => {
    try { ringHeard(JSON.parse(ev.data)); } catch (e) {}
  });
}

$("kRing").addEventListener("click", async (ev) => {
  const b = ev.target.closest("button[data-act]");
  const n = ring.queue[0];
  if (!b || !n) return;
  const d = b.dataset.act === "later"
    ? await api("/api/kitsune", { action: "snooze", id: n.id, due_date: isoUtc(Math.round(Date.now() / 1000) + 3600) })
    : await api("/api/kitsune", { action: "dismiss", id: n.id });
  if (d.error) return;
  ring.queue.shift();
  ringShow();
  await load();
  say(b.dataset.act === "later" ? "It comes back in an hour." : "Done.");
});

// asked once, on a gesture, so a reminder can reach the person when the page is hidden
function askToNotify() {
  try { if (window.Notification && Notification.permission === "default") Notification.requestPermission(); } catch (e) {}
}
$("kTalk").addEventListener("pointerdown", askToNotify, { once: true });

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
  ringListen();
  await phoneActions();
  todayLoad();
  const start = ["assist", "lists", "runs", "week"].find((v) => hash.has(v));
  if (start) showView(start);
  if (hash.has("talk")) window.inariTalk();
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
