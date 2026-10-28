"use strict";
const PATHS = {
  sidebar: '<rect x="3" y="4" width="18" height="16" rx="3"/><path d="M9 4v16"/>', edit: '<path d="M12 20h9"/><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4Z"/>',
  search: '<circle cx="11" cy="11" r="7"/><path d="m20 20-3.5-3.5"/>', bars: '<path d="M4 20V11M10 20V5M16 20v-6M2 20h20"/>',
  gear: '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.7 1.7 0 0 0 .3 1.8l.1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-1.8-.3 1.7 1.7 0 0 0-1 1.5V21a2 2 0 1 1-4 0v-.1a1.7 1.7 0 0 0-1.1-1.5 1.7 1.7 0 0 0-1.8.3l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1a1.7 1.7 0 0 0 .3-1.8 1.7 1.7 0 0 0-1.5-1H3a2 2 0 1 1 0-4h.1a1.7 1.7 0 0 0 1.5-1.1 1.7 1.7 0 0 0-.3-1.8l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1a1.7 1.7 0 0 0 1.8.3H9a1.7 1.7 0 0 0 1-1.5V3a2 2 0 1 1 4 0v.1a1.7 1.7 0 0 0 1 1.5 1.7 1.7 0 0 0 1.8-.3l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1a1.7 1.7 0 0 0-.3 1.8V9a1.7 1.7 0 0 0 1.5 1H21a2 2 0 1 1 0 4h-.1a1.7 1.7 0 0 0-1.5 1Z"/>',
  plus: '<path d="M12 5v14M5 12h14"/>', stop: '<rect x="6" y="6" width="12" height="12" rx="2" fill="currentColor"/>', mic: '<rect x="9" y="3" width="6" height="11" rx="3"/><path d="M5 11a7 7 0 0 0 14 0M12 18v3"/>', send: '<path d="M12 19V5M5 12l7-7 7 7"/>',
  expand: '<path d="M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7"/>', fold: '<rect x="3" y="4" width="18" height="16" rx="3"/><path d="M15 4v16M8 10l2 2-2 2"/>',
  panel: '<rect x="3" y="4" width="18" height="16" rx="3"/><path d="M15 4v16"/>', chevron: '<path d="m6 9 6 6 6-6"/>', more: '<circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/>',
  file: '<path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9Z"/><path d="M14 3v6h6"/>', image: '<rect x="3" y="3" width="18" height="18" rx="2"/><circle cx="9" cy="9" r="2"/><path d="m21 15-5-5L5 21"/>',
  code: '<path d="m16 18 6-6-6-6M8 6l-6 6 6 6"/>', doc: '<path d="M14 3H6a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9Z"/><path d="M14 3v6h6M8 13h8M8 17h5"/>',
  tool: '<path d="M14.7 6.3a4 4 0 0 0-5.4 5.4L3 18l3 3 6.3-6.3a4 4 0 0 0 5.4-5.4l-2.5 2.5-2.4-.6-.6-2.4Z"/>', terminal: '<path d="m4 17 6-6-6-6M12 19h8"/>',
  move: '<path d="M5 12h14M13 6l6 6-6 6"/>', fork: '<circle cx="6" cy="5" r="2"/><circle cx="18" cy="5" r="2"/><circle cx="12" cy="19" r="2"/><path d="M6 7v2a3 3 0 0 0 3 3h6a3 3 0 0 0 3-3V7M12 12v5"/>',
  download: '<path d="M12 4v11M7 10l5 5 5-5M5 20h14"/>', spinner: '<path d="M12 3a9 9 0 1 0 9 9"/>',
  merge: '<circle cx="6" cy="19" r="2"/><circle cx="6" cy="5" r="2"/><circle cx="18" cy="12" r="2"/><path d="M6 7v10M6 7a7 7 0 0 0 10 5"/>',
};
const icon = (n) => `<svg class="i" viewBox="0 0 24 24" aria-hidden="true">${PATHS[n] || ""}</svg>`;
document.querySelectorAll("[data-icon]").forEach((e) => e.insertAdjacentHTML("afterbegin", icon(e.dataset.icon)));
const GLYPH = {idle:"·", listening:"◐", loading:"⇣", inference:"⟳", success:"✓", error:"!"}, LABEL = {idle:t("idle"), listening:t("listening"), loading:t("loading the model"), inference:t("working"), success:t("done"), error:t("error")};
const STEPS = ["planned","frozen","offered","transferred","verified","ready","committing","committed"];
const $ = (id) => document.getElementById(id), esc = (t) => String(t).replace(/[&<>"]/g, (c) => ({"&":"&amp;","<":"&lt;",">":"&gt;",'"':"&quot;"}[c]));
const store = { get: (k, d) => { try { return JSON.parse(localStorage.getItem("inari-" + k)) ?? d; } catch (e) { return d; } }, set: (k, v) => { try { localStorage.setItem("inari-" + k, JSON.stringify(v)); } catch (e) {} } };
/* Signing in exchanges the owner's token or a pass for a session cookie this script cannot read: from then on the
   page keeps nothing secret and puts nothing secret in a URL. A link may carry one (#token=… or #pass=…), and older
   pages kept them in the browser's storage: either is exchanged once, then forgotten. */
const PAGE = (document.querySelector('meta[name="inari-page"]') || {}).content || ""; // the core writes the version of this page there
let stream = null, lastSnapAt = 0, offlineTick = null, pendingDraft = null;
let snap = null, selected = null, tab = "artifacts", events = [], files = null, viewing = null, forkAt = null, drag = null, termHist = [], termPos = 0, pendingModel = "", pendingCore = "", attachments = [];
const loadHist = {}, shut = store.get("shut", {}), feed = [], seen = new Set(); let firstSnap = true, tlStamp = 0, tlBusy = false, lastFiles = "";

/* ---------- families, badges, quotas ---------- */
// Local distillations first: a Qwen distilled from Opus ("Q3.8-OPUS-27B…") is a Qwen, not Claude.
const FAMILIES = [["qwen","qwen|q3\\.|q2\\.|heretic|rvn|opus-27b","#6b4fbb","Q"],["claude","claude|anthropic|sonnet|opus|haiku","#d97757","C"],["openai","gpt|codex|^o\\d|openai","#10a37f","O"],["gemini","gemini","#4285f4","G"],["gemma","gemma|g4-","#1a73e8","g"],["llama","llama","#0a66c2","L"],["mistral","mistral|codestral|devstral|magistral","#ff7000","M"],["glm","glm|zhipu|umans","#2c5bff","Z"],["kimi","kimi|moonshot","#16a085","K"],["deepseek","deepseek","#4d6bfe","D"],["hammer","hammer|coderfim","#b8612a","H"]];
function family(model) { const m = String(model || "").toLowerCase(); for (const [id, re, c, g] of FAMILIES) if (new RegExp(re).test(m)) return { id, c, g }; return { id: "local", c: "#8a8494", g: "·" }; }
// Each core may name its models (`model_labels`: [suffix, label] pairs), e.g. which GPU a slot loads on.
function modelLabels() { return snap ? snap.nodes.flatMap((n) => (n.capabilities && n.capabilities.model_labels) || []) : []; }
const shortModel = (m) => { let s = String(m || "").replace(/^[^/]+\//, ""); for (const [suf, label] of modelLabels()) if (suf && s.endsWith(suf)) { s = s.slice(0, -suf.length) + label; break; } return s || t("default model"); };
function badge(model, size) {
  const f = family(model), el = document.createElement("span"); el.className = "mb"; if (size) { el.style.width = el.style.height = size + "px"; }
  const hasLogo = snap && (snap.usage || []).some((u) => u.provider_id === f.id && u.logo);
  if (hasLogo) el.innerHTML = `<img alt="" src="/api/logo/${encodeURIComponent(f.id)}">`; else { el.style.background = f.c; el.textContent = f.g; }
  el.title = f.id; return el;
}
function usageFor(model) { const f = family(model).id; return snap && (snap.usage || []).find((u) => u.provider_id === f || (f === "openai" && u.provider_id === "codex")); }
function pctOf(u) { return u && u.windows && u.windows.length ? Math.max(...u.windows.map((w) => w.used_percent)) : null; }
function ring(model) { const p = pctOf(usageFor(model)); if (p == null) return null; const r = document.createElement("span"); r.className = "ring"; r.style.setProperty("--p", p); r.style.setProperty("--rc", p > 85 ? "var(--bad)" : p > 60 ? "var(--warn)" : "var(--copper)"); r.innerHTML = "<i></i>"; r.title = t("{pct}% used", { pct: Math.round(p) }); return r; }
function harnessesOf(node) { const n = snap.nodes.find((x) => x.node_id === node); return (n && n.capabilities && n.capabilities.harnesses) || {}; }
// A core may keep some of its models out of the menus (`model_hidden` suffixes): helpers its harness uses on its own.
function harnessModels(node, h) { const n = snap.nodes.find((x) => x.node_id === node), hide = (n && n.capabilities && n.capabilities.model_hidden) || []; return (harnessesOf(node)[h] || []).filter((m) => !hide.some((suf) => suf && m.endsWith(suf))); }
// What each harness announces besides its models: modes (plan, auto-edit…) and slash commands.
function harnessModes(node, h) { const n = snap.nodes.find((x) => x.node_id === node); return ((n && n.capabilities && n.capabilities.harness_modes) || {})[h] || []; }
function harnessCommands(node, h) { const n = snap.nodes.find((x) => x.node_id === node); return ((n && n.capabilities && n.capabilities.harness_commands) || {})[h] || []; }
function nodeModels(node) { const n = snap.nodes.find((x) => x.node_id === node), hide = (n && n.capabilities && n.capabilities.model_hidden) || []; return ((n && n.capabilities && n.capabilities.models) || []).filter((m) => !hide.some((suf) => suf && m.endsWith(suf))); }

// Every call answers an object: the reply, or {error} after telling the person what went wrong.
async function api(path, body) { const headers = { "Content-Type": "application/json" };
  let r; try { r = await fetch(path, { method: body ? "POST" : "GET", headers, body: body ? JSON.stringify(body) : undefined }); } catch (e) { toast(t("The core cannot be reached. Check the connection and try again.")); return { error: "offline" }; }
  const data = await r.json().catch(() => ({}));
  if (r.status === 401) showLogin(t("Your session has ended. Sign in again."));
  else if (!r.ok) toast(data.error ? t("The core says: {error}", { error: data.error }) : t("The core answered with an error ({status}).", { status: r.status }));
  return r.ok ? data : { ...data, error: data.error || String(r.status) }; }
function showLogin(why) { if (stream) { stream.close(); stream = null; }
  $("loginWhy").textContent = why || ""; $("app").hidden = true; $("offline").hidden = true; $("login").hidden = false; $("tokenInput").value = ""; $("tokenInput").focus(); }
function toast(t) { const e = $("toast"); e.textContent = t; e.style.display = "block"; clearTimeout(toast.h); toast.h = setTimeout(() => (e.style.display = "none"), 4200); }
function log(text, tone) { feed.unshift({ t: new Date(), text, tone: tone || "" }); feed.length = Math.min(feed.length, 30); }
function theme(v) { if (v === "auto") document.documentElement.removeAttribute("data-theme"); else document.documentElement.setAttribute("data-theme", v); store.set("theme", v); }
theme(store.get("theme", "auto"));
// Behind a gate that adds the access itself (the phone's way in), or on the core's own machine, the page needs no
// sign-in: it asks who it is first and only shows the login when the core does not know it.
async function connect() {
  const hash = new URLSearchParams(location.hash.slice(1)), linked = hash.get("pass") || hash.get("token"), kept = legacySecret();
  if (location.hash) history.replaceState(null, "", location.pathname);
  if (linked) { const r = await signIn(linked); if (!r.ok) { showLogin(r.why); return; } } // a link that fails says why
  else if (kept) await signIn(kept); // a stale secret from an older page is simply dropped
  forgetLegacy();
  const who = await fetch("/api/whoami").then((r) => (r.ok ? r.json() : null)).catch(() => null);
  if (!who) { $("login").hidden = false; $("tokenInput").focus(); return; }
  $("login").hidden = true; $("app").hidden = false; layout(); openStream(); restoreDraft();
}
function legacySecret() { try { return JSON.parse(localStorage.getItem("inari-pass")) || sessionStorage.getItem("inari-token") || ""; } catch (e) { return ""; } }
function forgetLegacy() { try { localStorage.removeItem("inari-pass"); sessionStorage.removeItem("inari-token"); } catch (e) {} }
async function signIn(secret) {
  const r = await fetch("/api/session", { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ secret }) }).catch(() => null);
  if (!r) return { ok: false, why: t("The core cannot be reached. Check the connection and try again.") };
  if (r.status === 429) { const d = await r.json().catch(() => ({})); return { ok: false, why: t("Too many attempts. Try again in {s} s.", { s: d.retry_after || 60 }) }; }
  return r.ok ? { ok: true } : { ok: false, why: t("This token or pass is not known by this core.") };
}
const signOut = () => fetch("/api/logout", { method: "POST", headers: { "Content-Type": "application/json" }, body: "{}" }).catch(() => null).then(() => location.reload());
function openStream() { const es = (stream = new EventSource("/events"));
  es.onmessage = (e) => { snap = JSON.parse(e.data); lastSnapAt = Date.now(); online(true); render(); };
  es.onopen = () => online(true);
  es.onerror = () => { online(false); if (es.readyState === EventSource.CLOSED && stream === es) setTimeout(async () => { if (stream !== es) return; await api("/api/whoami"); if (stream === es) openStream(); /* a refused access has shown the login instead */ }, 3000); };
}
function online(ok) { const bar = $("offline"); $("app").style.opacity = ok ? "1" : ".75"; clearInterval(offlineTick);
  if (ok) { bar.hidden = true; return; }
  const say = () => { const s = lastSnapAt ? Math.round((Date.now() - lastSnapAt) / 1000) : 0, age = !lastSnapAt ? "" : s < 60 ? `${s} s` : `${Math.round(s / 60)} min`;
    bar.innerHTML = "<span>" + esc(t("Connection to the core lost — reconnecting…")) + "</span>" + (age ? "<small>" + esc(t("last update {age} ago", { age })) + "</small>" : ""); };
  say(); bar.hidden = false; offlineTick = setInterval(say, 5000); }
/* A draft survives the page reloading itself for a new version. */
function saveDraft() { try { sessionStorage.setItem("inari-draft", JSON.stringify({ agent: selected, text: $("msg").value })); } catch (e) {} }
function restoreDraft() { try { pendingDraft = JSON.parse(sessionStorage.getItem("inari-draft")); sessionStorage.removeItem("inari-draft"); } catch (e) { pendingDraft = null; } if (pendingDraft && pendingDraft.text) $("msg").value = pendingDraft.text; }
// One field for both: the core knows whether it is a pass or the owner's token.
$("tokenInput").onkeydown = (e) => { if (e.key === "Enter") $("tokenGo").click(); };
$("tokenGo").onclick = async () => { const v = $("tokenInput").value.trim(); if (!v) return; $("tokenGo").disabled = true;
  const r = await signIn(v); $("tokenGo").disabled = false; if (!r.ok) { $("loginWhy").textContent = r.why; return; } $("tokenInput").value = ""; $("loginWhy").textContent = ""; connect(); };

/* ---------- layout: sidebar, artifacts pane (fold / expand / resize) ---------- */
function layout() {
  const app = $("app"); app.classList.toggle("noside", !!store.get("noside", false)); app.classList.toggle("nobench", !!store.get("nobench", false)); app.classList.toggle("wbmax", !!store.get("wbmax", false));
  app.style.setProperty("--benchw", store.get("benchw", "42vw")); $("benchBtn").classList.toggle("on", !app.classList.contains("nobench")); requestAnimationFrame(drawTethers);
  $("sideToggle").classList.toggle("on", !app.classList.contains("noside")); placeFox();
}
$("sideToggle").onclick = () => morph(() => { store.set("noside", !store.get("noside", false)); layout(); });
/* The mascot is one element that moves (its frame is never reloaded): in the sidebar's slot when the sidebar
   is open, at the top centre of the page when it is shut — it fades out, moves, and fades back in. */
/* The mascot is one element (its frame is never reloaded) that glides and scales between the sidebar's slot
   and the top centre of the page, following the panes while they move. */
const FOX = 240;
/* While a thread is carried, the mascot is a place to drop it: if its slot has scrolled out of view (a phone, a long
   list), it comes to the top of the screen for the time of the drag, then goes back. */
function foxTarget() { const home = foxHome(); if (!(drag && drag.active)) return home;
  const seen = home.y + home.size * 0.6 > 56 && home.y + home.size * 0.4 < innerHeight; if (seen) return home;
  const size = Math.min(140, innerWidth * 0.36); return { x: innerWidth / 2 - size / 2, y: 60, size }; }
function foxHome() { const app = $("app"), mobile = matchMedia("(max-width:760px)").matches, open = document.body.classList.contains("open");
  if (mobile && open) { const st = $("stage").getBoundingClientRect(), back = $("back").getBoundingClientRect();
    if (st.height >= 72) { const size = st.height; return { x: innerWidth / 2 - size / 2, y: st.top, size }; } // its stage, big enough to see its face
    const size = 56; return { x: innerWidth / 2 - size / 2, y: Math.max(0, back.top + back.height / 2 - size / 2), size }; }
  if (!mobile && app.classList.contains("noside")) { const m = $("main").getBoundingClientRect(), size = open ? 96 : 160; return { x: m.left + m.width / 2 - size / 2, y: open ? 0 : 6, size }; }
  const slot = $("foxSlot").getBoundingClientRect(), size = Math.min(slot.height, slot.width); return { x: slot.left + slot.width / 2 - size / 2, y: slot.top, size }; }
function placeFox() { const fox = $("fox"); if ($("app").hidden) return; const t = foxTarget();
  fox.style.transform = `translate(${t.x}px, ${t.y}px) scale(${t.size / FOX})`; fox.classList.add("shown"); }
/* Follow the layout frame by frame while a pane glides, then settle. */
function followFox(ms) { const end = performance.now() + ms; const step = () => { placeFox(); if (performance.now() < end) requestAnimationFrame(step); }; requestAnimationFrame(step); }
/* Freeze the page content's width to the wider of before and after while a pane moves. */
function morph(flip) { const app = $("app"), main = $("main"), before = main.clientWidth;
  app.classList.add("measuring"); flip(); const after = main.clientWidth; flip(); void main.offsetWidth; app.classList.remove("measuring"); flip(); // the final width, measured without animation
  app.style.setProperty("--freeze", Math.max(before, after) + "px"); app.classList.add("morphing"); followFox(460);
  clearTimeout(morph.t); morph.t = setTimeout(() => { app.classList.remove("morphing"); placeFox(); }, 420); }
addEventListener("resize", () => placeFox()); window.addEventListener("scroll", () => placeFox(), { passive: true });
/* The Android app keeps the page at full height and tells it the keyboard's height as it moves: the
   composer rides it with a transform (no layout per frame); the thread makes room once it settles. */
window.inariKeyboard = (h) => { h = Math.max(0, Math.round(h)); const root = document.documentElement, th = $("thread"), atEnd = th.scrollHeight - th.scrollTop - th.clientHeight < 60;
  root.style.setProperty("--kb", h + "px"); const up = h > 40; if (up !== document.body.classList.contains("kb")) { document.body.classList.toggle("kb", up); followFox(320); }
  clearTimeout(window.inariKeyboard.t); window.inariKeyboard.t = setTimeout(() => { root.style.setProperty("--kbpad", h + "px"); if (atEnd) th.scrollTop = th.scrollHeight; placeFox(); }, 90); };
$("benchClose").onclick = () => { if (innerWidth <= 1100) { store.set("nobench", true); document.body.classList.remove("bench-open"); layout(); return; } store.set("wbmax", false); if (!store.get("nobench", false)) morph(() => { store.set("nobench", !store.get("nobench", false)); layout(); }); else layout(); };
$("benchMobile").onclick = () => { document.body.classList.add("bench-open"); store.set("nobench", false); layout(); };
$("benchBtn").onclick = () => { if (innerWidth <= 1100) document.body.classList.add("bench-open"); else morph(() => { store.set("nobench", !store.get("nobench", false)); layout(); }); };
$("wbmax").onclick = () => { store.set("wbmax", !store.get("wbmax", false)); layout(); };
$("split").onpointerdown = (e) => { e.preventDefault(); const move = (m) => { const w = Math.min(innerWidth - 420, Math.max(360, innerWidth - m.clientX)); $("app").style.setProperty("--benchw", w + "px"); store.set("benchw", w + "px"); }; const up = () => { removeEventListener("pointermove", move); removeEventListener("pointerup", up); }; addEventListener("pointermove", move); addEventListener("pointerup", up); };

function absorb() {
  const keys = []; for (const n of snap.notices || []) keys.push(["n" + JSON.stringify(n), n]); for (const r of snap.results || []) keys.push(["r" + r.request, r]); for (const a of snap.approvals || []) keys.push(["a" + a.request_id, a]);
  for (const [k, v] of keys) { if (seen.has(k)) continue; seen.add(k); if (firstSnap) continue;
    if (k[0] === "r") { log(v.ok ? t("Teleport committed") : t("Teleport {state}: {error}", { state: t(v.state || "refused"), error: v.error || "" }), v.ok ? "ok" : "bad"); if (!v.ok) toast(t("Teleport {state}", { state: t(v.state || "refused") })); }
    else if (k[0] === "a") { log(t("Approval requested: {command}", { command: v.command }), "ask"); fox("event", "notify"); }
    else if (v.subject === "agent.created") log(v.ok ? t("Created {agent} on {node}", { agent: v.agent, node: v.node }) : t("Create failed: {error}", { error: v.error }), v.ok ? "ok" : "bad");
    else if (v.subject === "agent.merged") log(v.ok ? t("Merged {source} into {target}", { source: v.source, target: v.target }) : t("Merge failed: {error}", { error: v.error }), v.ok ? "ok" : "bad");
    else if (v.stopped) { log(`${v.agent} ${v.stopped === "done" ? t("finished") : t(v.stopped)}${v.error ? ": " + v.error : ""}`, v.stopped === "done" ? "ok" : "bad"); if (v.stopped !== "done") fox("event", "notify"); } }
  firstSnap = false;
}
function render() {
  if (snap.ui && snap.ui.page && PAGE && PAGE !== "__INARI_" + "PAGE__" && snap.ui.page !== PAGE) { saveDraft(); location.reload(); return; } // a newer page is served
  if (pendingDraft) { const d = pendingDraft; pendingDraft = null; if (d.agent && snap.agents[d.agent] && !selected) { const text = $("msg").value; select(d.agent).then(() => { if (!$("msg").value) $("msg").value = text; }); } }
  absorb(); $("brandfoot").textContent = (snap.ui && snap.ui.footer) || ""; $("brandName").textContent = (snap.ui && snap.ui.brand) || ""; renderPassBanner(); if (!$("fox").classList.contains("shown")) placeFox();
  for (const n of snap.nodes) { const h = (loadHist[n.node_id] = loadHist[n.node_id] || []); h.push((snap.load[n.node_id] || {}).gpu || 0); if (h.length > 40) h.shift(); }
  const agents = Object.entries(snap.agents), busy = agents.filter(([, a]) => a.phase !== "idle").length;
  $("stats").textContent = t("{n} agents · {busy} working", { n: agents.length, busy });
  if (!drag && !coreHold) renderCores(agents);
  renderFeed(); renderApprovals(); renderTop(); renderPill(); renderUsageCard(); mascot(); drawOrbParams(snap.mascot);
  if (selected && Date.now() - tlStamp > 1500) refreshDetail(); else if (!selected) { drawChat(); drawComposer(); if (!$("bbody").firstChild) drawBench(); }
}
const inFlight = (mv) => mv && !["committed", "rolled_back"].includes(mv.state);
function row(id, a, mv) {
  const el = document.createElement("div"); el.className = "row" + (selected === id ? " selected" : "") + (inFlight(mv) ? " moving" : ""); el.dataset.agent = id; el.dataset.phase = a.phase; el.dataset.node = a.node; el.tabIndex = 0;
  el.setAttribute("aria-label", t("{title}, {phase}, on {node}", { title: a.title || id, phase: LABEL[a.phase], node: a.node })); el.innerHTML = `<span class="pdot"></span><span class="t"></span>`; el.querySelector(".t").textContent = a.title || id; if (a.pinned) el.querySelector(".t").insertAdjacentHTML("beforebegin", `<span class="pin" title="${esc(t("Pinned"))}" style="color:var(--copper);margin-right:4px">★</span>`);
  const pp = a.pass && (snap.passes || []).find((x) => x.id === a.pass); if (pp) el.insertAdjacentHTML("beforeend", `<span class="ptag" title="${esc(t(pp.role + " pass"))}">${esc(pp.name)}</span>`); el.title = (a.last || "") + "\n" + t("{events} events · {files} files", { events: a.events ?? 0, files: a.files ?? 0 });
  if (inFlight(mv)) el.insertAdjacentHTML("beforeend", `<span class="prog">→ ${esc(mv.target)} ${Math.round(Math.max(0, STEPS.indexOf(mv.state)) / 7 * 100)}%</span>`); else el.append(badge(a.model || ""), ...(ring(a.model || "") ? [ring(a.model || "")] : []));
  if (!inFlight(mv) && !(snap.ui && snap.ui.pass) && snap.nodes.filter((n) => n.node_id !== a.node && n.role === "heavy").length) {
    const mvb = document.createElement("button"); mvb.className = "rmove"; mvb.type = "button"; mvb.innerHTML = icon("move"); mvb.title = t("Move to another core (T)"); mvb.setAttribute("aria-label", t("Move {title} to another core", { title: a.title || id }));
    mvb.onpointerdown = (e) => e.stopPropagation(); mvb.onclick = (e) => { e.stopPropagation(); openMove(id, a.node); }; el.append(mvb); }
  el.onpointerdown = (e) => startDrag(e, el, id, a.node); el.onkeydown = (e) => { if (e.key === "t" || e.key === "T") openMove(id, a.node); if (e.key === "Enter") select(id); }; return el;
}
function renderCores(agents) {
  const box = $("cores"); box.textContent = ""; const asleep = agents.filter(([, a]) => a.archived); agents = agents.filter(([, a]) => !a.archived);
  $("sleepers").hidden = !asleep.length; $("sleepers").textContent = t("Sleeping {n}", { n: asleep.length });

  if (!snap.nodes.length) { box.insertAdjacentHTML("beforeend", `<div class="empty-core">${esc(t("Waiting for a core to announce itself…"))}</div>`); return; }
  if (snap.nodes.length > 1 && agents.length && !store.get("tip-gestures", false) && !(snap.ui && snap.ui.pass)) { const tip = document.createElement("div"); tip.className = "tip"; tip.setAttribute("role", "note");
    tip.innerHTML = `<p>${t("Drag a thread onto another core to move it there, or use its arrow. <b>+</b> on a core starts a thread on it. Press <kbd>?</kbd> for shortcuts.")}</p><button type="button" class="btn small">${esc(t("Got it"))}</button>`;
    tip.querySelector("button").onclick = () => { store.set("tip-gestures", true); tip.remove(); }; box.append(tip); }
  for (const n of snap.nodes) {
    const load = (snap.load[n.node_id] || {}).gpu || 0, mine = agents.filter(([, a]) => a.node === n.node_id), c = document.createElement("div");
    c.className = "core" + (shut[n.node_id] ? " shut" : ""); c.dataset.node = n.node_id;
    c.innerHTML = `<div class="chead"><span class="chev">${icon("chevron")}</span><span class="dot"></span><b></b><span class="pct">${Math.round(load * 100)}% · ${mine.length}</span><button type="button" class="cnew" title="${esc(t("New thread on this core"))}">${icon("plus")}</button></div><div class="bar"><i style="width:${Math.round(load * 100)}%"></i></div><div class="rows"></div>`;
    c.querySelector("b").textContent = n.node_id; const cn = c.querySelector(".cnew"); cn.setAttribute("aria-label", t("New thread on {node}", { node: n.node_id })); cn.onpointerdown = (e) => e.stopPropagation(); cn.onclick = (e) => { e.stopPropagation(); newThreadOn(n.node_id); };
    c.querySelector(".chead").onclick = () => { shut[n.node_id] = !shut[n.node_id]; store.set("shut", shut); c.classList.toggle("shut"); requestAnimationFrame(drawTethers); };
    const rows = c.querySelector(".rows"); for (const [id, a] of [...mine].sort((x, y) => (y[1].pinned ? 1 : 0) - (x[1].pinned ? 1 : 0))) rows.append(row(id, a, snap.moves[id]));
    for (const mv of Object.values(snap.moves)) if (mv.target === n.node_id && mv.source !== n.node_id && mv.state !== "committed") rows.insertAdjacentHTML("beforeend", `<div class="row moving"><span class="pdot" style="background:var(--copper)"></span><span class="t">⇢ ${esc(mv.agent)}</span><span class="prog">${esc(t(mv.state === "rolled_back" ? "rolled back" : mv.state))}</span></div>`);
    if (!rows.children.length) rows.innerHTML = `<div class="empty-core">${esc(t("Drop a thread here"))}</div>`; box.append(c);
  }
  requestAnimationFrame(drawTethers);
}
function drawTethers() {
  const svg = $("tethers"); svg.textContent = "";
  for (const mv of Object.values((snap && snap.moves) || {})) { if (!inFlight(mv)) continue;
    const a = document.querySelector(`.core[data-node="${CSS.escape(mv.source)}"]`), b = document.querySelector(`.core[data-node="${CSS.escape(mv.target)}"]`); if (!a || !b) continue;
    const ra = a.getBoundingClientRect(), rb = b.getBoundingClientRect(), x = ra.right - 6, y1 = ra.top + ra.height / 2, y2 = rb.top + rb.height / 2; svg.insertAdjacentHTML("beforeend", `<path d="M${x},${y1} C${x + 60},${y1} ${x + 60},${y2} ${x},${y2}"/>`); }
}
function renderFeed() { $("feed").innerHTML = feed.length ? feed.slice(0, 6).map((e) => `<div class="${e.tone}"><time>${e.t.toTimeString().slice(0, 5)}</time>${esc(e.text)}</div>`).join("") : `<div class="empty">${esc(t("Moves, approvals and finished turns show here."))}</div>`; $("feed").style.display = feed.length ? "block" : "none"; }
/* A pass holder always sees who can read its threads: supervision is said, never hidden. */
function renderPassBanner() { const p = snap.ui && snap.ui.pass, b = $("passBanner"); document.body.classList.toggle("guest", !!p); b.hidden = !p; if (!p) return;
  const owner = esc(snap.ui.owner || t("the owner of this core"));
  b.innerHTML = p.role === "supervision" ? t("<b>Supervised</b> — {owner} can read these threads and answers the agents' requests.", { owner }) : t("<b>Shared</b> — {owner} sees these threads and can take part.", { owner });
  const out = document.createElement("button"); out.className = "btn small"; out.textContent = t("Leave this pass"); out.onclick = signOut; b.append(out); }
function renderApprovals() {
  const sig = selected + "|" + snap.approvals.map((r) => r.request_id).join(","); if (sig === renderApprovals.sig) return; renderApprovals.sig = sig;
  const box = $("approvals"), here = $("askbox"); box.textContent = ""; here.textContent = ""; const guest = snap.ui && snap.ui.pass;
  const card = (r, wide) => { const d = document.createElement("div"); d.className = "ask glass"; d.setAttribute("role", "alert");
    if (guest && guest.role === "supervision") { d.innerHTML = `<strong>${esc(t("Waiting for approval"))}</strong><code></code>`; d.querySelector("code").textContent = r.command; return d; }
    d.innerHTML = `<strong>${esc(t("The agent asks to run"))}</strong><code></code><div class="row2"><button class="btn" data-a="no">${esc(t("Deny"))}</button><button class="btn primary" data-a="yes">${esc(t("Allow once"))}</button>` + (!guest && wide ? `<button class="btn" data-a="auto" title="${esc(t("Stop asking in this thread"))}">${esc(t("Always allow here"))}</button>` : "") + "</div>";
    d.querySelector("code").textContent = r.command + (wide ? "" : " · " + r.agent_id);
    const answer = (ok) => { d.querySelectorAll("button").forEach((b) => (b.disabled = true)); d.style.opacity = ".6"; return api("/api/approval", { request_id: r.request_id, approved: ok }); };
    d.querySelector('[data-a="no"]').onclick = () => answer(false); d.querySelector('[data-a="yes"]').onclick = () => answer(true);
    const al = d.querySelector('[data-a="auto"]'); if (al) al.onclick = async () => { await api("/api/auto", { agent: r.agent_id, auto: true }); answer(true); toast(t("Auto mode on for this thread")); };
    return d; };
  let fresh = false;
  for (const r of snap.approvals) {
    if (selected && r.agent_id === selected) { here.append(card(r, true)); fresh = true; }
    else box.append(card(r, false)); }
  if (fresh && !renderApprovals.seen) { const th = $("thread"); th.scrollTop = th.scrollHeight; }
  renderApprovals.seen = fresh;
}
/* Auto mode: the owner lets a thread act without asking; the chip shows and flips it. Never for a pass. */
function fillAutoChip(a) { const chip = $("autoChip"), guest = snap.ui && snap.ui.pass; chip.hidden = !a || !!guest; if (chip.hidden) return;
  chip.classList.toggle("on", !!a.auto); chip.textContent = a.auto ? "⚡ " + t("Auto") : t("Ask first"); chip.title = a.auto ? t("This thread acts without asking. Click to ask again.") : t("The agent asks before sensitive actions. Click to let it act without asking."); }
$("autoChip").onclick = () => { const a = snap.agents[selected]; if (!a) return; api("/api/auto", { agent: selected, auto: !a.auto }); toast(a.auto ? t("The agent will ask again") : t("Auto mode: the agent acts without asking")); };
/* Harness updates, as the old sidebar badge: a download mark and the logos of what is out of date, on every core. */
const HARNESS = { claude: ["claude", "Claude Code"], gemini: ["gemini", "Gemini CLI"], openfox: ["local", "OpenFox"] };
function harnessName(h) { return (HARNESS[h] || [h, h])[1]; }
function updateEntries() { const out = [], jobs = snap.update_jobs || {};
  for (const n of snap.nodes || []) for (const [h, u] of Object.entries((n.capabilities || {}).updates || {})) { const job = jobs[n.node_id + "/" + h], v = { ...u, ...(job || {}) }; out.push({ node: n.node_id, harness: h, ...v }); }
  return out; }
function renderPill() {
  const p = $("pill"), all = updateEntries(), stale = all.filter((e) => e.available || e.state === "running" || e.state === "failed"); p.textContent = "";
  p.classList.toggle("busy", all.some((e) => e.state === "running")); p.insertAdjacentHTML("beforeend", `<svg class="i dl" viewBox="0 0 24 24" aria-hidden="true">${PATHS[all.some((e) => e.state === "running") ? "spinner" : "download"]}</svg>`);
  for (const h of [...new Set(stale.map((e) => e.harness))]) { const b = badge((HARNESS[h] || [h])[0], 16); b.title = harnessName(h); p.append(b); }
  const names = [...new Set(stale.map((e) => harnessName(e.harness)))]; p.title = names.length ? t(names.length > 1 ? "{names}: updates available" : "{names}: update available", { names: names.join(", ") }) : t("Harness updates");
  p.style.display = stale.length ? "flex" : "none"; if ($("pop").classList.contains("open") && $("pop").dataset.for === "pill") updatesPanel($("pop"), true);
}
function updatesPanel(pop, refresh) { if (refresh) pop.innerHTML = ""; pop.insertAdjacentHTML("beforeend", `<h3>${esc(t("Harness updates"))}</h3>`); const all = updateEntries(), shown = all.filter((e) => e.available || e.state === "running" || e.state === "failed" || e.state === "done");
  if (!shown.length) pop.insertAdjacentHTML("beforeend", `<div class="empty">${esc(t("Every harness is up to date."))}</div>`);
  for (const e of shown) { const row = document.createElement("div"); row.className = "uprow"; row.append(badge((HARNESS[e.harness] || [e.harness])[0], 20));
    row.insertAdjacentHTML("beforeend", `<div><b>${esc(harnessName(e.harness))}</b> <small>${esc(e.node)}</small><br><small>${esc(e.current || "?")}${e.latest && e.latest !== e.current ? " → " + esc(e.latest) : ""}</small></div>`);
    const b = document.createElement("button"); b.className = "btn small go"; b.textContent = t(e.state === "running" ? "Updating…" : e.state === "done" && !e.available ? "Updated" : e.state === "failed" ? "Retry" : "Update"); b.disabled = e.state === "running" || (e.state === "done" && !e.available);
    b.onclick = () => { b.disabled = true; b.textContent = t("Updating…"); api("/api/update", { node: e.node, harness: e.harness }); }; row.append(b);
    if (e.state === "failed" && e.error) row.insertAdjacentHTML("beforeend", `<div class="err">${esc(e.error)}</div>`);
    if (e.state === "done" && !e.available) row.insertAdjacentHTML("beforeend", `<div class="note">${esc(t("In use from the next message."))}</div>`); pop.append(row); }
  const row = document.createElement("div"); row.className = "row2"; row.style.marginTop = "8px"; const chk = document.createElement("button"); chk.className = "btn small"; chk.textContent = t("Check now"); chk.onclick = () => { chk.disabled = true; chk.textContent = t("Checking…"); api("/api/update/check", {}); }; row.append(chk);
  const idle = shown.filter((e) => e.available && e.state !== "running"); if (idle.length > 1) { const allb = document.createElement("button"); allb.className = "btn small go"; allb.textContent = t("Update all"); allb.onclick = () => { allb.disabled = true; for (const e of idle) api("/api/update", { node: e.node, harness: e.harness }); }; row.append(allb); }
  pop.append(row); }
/* Provider usage opens in the sidebar, as the old footer card did, not over the page. */
function renderUsageCard() { const open = store.get("usageOpen", false); $("usageCard").hidden = !open; $("usageBtn").setAttribute("aria-expanded", String(open)); if (open) { const body = $("usageBody"); body.textContent = ""; usageRows(body); } }
function openPop(anchor, build) { const pop = $("pop"); if (pop.classList.contains("open") && pop.dataset.for === anchor.id) { pop.classList.remove("open"); return; } pop.innerHTML = ""; build(pop); pop.dataset.for = anchor.id; const r = anchor.getBoundingClientRect(); pop.classList.add("open");
  pop.style.left = Math.max(8, Math.min(r.left, innerWidth - pop.offsetWidth - 8)) + "px"; pop.style.top = Math.max(8, r.top - pop.offsetHeight - 8 > 8 ? r.top - pop.offsetHeight - 8 : r.bottom + 8) + "px"; }
addEventListener("pointerdown", (e) => { const pop = $("pop"); if (pop.classList.contains("open") && !pop.contains(e.target) && !e.target.closest("#gear,#pill,#modelChip,#modeChip,#ctxChip,#autoChip,#pickModel,#coreChip,#actionsBtn")) pop.classList.remove("open"); });
function usageRows(pop) { const items = snap.usage || [];
  if (!items.length) pop.insertAdjacentHTML("beforeend", `<div class="empty">${esc(t("No quota source is configured. Local models have no quota."))}</div>`);
  for (const u of items) { const row = document.createElement("div"); row.className = "urow"; const head = document.createElement("div"); head.className = "uhead"; head.append(badge(u.provider_id, 22)); head.insertAdjacentHTML("beforeend", `<b>${esc(u.name)}</b><small>${esc(u.plan || (u.status === "unlimited" ? t("no quota") : u.status))}</small>`); row.append(head);
    for (const w of u.windows) { const hot = w.used_percent > 85 ? " hot" : w.used_percent > 60 ? " warm" : ""; row.insertAdjacentHTML("beforeend", `<div class="uhead"><span>${esc(w.label)}</span><small>${Math.round(w.used_percent)}%</small></div><div class="meter${hot}"><i style="width:${Math.min(100, w.used_percent)}%"></i></div>${w.resets_at ? `<div class="rs">${esc(t("resets {when}", { when: new Date(w.resets_at).toLocaleString(LANG, { weekday: "short", hour: "2-digit", minute: "2-digit" }) }))}</div>` : ""}`); }
    pop.append(row); } }
$("usageBtn").onclick = () => { store.set("usageOpen", !store.get("usageOpen", false)); renderUsageCard(); }; $("usageFold").onclick = () => { store.set("usageOpen", false); renderUsageCard(); };
$("pill").onclick = () => openPop($("pill"), updatesPanel);
$("gear").onclick = () => openPop($("gear"), (pop) => { pop.insertAdjacentHTML("beforeend", `<h3>${esc(t("Appearance"))}</h3>`); const seg = document.createElement("div"); seg.className = "seg"; for (const th of ["auto", "light", "dark"]) { const b = document.createElement("button"); b.className = "btn small" + (store.get("theme", "auto") === th ? " on" : ""); b.textContent = t(th); b.onclick = () => { theme(th); pop.classList.remove("open"); }; seg.append(b); } pop.append(seg);
  pop.insertAdjacentHTML("beforeend", `<h3 style="margin-top:12px">${esc(t("Language"))}</h3>`); const langs = document.createElement("div"); langs.className = "seg"; for (const [code, name] of Object.entries(LANGS)) { const b = document.createElement("button"); b.className = "btn small" + (LANG === code ? " on" : ""); b.textContent = name; b.lang = code; b.onclick = () => { if (code === LANG) return; setLang(code); saveDraft(); location.reload(); }; langs.append(b); } pop.append(langs);
  pop.insertAdjacentHTML("beforeend", `<h3 style="margin-top:12px">${esc(t("Session"))}</h3>`); const out = document.createElement("button"); out.className = "btn small"; out.textContent = t("Sign out"); out.onclick = signOut; pop.append(out); });

/* ---------- the mascot: FoxBot when its folder is served, else the orb ---------- */
let foxFrame = null, foxReady = false, foxLast = "";
function fox(method, a) { try { const f = foxFrame && foxFrame.contentWindow && foxFrame.contentWindow.fox; if (!f) return false; if (method === "event") f.setEvent && f.setEvent(a); else if (method === "state") { f.setHarness && f.setHarness(a.harness); f.setHealth && f.setHealth(a.health); f.setActivity && f.setActivity(a.activity); } return true; } catch (e) { return false; } }
function mountFox() {
  const box = $("fox"); if (!(snap && snap.ui && snap.ui.mascot)) { if (!box.firstChild) { const c = document.createElement("canvas"); c.id = "orb"; c.width = c.height = 300; c.setAttribute("role", "img"); c.setAttribute("aria-label", t("Network activity indicator")); box.append(c); orbInit(c); } return; }
  if (foxFrame) return; foxFrame = document.createElement("iframe"); foxFrame.title = t("Mascot"); foxFrame.src = "/fox/index.html"; foxFrame.onload = () => { foxReady = true; foxLast = ""; try { foxFrame.contentWindow.fox.setReducedMotion(matchMedia("(prefers-reduced-motion: reduce)").matches); } catch (e) {} mascot(); }; box.textContent = ""; box.append(foxFrame);
}
function mascot() {
  mountFox(); if (!foxReady) return; const phases = Object.values(snap.agents).map((a) => a.phase), sel = selected && snap.agents[selected], errors = phases.filter((p) => p === "error").length;
  const activity = snap.approvals.length ? "waiting" : phases.some((p) => p === "inference" || p === "loading" || p === "listening") ? "working" : "idle"; const f = family((sel && sel.model) || "").id;
  const harness = f === "claude" ? "claude" : f === "kimi" ? "kimi" : "local"; const st = { activity, harness, health: Math.max(20, 100 - errors * 20) }, key = JSON.stringify(st); if (key !== foxLast) { fox("state", st); foxLast = key; }
}
const COPPER = "#e48a46"; let orbCtx = null, orbEl = null, P = { colour: COPPER, pulse_hz: .25, turbulence: 0, glow: .2 }, cur = { ...P };
function drawOrbParams(m) { P = { ...m, colour: m.colour === "#22d3ee" ? COPPER : m.colour }; }
function orbInit(c) { orbEl = c; orbCtx = c.getContext("2d"); requestAnimationFrame(frame); }
function frame(t) { if (!orbEl || !orbEl.isConnected) return; const ctx = orbCtx, w = orbEl.width, c = w / 2, s = t / 1000, still = matchMedia("(prefers-reduced-motion: reduce)").matches;
  cur.pulse_hz += (P.pulse_hz - cur.pulse_hz) * .05; cur.turbulence += (P.turbulence - cur.turbulence) * .05; cur.glow += (P.glow - cur.glow) * .05; ctx.clearRect(0, 0, w, w);
  const r0 = w * .27 * (still ? 1 : 1 + .06 * Math.sin(s * 2 * Math.PI * cur.pulse_hz)), halo = ctx.createRadialGradient(c, c, r0 * .6, c, c, w * .5); halo.addColorStop(0, P.colour + Math.round(40 + cur.glow * 120).toString(16).padStart(2, "0")); halo.addColorStop(1, P.colour + "00"); ctx.fillStyle = halo; ctx.fillRect(0, 0, w, w); ctx.beginPath();
  for (let i = 0; i <= 120; i++) { const a = i / 120 * 2 * Math.PI, n = still ? 0 : cur.turbulence * w * .035 * (Math.sin(a * 3 + s * 1.7) + Math.sin(a * 5 - s * 2.3) * .6); const x = c + Math.cos(a) * (r0 + n), y = c + Math.sin(a) * (r0 + n); i ? ctx.lineTo(x, y) : ctx.moveTo(x, y); }
  ctx.closePath(); const g = ctx.createRadialGradient(c - r0 * .3, c - r0 * .3, r0 * .1, c, c, r0 * 1.2); g.addColorStop(0, "#fff8f0cc"); g.addColorStop(.35, P.colour); g.addColorStop(1, P.colour + "55"); ctx.fillStyle = g; ctx.fill(); requestAnimationFrame(frame); }

/* ---------- top bar, models, composer ---------- */
function renderTop() {
  const a = selected && snap.agents[selected], app = $("app"); app.classList.toggle("nosel", !a); if ($("ptitle").contentEditable !== "true") $("ptitle").textContent = a ? (a.title || selected) : composing ? t("New thread · {core}", { core: pendingCore || "" }) : "Inari";
  $("pphase").innerHTML = a ? `<span class="state${["listening", "loading", "inference"].includes(a.phase) ? " live" : ""}" style="--c:var(--${{ inference: "copper", loading: "copper", listening: "purple", success: "ok", error: "bad" }[a.phase] || "dim"})">${LABEL[a.phase]} <small>${esc(t("on {node}", { node: a.node }))}</small></span>` : "";
  $("actionsBtn").hidden = !a; fillModeChip(a); fillCtxChip(a); fillAutoChip(a); const chip = $("modelChip"); chip.hidden = !a; if (a) fillModelChip(chip, a.model); document.body.classList.toggle("open", !!a || composing); placeFox(); drawComposer(); drawStop();
}
function fillModeChip(a) { const chip = $("modeChip"), modes = a ? harnessModes(a.node, a.harness) : []; chip.hidden = !modes.length; if (chip.hidden) return;
  const cur = modes.find((m) => m.id === a.mode) || modes[0]; chip.textContent = ""; const n = document.createElement("span"); n.className = "n"; n.textContent = cur.name; chip.title = cur.description || ""; chip.append(n); }
/* How full the agent's context is, as it reports it: a ring and a percentage; the popover has the numbers. */
function fillCtxChip(a) { const chip = $("ctxChip"), c = a && a.context; chip.hidden = !(c && c.size); if (chip.hidden) return;
  const pct = Math.min(100, Math.round((c.used / c.size) * 100)); chip.textContent = ""; const r = document.createElement("span"); r.className = "ring"; r.style.setProperty("--p", pct); r.style.setProperty("--rc", pct > 85 ? "var(--bad)" : pct > 60 ? "var(--warn)" : "var(--copper)"); r.innerHTML = "<i></i>";
  const n = document.createElement("span"); n.className = "n"; n.textContent = pct + "%"; chip.append(r, n); chip.title = t("{used} of {size} tokens", { used: c.used.toLocaleString(LANG), size: c.size.toLocaleString(LANG) }); }
$("ctxChip").onclick = () => { const a = snap.agents[selected], c = a && a.context; if (!c) return; const pct = Math.min(100, Math.round((c.used / c.size) * 100));
  openPop($("ctxChip"), (pop) => { pop.insertAdjacentHTML("beforeend", `<h3>${esc(t("Context"))}</h3><div class="urow"><div class="uhead"><span>${esc(t("{pct}% used", { pct }))}</span><span>${esc(t("{used} / {size} tokens", { used: c.used.toLocaleString(LANG), size: c.size.toLocaleString(LANG) }))}</span></div><div class="ubar"><i style="width:${pct}%"></i></div></div>${c.cost != null ? `<div class="empty" style="padding:6px 0">${esc(t("Spent this session: ${cost}", { cost: Number(c.cost).toFixed(2) }))}</div>` : ""}`);
    const compact = harnessCommands(a.node, a.harness).find((x) => x.name === "compact");
    if (compact) { const b = document.createElement("button"); b.className = "mrow"; b.textContent = t("Compact the conversation (/compact)"); b.onclick = () => { $("pop").classList.remove("open"); api("/api/message", { agent: selected, text: "/compact" }); toast(t("Compacting…")); }; pop.append(b); }
    else pop.insertAdjacentHTML("beforeend", `<div class="empty" style="padding:6px 0">${esc(t("The agent compacts by itself when the window fills."))}</div>`); }); };
$("modeChip").onclick = () => { const a = snap.agents[selected]; if (!a) return; const modes = harnessModes(a.node, a.harness);
  openPop($("modeChip"), (pop) => { pop.insertAdjacentHTML("beforeend", `<h3>${esc(t("{harness} — mode", { harness: harnessName(a.harness) }))}</h3><div id="mlist"></div>`); const list = pop.querySelector("#mlist");
    for (const m of modes) { const b = document.createElement("button"); b.className = "mrow" + (m.id === (a.mode || modes[0].id) ? " on" : ""); const nm = document.createElement("span"); nm.textContent = m.name; b.append(nm);
      if (m.description) { const d = document.createElement("small"); d.style.cssText = "margin-left:auto;color:var(--faint);max-width:55%;text-align:right"; d.textContent = m.description; b.append(d); }
      b.onclick = () => { $("pop").classList.remove("open"); api("/api/mode", { agent: selected, mode: m.id }); toast(t("Mode: {name}", { name: m.name })); }; list.append(b); } }); };
function fillModelChip(chip, model) { chip.textContent = ""; chip.append(badge(model, 18)); const n = document.createElement("span"); n.className = "n"; n.textContent = shortModel(model); chip.append(n); const r = ring(model); if (r) chip.append(r); }
function drawComposer() {
  const a = selected && snap.agents[selected], cc = $("coreChip"), pm = $("pickModel"); $("msg").placeholder = a ? t("Message the agent…") : t("Build anything…");
  const own = snap.nodes.find((n) => n.node_id === snap.ui.self && n.role === "heavy"), core = a ? a.node : (pendingCore || (own || snap.nodes.find((n) => n.role === "heavy") || {}).node_id || ""); if (!a) pendingCore = core; cc.textContent = "⌂ " + (core || t("no core")); cc.style.display = a ? "none" : "";
  fillModelChip(pm, a ? a.model : pendingModel); if (!a && pendingHarness) { const n = pm.querySelector(".n"); if (n) n.textContent = harnessName(pendingHarness) + " · " + (pendingModel ? shortModel(pendingModel) : t("default")); } $("attached").innerHTML = attachments.map((f) => `<span class="chip" style="cursor:default">📎 ${esc(f.name)}</span>`).join("");
}
function modelPicker(anchor, node, current, onPick, harness) {
  openPop(anchor, (pop) => { pop.insertAdjacentHTML("beforeend", `<h3>${esc(harness ? t("{harness} — model", { harness: harnessName(harness) }) : t("Model"))}</h3><input id="mfilter" placeholder="${esc(t("Filter models…"))}" aria-label="${esc(t("Filter models"))}"><div id="mlist"></div>`); const list = pop.querySelector("#mlist"), models = [...new Map((harness ? harnessModels(node, harness) : nodeModels(node)).map((m) => [shortModel(m), m])).values()], rank = (m) => FAMILIES.findIndex(([id]) => id === family(m).id) >>> 0;
    models.sort((a, b) => rank(a) - rank(b) || shortModel(a).localeCompare(shortModel(b)));
    const draw = (q) => { list.textContent = ""; const all = [""].concat(models).filter((m) => !q || shortModel(m).toLowerCase().includes(q)); let lastF = null;
      for (const m of all) { const f = m ? family(m).id : "default"; if (f !== lastF) { list.insertAdjacentHTML("beforeend", `<div class="fam">${m ? f : esc(t("harness default"))}</div>`); lastF = f; }
        const b = document.createElement("button"); b.className = "mrow" + (m === (current || "") ? " on" : ""); b.append(badge(m, 20)); const n = document.createElement("span"); n.textContent = m ? shortModel(m) : t("Default model"); b.append(n); const r = ring(m); if (r) { r.style.marginLeft = "auto"; b.append(r); } b.onclick = () => { $("pop").classList.remove("open"); onPick(m); }; list.append(b); }
      if (!models.length) list.insertAdjacentHTML("beforeend", `<div class="empty" style="padding:8px">${esc(t("This core did not list its models."))}</div>`); }; draw(""); pop.querySelector("#mfilter").oninput = (e) => draw(e.target.value.toLowerCase()); });
}
$("modelChip").onclick = () => { const a = snap.agents[selected]; modelPicker($("modelChip"), a.node, a.model, (m) => { api("/api/model", { agent: selected, model: m }); toast(t("Model: {name}", { name: shortModel(m) })); }, a.harness); };
$("pickModel").onclick = () => { const a = selected && snap.agents[selected]; if (!a) return chooseSession(pendingCore); modelPicker($("pickModel"), a.node, a.model, (m) => api("/api/model", { agent: selected, model: m }), a.harness); };
/* The step that opens a new thread: which agent, then which of its models. */
let pendingHarness = "";
function chooseSession(node, then) { const hs = Object.keys(harnessesOf(node));
  const pickModel = (h) => { pendingHarness = h; pendingModel = ""; drawComposer();
    const models = harnessModels(node, h); if (!models.length) { if (then) then(); return; }
    modelPicker($("pickModel"), node, "", (m) => { pendingModel = m; drawComposer(); if (then) then(); }, h); };
  if (hs.length <= 1) return pickModel(hs[0] || "");
  sheet(t("New thread on {node} — which agent?", { node }), hs.map((h) => [t("{harness} · {n} models", { harness: harnessName(h), n: new Set(harnessModels(node, h).map(shortModel)).size || "?" }), () => pickModel(h)]), t("Then choose its model; you can change both later.")); }
$("coreChip").onclick = () => { const heavy = snap.nodes.filter((n) => n.role === "heavy"); sheet(t("Run it on…"), heavy.map((n) => [n.node_id, () => { pendingCore = n.node_id; pendingHarness = ""; pendingModel = ""; drawComposer(); chooseSession(n.node_id); }])); };
/* Typing "/" offers the commands the thread's agent announces (sent as plain text, as the agent expects). */
(() => { const box = document.createElement("div"); box.id = "slash"; box.hidden = true; box.setAttribute("role", "listbox"); box.style.cssText = "position:absolute;left:12px;right:12px;bottom:calc(100% + 6px);max-height:240px;overflow:auto;background:var(--panel-solid);border:1px solid var(--hair);border-radius:14px;box-shadow:var(--shadow);padding:6px;z-index:60";
  $("composer").style.position = "relative"; $("composer").append(box);
  const cmds = () => { const a = selected && snap.agents[selected]; return a ? harnessCommands(a.node, a.harness) : (pendingHarness ? harnessCommands(pendingCore, pendingHarness) : []); };
  const hide = () => { box.hidden = true; };
  $("msg").addEventListener("input", () => { const v = $("msg").value, m = /^\/(\S*)$/.exec(v); if (!m || !snap) return hide(); const q = m[1].toLowerCase(), list = cmds().filter((c) => c.name.toLowerCase().includes(q)).slice(0, 40); if (!list.length) return hide();
    box.textContent = ""; for (const c of list) { const b = document.createElement("button"); b.type = "button"; b.className = "mrow"; b.setAttribute("role", "option"); const n = document.createElement("b"); n.textContent = "/" + c.name; b.append(n);
      if (c.description) { const d = document.createElement("small"); d.style.cssText = "margin-left:10px;color:var(--faint);overflow:hidden;text-overflow:ellipsis;white-space:nowrap"; d.textContent = c.description; b.append(d); }
      b.onclick = () => { $("msg").value = "/" + c.name + " "; hide(); $("msg").focus(); }; box.append(b); } box.hidden = false; });
  $("msg").addEventListener("keydown", (e) => { if (e.key === "Escape") hide(); }); $("msg").addEventListener("blur", () => setTimeout(hide, 150)); })();
$("attach").onclick = () => $("file").click();
$("file").onchange = (e) => { for (const f of e.target.files) { if (f.size > 25e6) { toast(t("{name} is over 25 MB", { name: f.name })); continue; } attachments.push({ name: f.name, file: f }); } e.target.value = ""; drawComposer(); };
// A file goes in parts of 512 KiB: no large request, and a part that failed is simply sent again.
const PART = 512 * 1024;
function b64of(buf) { let out = ""; const bytes = new Uint8Array(buf); for (let i = 0; i < bytes.length; i += 32768) out += String.fromCharCode.apply(null, bytes.subarray(i, i + 32768)); return btoa(out); }
async function sendFile(agent, f) { const id = "up" + Date.now().toString(36) + Math.random().toString(36).slice(2, 8), parts = Math.max(1, Math.ceil(f.file.size / PART));
  for (let n = 0; n < parts; n++) { const at = n * PART, b64 = b64of(await f.file.slice(at, at + PART).arrayBuffer()); let reply = null;
    for (let tries = 0; tries < 3 && !(reply && reply.ok !== false); tries++) reply = await api("/api/upload", { agent, path: f.name, upload_id: id, offset: at, total: f.file.size, b64 }).catch(() => null);
    if (!reply || reply.ok === false) { toast(f.name + ": " + ((reply && reply.error) || t("upload failed"))); return false; } }
  return true; }
// The Android app hands what was shared to it (a photo, a PDF, a text): into the composer, ready to send.
window.inariShare = (p) => { const names = [];
  for (const f of (p && p.files) || []) { const bin = atob(f.b64), bytes = new Uint8Array(bin.length); for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i); attachments.push({ name: f.name, file: new File([bytes], f.name, { type: f.mime }) }); names.push(f.name); }
  if (p && p.text) { const m = $("msg"); m.value = (m.value ? m.value + "\n" : "") + p.text; }
  try { drawComposer(); } catch (e) {} if (names.length) toast(t("Added to the message: {names}", { names: names.join(", ") })); $("msg").focus(); };
const SR = window.SpeechRecognition || window.webkitSpeechRecognition; if (SR) { $("mic").hidden = false; let rec = null; $("mic").onclick = () => { if (rec) { rec.stop(); return; } rec = new SR(); rec.interimResults = false; rec.lang = navigator.language; rec.onresult = (e) => { $("msg").value += (($("msg").value ? " " : "") + e.results[0][0].transcript); }; rec.onend = () => { rec = null; $("mic").classList.remove("on"); }; $("mic").classList.add("on"); rec.start(); }; }
$("composer").onsubmit = async (e) => { e.preventDefault(); let text = $("msg").value.trim(); if (!text && !attachments.length) return; $("sendBtn").disabled = true;
  try { let id = selected;
    if (!id && !pendingHarness && Object.keys(harnessesOf(pendingCore)).length > 1) { $("sendBtn").disabled = false; return chooseSession(pendingCore, () => $("composer").requestSubmit()); }
    if (!id) { const core = pendingCore; if (!core) return toast(t("No core available")); id = (text.split(/\s+/).slice(0, 4).join("-").replace(/[^\w-]+/g, "") || "thread").slice(0, 28) + "-" + (Date.now() % 1000); const made = await api("/api/agent", { thread_id: id, node: core, title: text.slice(0, 40), ...(pendingHarness ? { harness: pendingHarness } : {}), ...(pendingModel ? { model: pendingModel } : {}) }); id = made.thread_id || id; composing = false;
      // the model travels with the thread's creation: set before its first turn reads it
      for (let i = 0; i < 40 && !(snap.agents[id]); i++) await new Promise((r) => setTimeout(r, 150)); pendingHarness = ""; pendingModel = ""; await select(id); }
    const sent = []; for (const f of attachments) if (await sendFile(id, f)) sent.push(f.name); attachments = [];
    $("msg").value = ""; await api("/api/message", { agent: id, text: text.trim(), attachments: sent }); tlStamp = 0; setTimeout(() => { $("thread").scrollTop = $("thread").scrollHeight; }, 60);
  } finally { $("sendBtn").disabled = false; } };
$("msg").onkeydown = (e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); $("composer").requestSubmit(); } };

/* ---------- conversation ---------- */
async function select(id) { selected = id; forkAt = null; viewing = null; files = null; events = []; tlStamp = 0; lastFiles = ""; renderTop(); drawChat(); drawBench(); await refreshDetail(); if (snap) renderCores(Object.entries(snap.agents)); $("msg").focus(); }
async function refreshDetail() { if (!selected || tlBusy) return; tlBusy = true; tlStamp = Date.now(); try { const id = selected, base = `/api/agent/${encodeURIComponent(id)}/timeline`, last = events.length ? events[events.length - 1].id : "";
    let tl = await api(last ? `${base}?after=${encodeURIComponent(last)}` : base); if (id !== selected) return;
    // a plain continuation is appended; a journal that changed under the page (a merge, a fork) is asked for whole
    if (last && !(tl.from === events.length && events.length + (tl.events || []).length === tl.count)) tl = await api(base);
    if (id !== selected || tl.error) return; events = tl.from ? events.concat(tl.events || []) : tl.events || []; const a = snap.agents[selected] || {}, sig = a.files + ":" + a.events; if (!files || sig !== lastFiles) { files = (await api(`/api/agent/${encodeURIComponent(selected)}/files`)).files || []; lastFiles = sig; } drawChat(); drawBench(true); } finally { tlBusy = false; } }
/* Markdown, the subset models write: headings, lists, quotes, tables, rules, links, code. Everything is
   escaped first; only http(s) links become anchors. One renderer for the chat and the viewer. */
const LINK = 'target="_blank" rel="noopener noreferrer"';
function inlineMd(s) {
  return esc(s).replace(/`([^`]+)`/g, "<code>$1</code>").replace(/\*\*([^*]+)\*\*/g, "<b>$1</b>").replace(/(^|[^*\w])\*(\S(?:[^*\n]*\S)?)\*(?![*\w])/g, "$1<i>$2</i>")
    .replace(/\[([^\]]+)\]\((https?:\/\/[^\s)]+)\)/g, `<a href="$2" ${LINK}>$1</a>`).replace(/(^|[\s(])(https?:\/\/[^\s<)]+[^\s<).,;:!?])/g, `$1<a href="$2" ${LINK}>$2</a>`);
}
function mdBlocks(src) {
  const lines = src.split("\n"), out = []; let para = [], list = null, quote = [];
  const flush = () => { if (para.length) out.push("<p>" + para.map(inlineMd).join("<br>") + "</p>"); para = [];
    if (list) out.push(`<${list.t}>` + list.items.map((x) => `<li>${inlineMd(x)}</li>`).join("") + `</${list.t}>`); list = null;
    if (quote.length) out.push(`<blockquote>${mdBlocks(quote.join("\n"))}</blockquote>`); quote = []; };
  const cells = (l) => l.trim().replace(/^\||\|$/g, "").split("|").map((c) => c.trim());
  for (let i = 0; i < lines.length; i++) { const l = lines[i]; let m;
    if (!l.trim()) { flush(); continue; }
    if ((m = l.match(/^>\s?(.*)$/))) { if (para.length || list) flush(); quote.push(m[1]); continue; } else if (quote.length) flush();
    if ((m = l.match(/^(#{1,4})\s+(.*)$/))) { flush(); const h = m[1].length + 2; out.push(`<h${h} class="mh">${inlineMd(m[2])}</h${h}>`); continue; }
    if (/^\s*([-*_])(\s*\1){2,}\s*$/.test(l)) { flush(); out.push("<hr>"); continue; }
    if (/^\s*\|.*\|\s*$/.test(l) && /^\s*\|?\s*:?-{3,}/.test(lines[i + 1] || "")) { flush(); const head = cells(l), rows = []; i += 2;
      while (i < lines.length && /^\s*\|.*\|\s*$/.test(lines[i])) rows.push(cells(lines[i++])); i--;
      out.push(`<div class="tbl"><table><thead><tr>${head.map((c) => `<th>${inlineMd(c)}</th>`).join("")}</tr></thead><tbody>${rows.map((r) => `<tr>${r.map((c) => `<td>${inlineMd(c)}</td>`).join("")}</tr>`).join("")}</tbody></table></div>`); continue; }
    if ((m = l.match(/^\s*(?:([-*+])|(\d+)[.)])\s+(.*)$/))) { const t = m[1] ? "ul" : "ol"; if (para.length) flush(); if (list && list.t !== t) flush(); list = list || { t, items: [] }; list.items.push(m[3]); continue; }
    if (list && /^\s{2,}\S/.test(l)) { list.items[list.items.length - 1] += " " + l.trim(); continue; }
    if (list) flush(); para.push(l); }
  flush(); return out.join("");
}
function md(text) { return String(text).replace(/\r\n?/g, "\n").trim().split("```").map((p, i) => { if (!(i % 2)) return mdBlocks(p);
  const lang = (p.match(/^([\w+#.-]*)\n/) || [])[1] || "", code = p.replace(/^[\w+#.-]*\n/, "").replace(/\n$/, "");
  return `<div class="cb">${lang ? `<span class="lang">${esc(lang)}</span>` : ""}<pre><code>${esc(code)}</code></pre><button type="button" class="cpy" aria-label="${esc(t("Copy the code"))}">${esc(t("Copy"))}</button></div>`; }).join(""); }
/* Copy buttons on code blocks, wherever they are rendered. */
document.addEventListener("click", async (e) => { const b = e.target.closest(".cb .cpy"); if (!b) return; const text = b.parentElement.querySelector("code").textContent;
  try { await navigator.clipboard.writeText(text); } catch (err) { const ta = document.createElement("textarea"); ta.value = text; document.body.append(ta); ta.select(); document.execCommand("copy"); ta.remove(); }
  b.textContent = t("Copied"); b.classList.add("done"); setTimeout(() => { b.textContent = t("Copy"); b.classList.remove("done"); }, 1400); });
/* The thread is rebuilt off-screen, then only what changed is swapped in: an open tool call stays open,
   a selection stays selected, and a screen reader hears only the new messages. */
function reconcile(box, out) {
  const next = [...out.children];
  next.forEach((n, i) => { const cur = box.children[i];
    if (cur && (n.dataset.k ? cur.dataset.k === n.dataset.k && cur.dataset.s === n.dataset.s : !cur.dataset.k && cur.outerHTML === n.outerHTML)) return;
    if (cur && cur.dataset.k && cur.dataset.k === n.dataset.k && cur.open) n.open = true;
    if (cur) box.replaceChild(n, cur); else box.append(n); });
  while (box.children.length > next.length) box.lastElementChild.remove();
}
function drawChat() {
  const shown = $("msgs"), th = $("thread"), atEnd = th.scrollHeight - th.scrollTop - th.clientHeight < 90, before = shown.children.length, box = document.createElement("div"), fresh = drawChat.thread !== selected; drawChat.thread = events.length ? selected : null;
  if (!selected) { shown.textContent = ""; shown.innerHTML = `<div class="hero"><h2>${esc(t("What should we build?"))}</h2><p>${esc(t("Pick a thread on the left, or describe a task below. It runs on the core you choose and can move to another one at any time."))}</p></div>`; $("jump").hidden = true; return; }
  const a = snap.agents[selected] || {};
  if (!events.filter((e) => e.kind !== "system" && e.kind !== "provider_switch").length) { box.innerHTML = `<div class="hero"><h2>${esc(t("What should this agent do?"))}</h2><p>${esc(t("It works in its own sandboxed workspace. Anything it creates appears under Artifacts."))}</p></div>`; }
  let lastNote = "";
  const note = (html) => { if (html === lastNote) return; lastNote = html; box.insertAdjacentHTML("beforeend", `<div class="note">${html}</div>`); };
  for (let i = 0; i < events.length; i++) { const e = events[i];
    if (e.kind === "user_message" || e.kind === "assistant_message") { lastNote = ""; const d = document.createElement("div"); d.className = "msg " + (e.kind === "user_message" ? "user" : "assistant"); d.dataset.event = d.dataset.k = e.id; d.innerHTML = md(e.text); box.append(d); }
    else if (e.kind === "tool_call") { lastNote = ""; const r = events[i + 1] && events[i + 1].kind === "tool_result" ? events[++i] : null, err = r && /^(denied|\w+Error)/.test(r.text);
      const d = document.createElement("details"); d.className = "tool" + (err ? " err" : ""); d.dataset.k = e.id; d.dataset.s = r ? r.id : ""; const shell = /^(\$ |bash|execute|shell|run)/.test(e.text);
      d.innerHTML = `<summary>${icon(shell ? "terminal" : "tool")}<span class="tn"></span><span class="ts">${esc(t(r ? (err ? "failed" : "done") : "running…"))}</span><span class="cv">${icon("chevron")}</span></summary><pre></pre>`;
      d.querySelector(".tn").textContent = e.text; d.querySelector("pre").textContent = r ? r.text : t("running…");
      const path = (e.text.match(/(?:write_file|read_file|write|edit|create)[^:]*:\s*([\w./-]+\.\w+)/i) || [])[1]; if (path) { const b = document.createElement("button"); b.className = "btn small open"; b.innerHTML = icon("file") + "<span></span>"; b.lastChild.textContent = t("Open {path}", { path }); b.onclick = () => openFile(path); d.append(b); } box.append(d); }
    else if (e.kind === "provider_switch") note(t("Model <b>{name}</b>", { name: esc(shortModel(e.text)) }));
    else if (e.kind === "decision") note(esc(e.text));
    else if (e.kind === "checkpoint") { const [why, ...rest] = e.text.split(": "); const map = { teleport: "Moved between cores", provider_error: "The model failed", harness_error: "The agent stopped", harness_timeout: "The agent took too long", model_refused: "Model not available", mode_changed: "Mode", stopped: "Stopped", auto_approved: "Allowed automatically", mode_refused: "Mode not available" }; note(esc(t(map[why] || "Checkpoint")) + (rest.length && why !== "teleport" ? ` — ${esc(rest.join(": ").slice(0, 120))}` : "")); } }
  if (["listening", "loading", "inference"].includes(a.phase)) box.insertAdjacentHTML("beforeend", `<div class="msg assistant typing" aria-label="${esc(t("The agent is working"))}"><i></i><i></i><i></i></div>`);
  if (fresh) { shown.setAttribute("aria-live", "off"); setTimeout(() => shown.setAttribute("aria-live", "polite"), 600); } /* a thread just opened is not read aloud whole */
  reconcile(shown, box); if (atEnd || fresh) { th.scrollTop = th.scrollHeight; $("jump").hidden = true; } else if (shown.children.length > before) $("jump").hidden = false;
}
$("jump").onclick = () => { const th = $("thread"); th.scrollTo({ top: th.scrollHeight, behavior: matchMedia("(prefers-reduced-motion: reduce)").matches ? "auto" : "smooth" }); $("jump").hidden = true; };
$("thread").addEventListener("scroll", () => { const th = $("thread"); if (th.scrollHeight - th.scrollTop - th.clientHeight < 90) $("jump").hidden = true; }, { passive: true });
$("back").onclick = () => { selected = null; composing = false; renderTop(); drawChat(); drawBench(); if (snap) renderCores(Object.entries(snap.agents)); };
$("newThread").onclick = () => newThreadOn(pendingCore || (snap && snap.ui && snap.ui.self) || "");
/* A new thread on a given core: the composer opens (on a phone too, where the page only shows with a thread), aimed at it. */
let composing = false;
function newThreadOn(node) { selected = null; composing = true; if (node) pendingCore = node; pendingHarness = ""; pendingModel = ""; renderTop(); drawChat(); drawBench(); if (snap) renderCores(Object.entries(snap.agents)); drawComposer();
  setTimeout(() => chooseSession(pendingCore, () => $("msg").focus()), 80); }
/* A long press on a core opens a new thread on it. */
let coreHold = null;
$("cores").addEventListener("pointerdown", (e) => { const head = e.target.closest(".chead"); if (!head) return; const node = head.closest(".core").dataset.node, x = e.clientX, y = e.clientY;
  const stop = () => { clearTimeout(coreHold && coreHold.t); coreHold = null; removeEventListener("pointerup", stop); removeEventListener("pointercancel", stop); removeEventListener("pointermove", far); };
  const far = (ev) => { if (Math.hypot(ev.clientX - x, ev.clientY - y) > 10) stop(); };
  coreHold = { t: setTimeout(() => { stop(); swallowUntil = Date.now() + 600; newThreadOn(node); }, 550) };
  addEventListener("pointerup", stop); addEventListener("pointercancel", stop); addEventListener("pointermove", far); });
$("searchBtn").onclick = () => { $("search").classList.toggle("on"); $("search").focus(); };
async function forkNow() { await api("/api/fork", { agent: selected, new_thread: selected + "-fork-" + Math.random().toString(36).slice(2, 8), upto: forkAt }); toast(t(forkAt ? "Forking {name} at the chosen event…" : "Forking {name}…", { name: selected })); }
function doMerge() { const me = snap.agents[selected], others = Object.entries(snap.agents).filter(([id, a]) => id !== selected && a.node === me.node); sheet(t("Merge which thread into {name}?", { name: selected }), others.map(([id]) => [id, () => api("/api/merge", { target: selected, source: id })]), others.length ? t("Both threads must run on the same core.") : t("No other thread on this core.")); }
$("actionsBtn").onclick = () => openPop($("actionsBtn"), (pop) => { const a = snap.agents[selected]; pop.insertAdjacentHTML("beforeend", `<h3>${esc(t("Thread"))}</h3>`); for (const [ic, label, fn] of [["move", t("Move to another core…"), () => openMove(selected, a.node)], ["fork", forkAt ? t("Fork from the chosen event") : t("Fork this thread"), forkNow], ["merge", t("Merge another thread into this one…"), doMerge]]) { const b = document.createElement("button"); b.className = "mrow"; b.innerHTML = icon(ic) + "<span></span>"; b.lastChild.textContent = label; b.onclick = () => { $("pop").classList.remove("open"); fn(); }; pop.append(b); } });

/* ---------- artifacts pane ---------- */
const KIND = { md: "doc", txt: "doc", py: "code", js: "code", ts: "code", json: "code", html: "code", css: "code", sh: "code", rs: "code", go: "code", png: "image", jpg: "image", jpeg: "image", gif: "image", svg: "image", webp: "image" }, MIME = { png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", gif: "image/gif", webp: "image/webp", svg: "image/svg+xml" }, ext = (p) => (p.split(".").pop() || "").toLowerCase();
document.querySelectorAll(".tab").forEach((t) => { t.onclick = () => { tab = t.dataset.tab; drawBench(); };
  t.onkeydown = (e) => { const all = [...document.querySelectorAll(".tab")].filter((x) => getComputedStyle(x).display !== "none"), i = all.indexOf(t), d = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0; if (!d) return; e.preventDefault(); const n = all[(i + d + all.length) % all.length]; n.focus(); n.click(); }; }); function setTab(n) { tab = n; drawBench(); }
async function openFile(path) { if (!selected) return; viewing = { path, ...(await api(`/api/agent/${encodeURIComponent(selected)}/file?path=${encodeURIComponent(path)}`)) }; store.set("nobench", false); layout(); document.body.classList.add("bench-open"); setTab("viewer"); }
function viewer(body) {
  if (!viewing) { body.innerHTML = `<p class="empty">${esc(t("Open a file from Artifacts, or from a tool call in the conversation, to see it here."))}</p>`; return; } const v = viewing, x = ext(v.path);
  body.innerHTML = `<div class="vhead"><h3></h3><button class="btn small" id="vcopy">${esc(t("Copy"))}</button><button class="btn small" id="vdl">${esc(t("Download"))}</button></div><div class="viewer"></div>`; body.querySelector("h3").textContent = `${v.path} · ${v.size} B`; const out = body.querySelector(".viewer");
  if (v.binary && MIME[x] && v.b64) out.innerHTML = `<img alt="${esc(v.path)}" src="data:${MIME[x]};base64,${v.b64}">`; else if (v.binary) out.innerHTML = `<p class="empty">${esc(t("Binary file — download it to open."))}</p>`;
  else if (x === "svg") out.innerHTML = `<img alt="${esc(v.path)}" src="data:image/svg+xml;base64,${btoa(unescape(encodeURIComponent(v.text)))}">`; else if (x === "html" || x === "htm") { out.innerHTML = `<iframe class="preview" sandbox="" title="${esc(t("Preview"))}"></iframe>`; out.querySelector("iframe").srcdoc = v.text; }
  else if (x === "md") out.innerHTML = `<div class="doc">${md(v.text)}</div>`; else { let txt = v.text; if (x === "json") { try { txt = JSON.stringify(JSON.parse(txt), null, 2); } catch (e) {} } out.innerHTML = `<pre class="code">${txt.split("\n").map((l) => `<span class="l">${esc(l)}</span>`).join("")}</pre>`; }
  $("vcopy").onclick = () => navigator.clipboard && navigator.clipboard.writeText(v.text || "").then(() => toast(t("Copied"))); $("vdl").onclick = () => { const blob = v.b64 ? new Blob([Uint8Array.from(atob(v.b64), (c) => c.charCodeAt(0))]) : new Blob([v.text]), u = URL.createObjectURL(blob), a = document.createElement("a"); a.href = u; a.download = v.path.split("/").pop(); a.click(); URL.revokeObjectURL(u); };
}
function terminalLines() { const out = []; for (let i = 0; i < events.length; i++) { const e = events[i]; if (e.kind !== "tool_call") continue; const r = events[i + 1] && events[i + 1].kind === "tool_result" ? events[i + 1] : null;
  let by = "act", cmd = e.text; const m = e.text.match(/^(?:bash|execute|shell|run)[^:]*:\s*(.*)$/s); if (e.text.startsWith("$ ")) { by = "you"; cmd = e.text.slice(2); } else if (m) { by = "agent"; cmd = m[1]; }
  out.push({ cmd, by, out: r ? r.text : "…", bad: r && /^(denied|\w+Error)/.test(r.text) }); } return out; }
function terminal(body) {
  body.style.padding = "10px"; body.innerHTML = `<div id="term"><div id="termout"></div><form id="termform"><span>$</span><input id="terminput" autocomplete="off" spellcheck="false" placeholder="${esc(t("Run a command in this agent's workspace"))}" aria-label="${esc(t("Terminal command"))}"></form></div>`;
  const o = $("termout"); o.innerHTML = terminalLines().map((l) => `<div class="cmd ${l.by === "agent" ? "ai" : l.by === "act" ? "act" : ""}">${l.by === "you" ? "$" : l.by === "agent" ? "agent $" : "agent ›"} ${esc(l.cmd)}</div><div class="out${l.bad ? " bad" : ""}">${esc(l.out)}</div>`).join("") || `<div class="out">${esc(t("Commands the agent runs appear here. Type one below to run it yourself — the agent will know what you did."))}</div>`; o.scrollTop = o.scrollHeight; const inp = $("terminput");
  $("termform").onsubmit = async (e) => { e.preventDefault(); const cmd = inp.value.trim(); if (!cmd) return; termHist.push(cmd); termPos = termHist.length; inp.value = ""; inp.disabled = true; await api("/api/exec", { agent: selected, cmd }); inp.disabled = false; tlStamp = 0; files = null; await refreshDetail(); drawBench(); const again = $("terminput"); if (again) again.focus(); };
  inp.onkeydown = (e) => { if (e.key === "ArrowUp" && termPos > 0) { inp.value = termHist[--termPos]; e.preventDefault(); } else if (e.key === "ArrowDown") inp.value = termHist[++termPos] || ""; };
}
function drawBench(soft) {
  document.querySelectorAll(".tab").forEach((t) => { const on = t.dataset.tab === tab; t.classList.toggle("on", on); t.setAttribute("aria-selected", on); t.tabIndex = on ? 0 : -1; }); const body = $("bbody"); body.setAttribute("aria-label", tab); if (soft && tab === "terminal" && document.activeElement && document.activeElement.id === "terminput") return;
  const keep = body.scrollTop; body.textContent = ""; body.style.padding = ""; if (!selected) { body.innerHTML = `<p class="empty">${esc(t("Select a thread to see its files, run commands, view results and read its journal."))}</p>`; return; }
  if (tab === "artifacts") { const hidden = !store.get("showgit", false); const shown = files ? files.filter((f) => !hidden || !f.path.startsWith(".git/")) : files; if (!files) { body.innerHTML = `<p class="empty">${esc(t("Loading…"))}</p>`; return; } if (!shown.length) { body.innerHTML = `<p class="empty">${esc(t("No files yet. Ask the agent to create something — its workspace travels with it when you move it."))}</p>`; return; } body.insertAdjacentHTML("beforeend", `<label class="empty" style="display:flex;gap:6px;align-items:center;margin-bottom:6px"><input type="checkbox" id="showgit" ${hidden ? "" : "checked"}> ${esc(t("show .git ({n} files)", { n: files.length - files.filter((f) => !f.path.startsWith(".git/")).length }))}</label><div class="files"></div>`); $("showgit").onchange = (e) => { store.set("showgit", e.target.checked); drawBench(); };
    for (const f of shown) { const b = document.createElement("button"); b.innerHTML = `<span class="ic">${icon(KIND[ext(f.path)] || "file")}</span><span></span><small></small>`; b.children[1].textContent = f.path; b.children[2].textContent = f.size + " B"; b.onclick = () => openFile(f.path); body.querySelector(".files").append(b); } }
  else if (tab === "terminal") terminal(body); else if (tab === "viewer") viewer(body);
  else { body.insertAdjacentHTML("beforeend", `<p class="empty">${esc(t("Tap an event to fork the thread from it, then press Fork."))}</p><ul id="tree"></ul>`); const tree = body.querySelector("#tree"); events.forEach((ev) => { const li = document.createElement("li"); li.className = ev.kind + (ev.id === forkAt ? " pick" : ""); const k = document.createElement("b"); k.textContent = ev.kind.replace("_", " "); li.append(k, " " + ev.text.slice(0, 160)); li.onclick = () => { forkAt = ev.id; tree.querySelectorAll(".pick").forEach((x) => x.classList.remove("pick")); li.classList.add("pick"); }; tree.append(li); }); }
  body.scrollTop = keep;
}

/* ---------- search, sheets, create, drag to teleport ---------- */
let hitTimer = null;
$("search").oninput = () => { clearTimeout(hitTimer); hitTimer = setTimeout(async () => { const q = $("search").value.trim(), box = $("hits"); box.textContent = ""; if (!q) { box.style.display = "none"; return; } const { hits = [] } = await api("/api/search?q=" + encodeURIComponent(q)); box.style.display = "block"; box.style.top = $("search").getBoundingClientRect().bottom + 4 + "px"; if (!hits.length) box.innerHTML = `<small class="empty">${esc(t("No match"))}</small>`; for (const h of hits) { const b = document.createElement("button"); b.innerHTML = "<span></span><small></small>"; b.firstChild.textContent = h.text.slice(0, 90); b.lastChild.textContent = h.agent + " · " + h.kind; b.onclick = () => { box.style.display = "none"; select(h.agent); }; box.append(b); } }, 300); };
function sheet(title, options, note, extra) { $("sheetTitle").textContent = title; const opts = $("sheetOpts"); opts.textContent = ""; if (extra) opts.append(extra); if (note) { const p = document.createElement("small"); p.className = "empty"; p.textContent = note; opts.append(p); } for (const [label, fn] of options) { const b = document.createElement("button"); b.className = "btn"; b.textContent = label; b.onclick = () => { $("sheet").classList.remove("open"); fn(); }; opts.append(b); } const c = document.createElement("button"); c.className = "btn"; c.textContent = options.length ? t("Cancel") : t("Close"); c.onclick = () => $("sheet").classList.remove("open"); opts.append(c); $("sheet").classList.add("open"); (extra || opts.querySelector("button")).focus(); }
/* Add a core: attach one you deployed (a VPS, a new VM, another workstation) through your
   private network, or a core pass (coming next). */
function addCore() { const n = (snap.passes || []).length;
  sheet(t("Add a core"), [[t("Attach a core you deployed"), attachCore], [t("Core pass for a person"), corePass], ...(n ? [[t("Passes ({n})", { n }), passList]] : [])],
  t("A deployed core joins through your private network (Tailscale, WireGuard, an SSH tunnel). A pass lets a person have threads on your core: you see them.")); }
/* A core pass: a person gets threads on this core. Supervision (a child): you follow every thread and answer
   the agents' requests, local models by default. Collaboration: you both see and act on the shared threads. */
function corePass() { const names = [...new Set(snap.nodes.flatMap((n) => Object.keys((n.capabilities || {}).harnesses || {})))];
  const box = document.createElement("div"); box.className = "passform";
  box.innerHTML = `<label for="passName">${esc(t("Person"))}</label><input id="passName" type="text" maxlength="40" placeholder="${esc(t("First name"))}" autocomplete="off">
    <label><input type="radio" name="passRole" value="supervision" checked> ${esc(t("Supervision — a child: you follow the threads and approve"))}</label>
    <label><input type="radio" name="passRole" value="collaboration"> ${esc(t("Collaboration — another adult: shared threads"))}</label>
    <small class="empty">${esc(t("Harnesses allowed"))}</small>${names.map((h) => `<label><input type="checkbox" name="passH" value="${esc(h)}"${h === "openfox" ? " checked" : ""}> ${esc(harnessName(h))}</label>`).join("")}
    <label for="passUntil">${esc(t("Until (optional)"))}</label><input id="passUntil" type="date">`;
  sheet(t("Core pass"), [[t("Create the pass"), async () => {
    const name = $("passName").value.trim(), role = (document.querySelector("input[name=passRole]:checked") || {}).value, harnesses = [...document.querySelectorAll("input[name=passH]:checked")].map((i) => i.value);
    if (!name) return toast(t("Name the person")); if (!harnesses.length) return toast(t("Allow at least one harness"));
    const until = $("passUntil").value ? new Date($("passUntil").value + "T23:59:59").getTime() / 1000 : 0;
    const r = await api("/api/pass", { name, role, harnesses, until }); if (!r.token) return toast(t("The pass was refused"));
    const link = `${location.origin}${location.pathname}#pass=${r.token}`, out = document.createElement("div"); out.className = "invite";
    out.innerHTML = `<small class="empty">${esc(t(role === "supervision" ? "Give {name} this pass — in the Inari app or a browser, it opens their threads on this core; they will see that you follow them:" : "Give {name} this pass — in the Inari app or a browser, it opens their threads on this core:", { name }))}</small><code></code><small class="empty">${esc(t("Or the link:"))}</small><code></code>`;
    const [c1, c2] = out.querySelectorAll("code"); c1.textContent = r.token; c2.textContent = link;
    sheet(t("Pass for {name}", { name }), [[t("Copy the link"), () => navigator.clipboard && navigator.clipboard.writeText(link).then(() => toast(t("Copied")))]], t("Shown once: keep it until it is given."), out);
  }]], "", box); }
function passList() { sheet(t("Passes"), (snap.passes || []).map((p) => [t(p.until ? "Revoke {name} ({role}, until {date})" : "Revoke {name} ({role})", { name: p.name, role: t(p.role), date: p.until ? new Date(p.until * 1000).toLocaleDateString(LANG) : "" }), () => api("/api/pass/revoke", { id: p.id }).then(() => toast(t("{name}'s pass is revoked", { name: p.name })))]), t("Revoking ends the access at once; the threads stay on this core.")); }
$("addCore").onclick = addCore; // outside the list the snapshots redraw: a click on it is never lost
function attachCore() { const hub = snap.nodes.find((n) => (n.capabilities || {}).enroll); if (!hub) return toast(t("The hub takes no invitations: set enroll_port in its configuration"));
  const box = document.createElement("div"); box.className = "invite"; box.innerHTML = `<label for="coreName">${esc(t("Name of the new core"))}</label><input id="coreName" placeholder="vps-1" maxlength="32" autocomplete="off"><div id="inviteOut"></div>`;
  sheet(t("Attach a core"), [[t("Create the invitation"), async () => {
    const name = $("coreName").value.trim().toLowerCase(); if (!/^[a-z0-9-]{1,32}$/.test(name)) return toast(t("Letters, digits and dashes only"));
    const r = await api("/api/invite", { node_id: name }); if (!r.code) return toast(t("The invitation was refused"));
    const cmd = `inari-core-rs join ${hub.capabilities.enroll} ${r.code} --service`, out = document.createElement("div"); out.className = "invite";
    out.innerHTML = `<b>${esc(t("On the new machine"))}</b><small class="empty">${esc(t("Copy the inari-core-rs binary there, then run (valid {min} minutes, once):", { min: Math.round(r.expires_in / 60) }))}</small><code></code>`; out.querySelector("code").textContent = cmd;
    sheet(t("Invitation for {name}", { name }), [[t("Copy the command"), () => navigator.clipboard && navigator.clipboard.writeText(cmd).then(() => toast(t("Copied")))]], t("It joins over your private network and appears here as {name}.", { name }), out);
  }]], "", box); }
function openMove(agent, from) { sheet(t("Move {name} to…", { name: (snap.agents[agent] || {}).title || agent }), snap.nodes.filter((n) => n.node_id !== from && n.role === "heavy").map((n) => [n.node_id, () => teleport(agent, from, n.node_id)])); }
async function teleport(agent, source, target) { const r = await api("/api/teleport", { agent, source, target }); if (r.request) { toast(t("Moving {name} to {target}…", { name: agent, target })); log(t("Moving {name}: {source} → {target}", { name: agent, source, target })); } else toast(t("Request refused")); }
/* ---------- gestures: complex things anchored in simple moves ----------
   Drag a thread onto a core: it moves there. Onto another thread: they merge. Onto the dock that appears:
   clone it to another model, share it with a person, put it to sleep. Onto the mascot: summarise or sleep.
   On a phone: swipe a thread left to move it, right for its artifacts; long-press a message to start again
   from it; drop a file on a core to begin a thread with it. */
function startDrag(e, el, id, from) { if (e.button > 0) return;
  drag = { id, from, x: e.clientX, y: e.clientY, active: false, el, ghost: null, hold: null, touch: e.pointerType === "touch", guest: !!(snap.ui && snap.ui.pass), held: false };
  // touch: a short press lifts the thread (then it is dragged like with a mouse); mouse: the press is the drag
  drag.hold = setTimeout(() => { if (!drag || drag.active) return; drag.held = true; if (drag.touch && !drag.guest) { lift(); if (navigator.vibrate) navigator.vibrate(12); } }, drag.touch ? 380 : 650);
  window.onpointermove = onMove; window.onpointerup = onUp; window.onpointercancel = cancelDrag; }
function lift() { drag.active = true; { const r = drag.el.getBoundingClientRect(); drag.origin = [r.left + r.width / 2, r.top + r.height / 2]; } drag.ghost = drag.el.cloneNode(true); drag.ghost.classList.add("ghost", "glass"); drag.ghost.style.left = drag.x + "px"; drag.ghost.style.top = drag.y + "px"; document.body.append(drag.ghost);
  drag.el.style.opacity = ".35"; document.querySelectorAll(".core").forEach((n) => n.classList.add(n.dataset.node === drag.from ? "invalid" : "valid")); placeFox(); showDock(drag.id); }
function cancelDrag() { window.onpointermove = window.onpointerup = window.onpointercancel = null; if (!drag) return; clearTimeout(drag.hold);
  if (drag.ghost) drag.ghost.remove(); drag.el.style.opacity = ""; $("dropdock").hidden = true; $("dragline").setAttribute("hidden", ""); hot(null); document.querySelectorAll(".core").forEach((n) => n.classList.remove("valid", "invalid")); drag = null; placeFox(); }
// while a lifted thread is carried, the finger must not scroll the page
document.addEventListener("touchmove", (e) => { if (drag && drag.active) e.preventDefault(); }, { passive: false });
// a long press on a thread is ours: not the system's text selection or context menu on top of it
document.addEventListener("contextmenu", (e) => { if (e.target.closest && e.target.closest(".row")) e.preventDefault(); });
function onMove(e) { if (!drag) return; const far = Math.hypot(e.clientX - drag.x, e.clientY - drag.y) > 8;
  if (!drag.active) { if (!far) return; if (drag.touch || drag.guest) { cancelDrag(); return; } clearTimeout(drag.hold); lift(); } // a finger moving before the lift is a scroll
  drag.lx = e.clientX; drag.ly = e.clientY; drag.ghost.style.left = e.clientX + "px"; drag.ghost.style.top = e.clientY + "px"; const t = dropTarget(e.clientX, e.clientY, drag); hot(t); drawLine(e.clientX, e.clientY, t); }
// after a long press or a carry, the browser's own tap at the finger's spot must not touch what just opened
let swallowUntil = 0; addEventListener("click", (e) => { if (Date.now() < swallowUntil) { e.stopPropagation(); e.preventDefault(); } }, true);
function onUp(e) { window.onpointermove = window.onpointerup = window.onpointercancel = null; if (!drag) return; clearTimeout(drag.hold); const d = drag; drag = null; setTimeout(placeFox); /* home again, once the drop below is read */
  if (d.held || d.active) swallowUntil = Date.now() + 500;
  // where the finger last was: a touch's end does not always carry a position
  const ux = d.lx ?? e.clientX, uy = d.ly ?? e.clientY, moved = Math.hypot(ux - d.x, uy - d.y) > 8;
  if (d.ghost) d.ghost.remove(); d.el.style.opacity = ""; hot(null); $("dragline").setAttribute("hidden", "");
  if (!d.active || (d.held && !moved)) { $("dropdock").hidden = true; document.querySelectorAll(".core").forEach((n) => n.classList.remove("valid", "invalid")); if (d.held && !d.guest) actionSheet(d.id); else select(d.id); if (snap) renderCores(Object.entries(snap.agents)); return; }
  const t = dropTarget(ux, uy, d); $("dropdock").hidden = true; document.querySelectorAll(".core").forEach((n) => n.classList.remove("valid", "invalid"));
  if (snap) renderCores(Object.entries(snap.agents)); if (!t) return;
  if (t.kind === "core") { if (t.node !== d.from) teleport(d.id, d.from, t.node); }
  else if (t.kind === "thread") mergeInto(t.id, d.id);
  else if (t.kind === "clone") cloneTo(d.id, t.harness);
  else if (t.kind === "share") shareWith(d.id, t.pass);
  else if (t.kind === "sleep") sleepThread(d.id, true);
  else if (t.kind === "fox") foxMenu(d.id); }
/* A long press without moving: everything else one can do with the thread (moving it is a drag onto a core). */
/* Rename in place: the title becomes editable; Enter keeps it, Escape leaves it. */
function renameThread(id) { select(id).then(() => { const t = $("ptitle"); t.contentEditable = "true"; t.focus(); const r = document.createRange(); r.selectNodeContents(t); getSelection().removeAllRanges(); getSelection().addRange(r);
  const done = (ok) => { t.onkeydown = t.onblur = null; t.contentEditable = "false"; const v = t.textContent.trim(); if (ok && v) api("/api/rename", { agent: id, title: v }); else renderTop(); };
  t.onkeydown = (e) => { if (e.key === "Enter") { e.preventDefault(); done(true); } else if (e.key === "Escape") done(false); }; t.onblur = () => done(true); }); }
$("ptitle").ondblclick = () => { if (selected) renameThread(selected); };
/* Stop replaces Send while the thread works and the box is empty. */
function drawStop() { const a = selected && snap && snap.agents[selected], busy = !!a && ["listening", "loading", "inference"].includes(a.phase), empty = !$("msg").value.trim() && !attachments.length; $("stopBtn").hidden = !(busy && empty); $("sendBtn").hidden = busy && empty; }
$("stopBtn").onclick = () => { if (selected) { api("/api/cancel", { agent: selected }); toast(t("Stopping…")); } };
$("msg").addEventListener("input", drawStop);
function actionSheet(id) { const a = snap.agents[id]; if (!a) return; const node = snap.nodes.find((n) => n.node_id === a.node) || {};
  const opts = [[t("Open it, with its artifacts"), () => select(id).then(() => { store.set("nobench", false); document.body.classList.add("bench-open"); layout(); })]];
  const has = (n, h) => !!(((n.capabilities || {}).harnesses || {})[h]);
  for (const h of [...new Set(snap.nodes.flatMap((n) => Object.keys((n.capabilities || {}).harnesses || {})))].filter((h) => h !== a.harness)) {
    const where = has(node, h) ? "" : (snap.nodes.find((n) => has(n, h)) || {}).node_id; opts.push([t(where ? "Clone to {harness} (on {node})" : "Clone to {harness}", { harness: harnessName(h), node: where }), () => cloneTo(id, h)]); }
  for (const p of snap.passes || []) opts.push(a.pass === p.id ? [t("Stop sharing with {name}", { name: p.name }), () => shareWith(id, "")] : [t("Share with {name}", { name: p.name }), () => shareWith(id, p.id)]);
  const others = Object.entries(snap.agents).filter(([o, x]) => o !== id && !x.archived);
  if (others.length) opts.push([t("Merge into another thread…"), () => sheet(t("Merge {name} into…", { name: a.title || id }), others.map(([o, x]) => [x.title || o, () => mergeInto(o, id)]))]);
  opts.push([t("Rename…"), () => renameThread(id)]);
  opts.push([a.pinned ? t("Unpin") : t("Pin to the top"), () => api("/api/pin", { agent: id, pinned: !a.pinned })]);
  opts.push([t("Summarise it"), async () => { await api("/api/message", { agent: id, text: t("Summarise this thread so far in a few short lines.") }); select(id); }]);
  opts.push([t("Start a copy from here (fork)"), () => { selected = id; forkAt = null; forkNow(); }]);
  opts.push([t("Put it to sleep"), () => sleepThread(id, true)]);
  if (!(snap.ui && snap.ui.pass)) opts.push([t("Delete…"), () => sheet(t("Delete “{name}”?", { name: a.title || id }), [[t("Delete it"), async () => { await api("/api/delete", { agent: id }); if (selected === id) { selected = null; render(); } toast(t("Deleted — kept in the core's trash")); }]], t("It leaves Inari. The core keeps it in its trash folder, so it can still be restored by hand."))]);
  sheet(a.title || id, opts, t("To move it, drag it onto another core, or use its arrow.")); }
function dropTarget(x, y, d) { for (const el of document.elementsFromPoint(x, y)) { if (el === d.ghost || (d.ghost && d.ghost.contains(el))) continue;
    if (el.id === "fox" || el.closest("#fox")) return { kind: "fox", el: $("fox") };
    const dz = el.closest && el.closest(".dz"); if (dz) return { kind: dz.dataset.kind, harness: dz.dataset.harness, pass: dz.dataset.pass, el: dz };
    const r = el.closest && el.closest(".row[data-agent]"); if (r && r.dataset.agent !== d.id) return { kind: "thread", id: r.dataset.agent, el: r };
    const c = el.closest && el.closest(".core"); if (c) return { kind: "core", node: c.dataset.node, el: c }; } return null; }
function hot(t) { document.querySelectorAll(".hot").forEach((n) => n.classList.remove("hot")); if (t && t.kind !== "core") t.el.classList.add("hot"); }
/* the dock and the line are not targets themselves (the line ignores the pointer; the dock's gaps do not count) */
function showDock(id) { const a = snap.agents[id], dock = $("dropdock"), here = snap.nodes.find((n) => n.node_id === a.node) || {};
  const has = (n, h) => !!(((n.capabilities || {}).harnesses || {})[h]), all = [...new Set(snap.nodes.flatMap((n) => Object.keys((n.capabilities || {}).harnesses || {})))].filter((h) => h !== a.harness);
  dock.innerHTML = all.map((h) => { const where = has(here, h) ? "" : (snap.nodes.find((n) => has(n, h)) || {}).node_id; return `<span class="dz" data-kind="clone" data-harness="${esc(h)}">${esc(t("Clone to {harness}", { harness: harnessName(h) }))}${where ? ` <small>${esc(t("on {node}", { node: where }))}</small>` : ""}</span>`; }).join("")
    + (snap.passes || []).map((p) => a.pass === p.id ? `<span class="dz" data-kind="share" data-pass="">${esc(t("Stop sharing with {name}", { name: p.name }))}</span>` : `<span class="dz" data-kind="share" data-pass="${esc(p.id)}">${esc(t("Share with {name}", { name: p.name }))}</span>`).join("")
    + `<span class="dz" data-kind="sleep">${esc(t("Sleep"))}</span><small>${esc(t("or drop it on a core, a thread, the fox"))}</small>`;
  dock.querySelectorAll(".dz[data-kind=clone]").forEach((z) => z.prepend(badge((HARNESS[z.dataset.harness] || [z.dataset.harness])[0], 18)));
  // beside the thread: to its right on a wide screen, just above or below it on a phone
  const r = drag.el.getBoundingClientRect(), narrow = innerWidth <= 760; dock.classList.toggle("row-strip", narrow); dock.hidden = false; const w = dock.offsetWidth, hgt = dock.offsetHeight;
  // never over a place the thread could be dropped on: another core, the fox
  const blocks = [...document.querySelectorAll(".core")].filter((c) => c.dataset.node !== a.node).map((c) => c.getBoundingClientRect()).concat([$("fox").getBoundingClientRect()]);
  const clear = (x, y) => y >= 56 && y + hgt <= innerHeight - 4 && blocks.every((b) => x + w < b.left || x > b.right || y + hgt < b.top || y > b.bottom);
  const x = narrow ? (innerWidth - w) / 2 : Math.min(r.right + 18, innerWidth - w - 8);
  const ys = narrow ? [r.top - hgt - 12, r.bottom + 12, 60, innerHeight - hgt - 12] : [Math.max(60, Math.min(r.top - 8, innerHeight - hgt - 8)), 60, innerHeight - hgt - 12];
  const y = ys.find((v) => clear(x, v)) ?? ys[ys.length - 1];
  Object.assign(dock.style, { left: x + "px", top: y + "px" }); }
/* The dotted line from the thread to where it would land: the finger, or the target it is on. */
function drawLine(x, y, t) { const svg = $("dragline"); if (!drag) return; svg.removeAttribute("hidden"); const o = drag.origin;
  let ex = x, ey = y; if (t && t.el) { const b = t.el.getBoundingClientRect(); ex = b.left + b.width / 2; ey = b.top + b.height / 2; }
  const mx = (o[0] + ex) / 2, my = Math.min(o[1], ey) - 40; svg.querySelector("path").setAttribute("d", `M${o[0]},${o[1]} Q${mx},${my} ${ex},${ey}`);
  const [dot, ring] = svg.querySelectorAll("circle"); dot.setAttribute("cx", ex); dot.setAttribute("cy", ey); ring.setAttribute("cx", ex); ring.setAttribute("cy", ey); ring.style.display = t ? "" : "none"; }
const until = (test, ms) => new Promise((ok) => { const end = Date.now() + ms; const tick = () => (test() ? ok(true) : Date.now() > end ? ok(false) : setTimeout(tick, 300)); tick(); });
function mergeInto(target, source) { const tg = snap.agents[target], s = snap.agents[source]; if (!tg || !s) return;
  sheet(t("Merge “{source}” into “{target}”?", { source: s.title || source, target: tg.title || target }), [[t("Merge"), async () => {
    if (s.node !== tg.node) { toast(t("Bringing {name} to {node} first…", { name: source, node: tg.node })); await teleport(source, s.node, tg.node); if (!(await until(() => snap.agents[source] && snap.agents[source].node === tg.node, 90000))) return toast(t("The move did not finish")); }
    await api("/api/merge", { target, source }); toast(t("Merged: the journals are one")); select(target); }]], t("The second thread's events join the first one's journal; both threads stay.")); }
async function cloneTo(id, harness) { const r = await api("/api/clone", { agent: id, harness, label: harnessName(harness) }); if (!r.thread_id) return toast(t("The clone was refused"));
  toast(t("Cloning to {harness}…", { harness: harnessName(harness) })); if (await until(() => snap.agents[r.thread_id], 20000)) select(r.thread_id); }
async function shareWith(id, pass) { await api("/api/share", { agent: id, pass }); const p = (snap.passes || []).find((x) => x.id === pass); toast(p ? t("Shared with {name}", { name: p.name }) : t("No longer shared")); }
async function sleepThread(id, on) { await api("/api/archive", { agent: id, archived: on }); toast(on ? t("Asleep — find it under Sleeping") : t("Awake again")); if (on && selected === id) { selected = null; render(); } }
function foxMenu(id) { const a = snap.agents[id]; sheet(a.title || id, [[t("Summarise it"), async () => { await api("/api/message", { agent: id, text: t("Summarise this thread so far in a few short lines.") }); select(id); }], [t("Put it to sleep"), () => sleepThread(id, true)]]); }
$("sleepers").onclick = () => sheet(t("Sleeping threads"), Object.entries(snap.agents).filter(([, a]) => a.archived).map(([id, a]) => [t("Wake {name}", { name: a.title || id }), () => sleepThread(id, false)]));
/* A file dropped on a core starts a thread there, the file attached, ready for what to do with it. */
$("cores").addEventListener("dragover", (e) => { if (!e.dataTransfer || ![...e.dataTransfer.types].includes("Files")) return; e.preventDefault(); const c = e.target.closest(".core"); document.querySelectorAll(".filehot").forEach((n) => n !== c && n.classList.remove("filehot")); if (c) c.classList.add("filehot"); });
$("cores").addEventListener("dragleave", (e) => { if (!$("cores").contains(e.relatedTarget)) document.querySelectorAll(".filehot").forEach((n) => n.classList.remove("filehot")); });
$("cores").addEventListener("drop", async (e) => { const c = e.target.closest(".core"); document.querySelectorAll(".filehot").forEach((n) => n.classList.remove("filehot")); if (!c || !e.dataTransfer || !e.dataTransfer.files.length) return; e.preventDefault();
  const files = [...e.dataTransfer.files].filter((f) => f.size <= 25e6), name = (files[0].name.replace(/\.[^.]+$/, "").replace(/[^\w-]+/g, "-").slice(0, 28) || "file") + "-" + (Date.now() % 1000);
  const made = await api("/api/agent", { thread_id: name, node: c.dataset.node, title: files[0].name }), id = made.thread_id || name;
  if (!(await until(() => snap.agents[id], 15000))) return toast(t("The thread did not start")); await select(id);
  for (const f of files) attachments.push({ name: f.name, file: f }); drawComposer(); $("msg").focus(); toast(t("Say what to do with it")); });
/* A long press on a message: start again from there (a fork up to it). */
$("msgs").addEventListener("pointerdown", (e) => { const m = e.target.closest(".msg[data-event]"); if (!m || (snap.ui && snap.ui.pass)) return;
  const done = () => { clearTimeout(hold); removeEventListener("pointerup", done); removeEventListener("pointermove", far); }, far = (ev) => { if (Math.hypot(ev.clientX - e.clientX, ev.clientY - e.clientY) > 10) done(); };
  const hold = setTimeout(() => { done(); forkAt = m.dataset.event; sheet(t("Start again from here?"), [[t("Fork from this message"), forkNow]], t("A new thread with the journal up to this message; this one stays as it is.")); }, 600);
  addEventListener("pointerup", done); addEventListener("pointermove", far); });
addEventListener("resize", () => snap && drawTethers());
/* Keyboard: Escape closes what is open, / searches, ? lists the shortcuts (outside text fields). */
document.addEventListener("keydown", (e) => {
  if (e.key === "Escape") { if ($("sheet").classList.contains("open")) { $("sheet").classList.remove("open"); e.preventDefault(); } if ($("pop").classList.contains("open")) $("pop").classList.remove("open"); $("hits").style.display = "none"; return; }
  const typing = e.target.closest && e.target.closest("input, textarea, select, [contenteditable=true]"); if (typing || e.ctrlKey || e.metaKey || e.altKey || $("app").hidden) return;
  if (e.key === "/") { e.preventDefault(); $("search").classList.add("on"); $("search").focus(); }
  else if (e.key === "?") { e.preventDefault(); const list = document.createElement("dl"); list.className = "keys";
    for (const [k, what] of [["Enter", "send the message (in the composer); open the focused thread"], ["Shift + Enter", "new line in the message"], ["T", "move the focused thread to another core"], ["/", "search every thread"], ["← →", "switch the workbench tabs"], ["Esc", "close a menu or a sheet"], ["?", "this list"]]) list.insertAdjacentHTML("beforeend", `<dt><kbd>${esc(t(k))}</kbd></dt><dd>${esc(t(what))}</dd>`);
    sheet(t("Keyboard shortcuts"), [], "", list); }
});
translatePage();
connect();
