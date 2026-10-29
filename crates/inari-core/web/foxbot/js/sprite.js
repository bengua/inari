/* SwarmFox sprite mascot v3 — real-time 2D sprite engine for the Android WebView.
 * Preserves the Goupil contract: window.fox.setPet/Health/Activity/Offline/Mood/Stale/ReducedMotion/Event/say/pause/resume.
 * Transparent background, no panels, crisp transitions.
 */

const CANVAS_SIZE = 512;
const BG_TRANSPARENT = true;
const SPRITE_SIZE = 256;
const DRAW_SIZE = 280;

const ACTION_CONFIG = {
  respire:     { pingpong: true,  fps: 12, frames: 49, loop: true,  speed: 1.15 },
  cligne:      { pingpong: false, fps: 12, frames: 49, loop: false, speed: 1.30 },
  rit:         { pingpong: true,  fps: 14, frames: 49, loop: true,  speed: 1.30 },
  dort:        { pingpong: true,  fps: 8,  frames: 49, loop: true,  speed: 1.00 },
  tourne:      { pingpong: false, fps: 14, frames: 49, loop: true,  speed: 1.70 },
  joue:        { pingpong: true,  fps: 14, frames: 49, loop: true,  speed: 1.50 },
  salue:       { pingpong: true,  fps: 12, frames: 49, loop: true,  speed: 1.40 },
  court:       { pingpong: false, fps: 16, frames: 49, loop: true,  speed: 1.60, flip: true },
  malade:      { pingpong: true,  fps: 12, frames: 49, loop: true,  speed: 1.00 },
  tape:        { pingpong: true,  fps: 12, frames: 49, loop: true,  speed: 1.00 },
  tape_v2:     { pingpong: true,  fps: 14, frames: 30, loop: true,  speed: 1.00 },
  ballepart:   { pingpong: false, fps: 16, frames: 81, loop: false, speed: 1.60, flip: true },
  balleretour: { pingpong: false, fps: 16, frames: 81, loop: false, speed: 1.60 },
  chatouille:  { pingpong: true,  fps: 16, frames: 81, loop: true,  speed: 1.30 },
  chassequeue: { pingpong: false, fps: 16, frames: 81, loop: true,  speed: 1.70 },
  yoyo:        { pingpong: true,  fps: 14, frames: 81, loop: true,  speed: 1.30 },
  machouille:  { pingpong: true,  fps: 12, frames: 49, loop: true,  speed: 1.20 },
  toilette:    { pingpong: true,  fps: 12, frames: 81, loop: true,  speed: 1.20, chain: ['baille', 'dort'] },
  baille:      { pingpong: true,  fps: 10, frames: 49, loop: false, speed: 1.00, chain: ['dort'] },
  renifle:     { pingpong: true,  fps: 14, frames: 49, loop: true,  speed: 1.30 },
  oreilles:    { pingpong: false, fps: 16, frames: 33, loop: false, speed: 1.00 },
  parle:       { pingpong: true,  fps: 16, frames: 49, loop: true,  speed: 1.20 },
  amour: { pingpong: false, fps: 16, frames: 33, loop: false, speed: 1.00 },
  apparait: { pingpong: false, fps: 14, frames: 49, loop: false, speed: 1.20 },
  arrive: { pingpong: false, fps: 14, frames: 49, loop: false, speed: 1.30, flip: true },
  assis: { pingpong: true,  fps: 12, frames: 49, loop: true,  speed: 1.20 },
  attente_impatiente: { pingpong: true,  fps: 12, frames: 49, loop: true,  speed: 1.20 },
  attrape_friandise: { pingpong: false, fps: 14, frames: 49, loop: false, speed: 1.50, flip: true },
  cache_cache: { pingpong: true,  fps: 12, frames: 49, loop: true,  speed: 1.20 },
  colere: { pingpong: true,  fps: 12, frames: 49, loop: true,  speed: 1.20 },
  concentration: { pingpong: true,  fps: 12, frames: 49, loop: true,  speed: 1.20 },
  lie_down_on_command: { pingpong: false, fps: 14, frames: 49, loop: false, speed: 1.20 },
  danse: { pingpong: true,  fps: 12, frames: 49, loop: true,  speed: 1.20 },
  disparait: { pingpong: false, fps: 14, frames: 49, loop: false, speed: 1.20 },
  donne_patte: { pingpong: false, fps: 14, frames: 49, loop: false, speed: 1.30 },
  echec: { pingpong: false, fps: 14, frames: 49, loop: false, speed: 1.20 },
  entre_droite: { pingpong: false, fps: 14, frames: 49, loop: false, speed: 1.60, flip: true },
  etire: { pingpong: true,  fps: 12, frames: 49, loop: true,  speed: 1.20 },
  fierte: { pingpong: true,  fps: 12, frames: 49, loop: true,  speed: 1.20 },
  griffe: { pingpong: false, fps: 14, frames: 49, loop: false, speed: 1.40 },
  high_five: { pingpong: false, fps: 14, frames: 49, loop: false, speed: 1.50 },
  honte: { pingpong: true,  fps: 12, frames: 49, loop: true,  speed: 1.20 },
  malicieux: { pingpong: true,  fps: 12, frames: 49, loop: true,  speed: 1.20 },
  part: { pingpong: false, fps: 14, frames: 49, loop: false, speed: 1.30 },
  peur: { pingpong: true,  fps: 12, frames: 49, loop: true,  speed: 1.20 },
  rapporte_balle: { pingpong: false, fps: 14, frames: 49, loop: false, speed: 1.40 },
  reconfort: { pingpong: true,  fps: 12, frames: 49, loop: true,  speed: 1.20 },
  renifle_sol: { pingpong: true,  fps: 12, frames: 49, loop: true,  speed: 1.20 },
  roule: { pingpong: true,  fps: 12, frames: 49, loop: true,  speed: 1.20 },
  s_assoit: { pingpong: true,  fps: 12, frames: 49, loop: true,  speed: 1.20 },
  se_cache: { pingpong: true,  fps: 12, frames: 49, loop: true,  speed: 1.20 },
  se_couche: { pingpong: false, fps: 14, frames: 49, loop: false, speed: 1.10 },
  se_leve: { pingpong: false, fps: 14, frames: 49, loop: false, speed: 1.20 },
  se_reveille: { pingpong: false, fps: 14, frames: 49, loop: false, speed: 1.10 },
  se_secoue: { pingpong: true,  fps: 12, frames: 49, loop: true,  speed: 1.20 },
  sort_gauche: { pingpong: false, fps: 14, frames: 49, loop: false, speed: 1.60 },
  succes: { pingpong: false, fps: 14, frames: 49, loop: false, speed: 1.40 },
  surcharge: { pingpong: true,  fps: 12, frames: 49, loop: true,  speed: 1.20 },
  surprise: { pingpong: false, fps: 16, frames: 33, loop: false, speed: 1.00 },
  chassequeue_v2: { pingpong: false, fps: 16, frames: 81, loop: true,  speed: 1.40 },
  leche_babines: { pingpong: false, fps: 16, frames: 33, loop: false, speed: 1.00 },
  gratte_oreille: { pingpong: false, fps: 16, frames: 33, loop: false, speed: 1.00 },
  ebouriffe: { pingpong: false, fps: 16, frames: 33, loop: false, speed: 1.00 },
  renifle_air: { pingpong: false, fps: 16, frames: 33, loop: false, speed: 1.00 },
  patte_levee: { pingpong: false, fps: 16, frames: 33, loop: false, speed: 1.00 },
  sourire: { pingpong: false, fps: 16, frames: 33, loop: false, speed: 1.00 },
  cligne_double: { pingpong: false, fps: 16, frames: 33, loop: false, speed: 1.00 },
  grimace_langue: { pingpong: false, fps: 16, frames: 33, loop: false, speed: 1.00 },
  grimace_sourcil: { pingpong: false, fps: 16, frames: 33, loop: false, speed: 1.00 },
  grimace_sourire_force: { pingpong: false, fps: 16, frames: 33, loop: false, speed: 1.00 },
  grimace_gonfle: { pingpong: false, fps: 16, frames: 33, loop: false, speed: 1.00 },
  grimace_plisse: { pingpong: false, fps: 16, frames: 33, loop: false, speed: 1.00 },
  grimace_nez: { pingpong: false, fps: 16, frames: 33, loop: false, speed: 1.00 },
  grimace_cote: { pingpong: false, fps: 16, frames: 33, loop: false, speed: 1.00 },
  grimace_moue: { pingpong: false, fps: 16, frames: 33, loop: false, speed: 1.00 },
  timide: { pingpong: true,  fps: 12, frames: 49, loop: true,  speed: 1.20 },
  chassequeue_v3: { pingpong: false, fps: 16, frames: 81, loop: true,  speed: 1.40 },
  // Signatures personas (sprites_persona/<p>/) — inertes sans persona actif.
  fume_clope:  { pingpong: false, fps: 14, frames: 49, loop: false, speed: 1.00 },
  tire_langue: { pingpong: false, fps: 16, frames: 33, loop: false, speed: 1.00 },
  nonchalance: { pingpong: true,  fps: 12, frames: 33, loop: true,  speed: 1.00 },
  burger:      { pingpong: true,  fps: 14, frames: 49, loop: true,  speed: 1.20 },
  aviateur:    { pingpong: false, fps: 14, frames: 33, loop: false, speed: 1.00 },
  flingues:    { pingpong: false, fps: 16, frames: 49, loop: false, speed: 1.10 },
  soda:        { pingpong: true,  fps: 12, frames: 33, loop: true,  speed: 1.10 },
  salut_zen:   { pingpong: false, fps: 12, frames: 33, loop: false, speed: 1.00 },
  kata:        { pingpong: false, fps: 14, frames: 81, loop: true,  speed: 1.10 },
  yoga:        { pingpong: true,  fps: 10, frames: 49, loop: true,  speed: 0.90 },
  fruit:       { pingpong: false, fps: 14, frames: 49, loop: false, speed: 1.00 },
  // Artiste (ComfyUI) et militaire (rooms sentinelle).
  peint:          { pingpong: true,  fps: 14, frames: 49, loop: true,  speed: 1.20 },
  palette:        { pingpong: false, fps: 12, frames: 33, loop: false, speed: 1.00 },
  chef_oeuvre:    { pingpong: false, fps: 14, frames: 49, loop: false, speed: 1.00 },
  toile_ratee:    { pingpong: false, fps: 14, frames: 33, loop: false, speed: 1.10 },
  jumelles:       { pingpong: true,  fps: 12, frames: 49, loop: true,  speed: 1.00 },
  radio:          { pingpong: false, fps: 14, frames: 33, loop: false, speed: 1.00 },
  salut_militaire:{ pingpong: false, fps: 14, frames: 33, loop: false, speed: 1.00 },
  patrouille:     { pingpong: false, fps: 14, frames: 49, loop: true,  speed: 1.20 },
};

const ZONES = {
  head:  { x: 0.35, y: 0.18, w: 0.30, h: 0.28 },
  ears:  { x: 0.25, y: 0.02, w: 0.50, h: 0.22 },
  belly: { x: 0.30, y: 0.45, w: 0.40, h: 0.38 },
  tail:  { x: 0.68, y: 0.42, w: 0.28, h: 0.38 },
  nose:  { x: 0.43, y: 0.28, w: 0.14, h: 0.10 }
};

const canvas = document.getElementById('fox');
const ctx = canvas.getContext('2d', { alpha: true });
ctx.imageSmoothingEnabled = true;
ctx.imageSmoothingQuality = 'high';

let images = {};
let availableActions = new Set();
let state = 'idle';
let health = 100;
let reducedMotion = false;
let lastTime = performance.now();
let time = 0;

let current = 'respire';
let frameIndex = 0;
let frameDir = 1;
let pendingAction = null;
let pendingReason = null;
let interactionTimer = null;
let transitionFast = false;
// Cross-fade between clips (generations do not always return
// exactly to frame zero — the fade hides the pose jump).
let prevFrame = null;
let fadeT = 1;
const FADE_SEC_BASE = 0.14;
const FADE_SEC_LONG = 0.26;
let chainQueue = [];
// Transition rapide : rampe progressive au lieu d'un facteur constant brutal.
let transitionRamp = 0;
const TRANSITION_RAMP_SPEED = 5.0;
const TRANSITION_PEAK_MULT = 3.2;
// Global tempo lift (user: kill the slow-motion feel) — skipped under reduced motion.
const GLOBAL_SPEED = 1.15;

let proc = {
  squash: 1, stretch: 1, rot: 0,
  dx: 0, dy: 0,
  shake: 0, dizzy: 0,
  wiggle: 0, wigglePhase: 0,
  look: 0,
  anticipBounce: 0
};

let particles = [];
let hearts = [];
let sparks = [];
let zzzs = [];

let ball = null;
let ballState = 'none';
let ballReturnDelay = 0;
let ballIdleT = 0;
let ballReturnTarget = { x: CANVAS_SIZE / 2 + 80, y: CANVAS_SIZE / 2 + 140 };

var lastInteract = 0; // seconds on the loop's `time` clock, not performance.now()
let boredomTimer = null;
let nextBoredomAt = 0;
let petting = false;
let petTime = 0;
let tailHits = 0;

let rafId = null;
let paused = false;
let harness = null; // 'local', 'reflexion', 'claude', 'kimi', 'auto', null

let speakAudioCtx = null;
let speakAnalyser = null;
let speakSource = null;
let speakDataArray = null;
let speakActive = false;
let speakAmplitude = 0;
let speakMouthClip = null;
let speakMouthFrame = 0;
let speakReturnAction = 'respire';
let speakFallbackSpeed = 1;

function clamp(v, a, b) { return Math.max(a, Math.min(b, v)); }
function lerp(a, b, t) { return a + (b - a) * t; }
function easeInOutQuad(t) { return t < 0.5 ? 2 * t * t : 1 - Math.pow(-2 * t + 2, 2) / 2; }
function easeOutQuad(t) { return 1 - (1 - t) * (1 - t); }
function easeInOutCubic(t) { return t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2; }
function smoothstep(t) { return t * t * (3 - 2 * t); }
function smoothReturn(current, target, dt, speed) {
  const diff = Math.abs(current - target);
  const adaptiveSpeed = speed * (0.4 + diff * 0.6);
  return lerp(current, target, Math.min(1, dt * adaptiveSpeed));
}

// Deterministic seeded RNG for reproducible procedural variety.
function mulberry32(seed) {
  return function() {
    let t = seed += 0x6D2B79F5;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const PROC_SEED = (() => {
  const p = new URLSearchParams(location.search).get('fox_seed');
  return p ? parseInt(p, 10) : Math.floor(Math.random() * 0xFFFFFFFF);
})();
const rng = mulberry32(PROC_SEED);
const instanceSpeed = 0.8 + rng() * 0.6;

const SPRITE_BASE_URL = (() => {
  const p = new URLSearchParams(location.search).get('fox_sprites');
  if (p) return p.endsWith('/') ? p : p + '/';
  return 'https://cachyos-x8664.tailb0ff14.ts.net:8998/fox-sprites/';
})();
const CORE_ACTIONS = ['respire', 'cligne', 'rit', 'dort', 'tourne', 'joue', 'salue', 'court'];

// Base des sprites persona : locale dans la webapp (/foxbot/sprites_persona/,
// symlink), distante pour l'APK (tailnet :8998, fox-sprites-persona).
const PERSONA_BASE_URL = (() => {
  if (SPRITE_BASE_URL.includes('fox-sprites/'))
    return SPRITE_BASE_URL.replace(/fox-sprites\/$/, 'fox-sprites-persona/');
  if (SPRITE_BASE_URL.endsWith('/sprites/'))
    return SPRITE_BASE_URL.replace(/sprites\/$/, 'sprites_persona/');
  return SPRITE_BASE_URL + 'sprites_persona/';
})();

// Active persona: null = original fox (main mascot). Personas
// (rebel/merica/zen/artist/military) live in sprites_persona/<p>/ and
// only apply to rooms — never the home screen.
let currentPersona = null;
const KNOWN_PERSONAS = ['rebel', 'merica', 'zen', 'artiste', 'militaire',
                        'grok', 'google', 'openai', 'meta'];
// The person chose their avatar (the host said so with setPersona): from then on the harness no
// longer picks one. Any persona of sprites_persona/ may be chosen; the host checks the name.
let personaChosen = false;
const PERSONA_NAME = /^[a-z0-9_-]{1,40}$/;
// The poses the fox cannot do without (loadBaseActions).
const BASE_ACTIONS = ['respire', 'cligne', 'rit', 'dort', 'tourne', 'joue', 'salue', 'court', 'malade', 'tape'];

function actionPath(name, i, remote = false, persona = null) {
  if (persona) {
    const pBase = remote ? PERSONA_BASE_URL : 'sprites_persona/';
    return `${pBase}${persona}/${name}/f_${String(i).padStart(2, '0')}.webp`;
  }
  const base = remote ? SPRITE_BASE_URL : 'sprites/';
  return `${base}${name}/f_${String(i).padStart(2, '0')}.webp`;
}

function loadOneImage(src) {
  return new Promise((resolve) => {
    const img = new Image();
    img.crossOrigin = 'anonymous';
    img.onload = () => resolve(img);
    img.onerror = () => resolve(null);
    img.src = src;
  });
}

async function loadAction(name) {
  if (images[name]) return true;
  const cfg = ACTION_CONFIG[name];
  if (!cfg) return false;

  const tryLoad = async (remote, persona) => {
    const promises = [];
    for (let i = 0; i < cfg.frames; i++) {
      promises.push(loadOneImage(actionPath(name, i, remote, persona)));
    }
    const loaded = await Promise.all(promises);
    // Persona variants do not always have the same number of frames as
    // the global clip: accept the longest complete prefix (>= 8 frames).
    let prefix = 0;
    while (prefix < loaded.length && loaded[prefix]) prefix++;
    return prefix >= 8 ? loaded.slice(0, prefix) : null;
  };

  // An active persona takes priority: its poses replace the original fox's.
  // Local d'abord (webapp / bundle), distant ensuite (APK via tailnet).
  if (currentPersona) {
    for (const remote of [false, true]) {
      const persoLoaded = await tryLoad(remote, currentPersona);
      if (persoLoaded) {
        images[name] = persoLoaded;
        availableActions.add(name);
        return true;
      }
    }
    // Never the original fox's frames in a persona: the look would change mid-animation. A pose the
    // fox cannot do without falls back to the persona's own breathing; anything else is not played
    // until the persona has it (docs/personas-sequences.md lists what is missing).
    if (BASE_ACTIONS.includes(name) && name !== 'respire' && images['respire']) {
      images[name] = images['respire'];
      availableActions.add(name);
      return true;
    }
    return false;
  }
  // Core actions must come from the local APK bundle.
  const sources = CORE_ACTIONS.includes(name) ? [false] : [false, true];
  for (const remote of sources) {
    const loaded = await tryLoad(remote);
    if (loaded) {
      images[name] = loaded;
      availableActions.add(name);
      return true;
    }
  }
  return false;
}

async function loadBaseActions() {
  for (const name of BASE_ACTIONS) {
    const ok = await loadAction(name);
    if (!ok) throw new Error(`Missing required action: ${name}`);
  }
}

async function loadOptionalActions() {
  const [hasPart, hasReturn, hasTickle, hasChase] = await Promise.all([
    loadAction('ballepart'),
    loadAction('balleretour'),
    loadAction('chatouille'),
    loadAction('chassequeue'),
    loadAction('assis'),
    loadAction('oreilles'),
    loadAction('surprise'),
    loadAction('amour'),
    loadAction('tape_v2')
  ]);
  if (hasPart && hasReturn) console.log('Ball v2 clips available');
  if (hasTickle) console.log('Tickle clip available');
  if (hasChase) console.log('Tail-chase clip available');
  // Ambient nano-shorts: preloaded in the background so the boredom
  // pool can draw from them (they start/end on the neutral pose).
  const nanos = ['leche_babines', 'gratte_oreille', 'ebouriffe', 'renifle_air',
                 'patte_levee', 'sourire', 'cligne_double', 'malicieux', 'etire', 'toilette',
                 'grimace_langue', 'grimace_sourcil', 'grimace_sourire_force', 'grimace_gonfle',
                 'grimace_plisse', 'grimace_nez', 'grimace_cote', 'grimace_moue'];
  for (const n of nanos) loadAction(n);
}

function stateAction(s) {
  if (s === 'sleeping') return 'dort';
  // Health follows the HUD severity bands (ok > 55, warn > 18, crit ≤ 18):
  // a few penalty points (a silent backup, a sleepy node) must not put the
  // fox on a drip — warn reads as tired, only crit is the full sick act.
  if (health <= 18) return 'malade';
  if (health <= 55 && ACTION_CONFIG['assis'] && images['assis']) return 'assis';
  if (health <= 55) return 'respire';
  const map = {
    idle: 'respire', queued: 'joue',
    working: images['tape_v2'] ? 'tape_v2' : 'tape',
    waiting: 'salue', failed: 'malade'
  };
  return map[s] || 'respire';
}

const loadingActions = {};
let pendingAfterLoad = null;

function queueAction(name, reason) {
  if (!ACTION_CONFIG[name]) return false;
  if (images[name]) {
    pendingAction = name;
    pendingReason = reason;
    transitionFast = true;
    return true;
  }
  // Lazy-load the action; if it succeeds and no newer request arrived,
  // queue it as the pending action.
  if (!loadingActions[name]) {
    loadingActions[name] = loadAction(name).then(ok => {
      delete loadingActions[name];
      if (ok && pendingAfterLoad === name) {
        pendingAction = name;
        pendingReason = 'lazy-load';
        transitionFast = true;
        pendingAfterLoad = null;
      }
      return ok;
    });
  }
  pendingAfterLoad = name;
  return false;
}

let fadeDuration = FADE_SEC_BASE;

function playNow(name) {
  if (!ACTION_CONFIG[name] || !images[name]) return;
  const cfg = ACTION_CONFIG[name];
  // Capture the displayed pose for the fade to the new clip.
  if (images[current] && current !== name) {
    prevFrame = { name: current, idx: frameIndex };
    fadeT = 0;
    // Adaptive duration: different clips or distant frame -> longer fade.
    const sameFamily = current.startsWith(name.substring(0, 4)) || name.startsWith(current.substring(0, 4));
    const frameGap = Math.abs(frameIndex - (cfg.pingpong && frameDir < 0 ? cfg.frames - 1 : 0));
    fadeDuration = sameFamily && frameGap < 12 ? FADE_SEC_BASE : FADE_SEC_LONG;
  }
  current = name;
  frameIndex = 0;
  frameDir = 1;
  pendingAction = null;
  transitionFast = false;
  transitionRamp = 0;
  // Pre-compute deterministic chain for this instance.
  chainQueue = [];
  if (cfg.chain && Array.isArray(cfg.chain)) {
    for (const next of cfg.chain) {
      if (ACTION_CONFIG[next] && images[next]) chainQueue.push(next);
    }
  }
}

function setFoxState(next) {
  // Heartbeat with no real change (same state, same clip): touch NOTHING.
  // Before, every health poll cancelled running interactions (clearTimeout,
  // ball/petting killed) and reset frameIndex to 0 — stutter + chopped peek-a-boos.
  const target = stateAction(next);
  if (next === state && target === current) return;
  state = next;
  if (ballState !== 'none') { ballState = 'none'; ball = null; }
  petting = false; petTime = 0;
  proc.wiggle = 0; proc.anticipBounce = 0;
  if (interactionTimer) { clearTimeout(interactionTimer); interactionTimer = null; }

  if (state === 'failed') { proc.squash = 0.86; proc.stretch = 1; }
  const start = next === 'sleeping' ? 35 : 0;
  if (target === current) return;
  const cfg = ACTION_CONFIG[current];
  if (frameIndex === 0 && cfg.pingpong) { playNow(target); frameIndex = start; }
  else { transitionFast = true; queueAction(target, 'state'); }
}

function setHealthInput(pct) {
  health = clamp(Number(pct), 0, 100);
  reconcileState();
}

function setActivityInput(activity) {
  if (activity === 'working') setFoxState('working');
  else if (activity === 'idle') setFoxState('idle');
  else if (activity === 'waiting') setFoxState('waiting');
  else if (activity === null) setFoxState('idle');
}

function setOfflineInput(reason) {
  if (reason) setFoxState('sleeping');
  else reconcileState();
}

// Switch persona (rebel/merica/zen/artist/military) or go back to the fox
// original (null). The swap is atomic: the old sprite stays displayed
// while the new one loads — never an empty screen.
async function setPersonaInput(persona) {
  const p = (typeof persona === 'string' && PERSONA_NAME.test(persona)) ? persona : null;
  if (p === currentPersona) return;
  const prevImages = images, prevAvail = availableActions, prevPersona = currentPersona;
  currentPersona = p;
  images = {};
  availableActions = new Set();
  try {
    await loadBaseActions();
    await loadOptionalActions();
  } catch (e) {
    images = {}; // a persona without its breathing: handled just below
  }
  if (!images['respire']) {
    // Chargement impossible : on restaure l'ancien persona sans coupure.
    images = prevImages;
    availableActions = prevAvail;
    currentPersona = prevPersona;
    return;
  }
  // Re-sync the current clip into the new persona (fade via playNow).
  if (images[current]) { const keep = current; playNow(keep); }
  else playNow(stateAction(state));
}

function setPetInput(raw) {
  try {
    const o = JSON.parse(raw || '{}');
    health = clamp(Number(o.health ?? health), 0, 100);
    if (o.harness) harness = normalizeHarness(o.harness);
    if (o.offline) setFoxState('sleeping');
    else if (health < 30) setFoxState('failed');
    else if ((o.load ?? 0) > 50) setFoxState('working');
    else setFoxState('idle');
    // health-driven malade posture is handled inside stateAction().
  } catch (e) { setFoxState('idle'); }
}

function normalizeHarness(type) {
  if (!type) return null;
  const t = String(type).toLowerCase();
  if (t.includes('claude')) return 'claude';
  if (t.includes('kimi')) return 'kimi';
  if (t.includes('comfy') || t.includes('artiste') || t.includes('studio')) return 'comfyui';
  if (t.includes('sentin') || t.includes('secur') || t.includes('militaire') || t.includes('watch')) return 'sentinelle';
  if (t.includes('local') || t.includes('reflexion')) return 'local';
  if (t === 'auto') return 'auto';
  return null;
}

function setHarnessInput(type) {
  harness = normalizeHarness(type);
  // Le persona suit le harnais : claude→merica, kimi→zen, local→rebel,
  // comfyui→artiste, sentinelle→militaire.
  // null/auto → original fox (main mascot, home screen).
  const PERSONA_BY_HARNESS = { claude: 'merica', kimi: 'zen', local: 'rebel',
                               comfyui: 'artiste', sentinelle: 'militaire',
                               grok: 'grok', openai: 'openai',
                               google: 'google', meta: 'meta' };
  if (!personaChosen) setPersonaInput(PERSONA_BY_HARNESS[harness] || null);
}

// The host's choice of avatar (the person's, kept by the core): null is the original fox.
function choosePersona(persona) {
  personaChosen = true;
  return setPersonaInput(persona);
}

function reconcileState() {
  if (state === 'sleeping') return;
  const target = stateAction(state);
  if (target !== current) setFoxState(state);
}

function bumpInteraction() {
  lastInteract = time;
}

function queueInteraction(name, durationSec, thenAction = null) {
  if (ballState !== 'none') { ballState = 'none'; ball = null; }
  if (interactionTimer) { clearTimeout(interactionTimer); interactionTimer = null; }
  queueAction(name, 'interaction');
  if (thenAction) {
    setTimeout(() => { if (current === name) thenAction(); }, durationSec * 1000);
  }
  interactionTimer = setTimeout(() => {
    if (current === name && ballState === 'none') queueAction(stateAction(state), 'state');
    interactionTimer = null;
  }, durationSec * 1000);
}

function spawnBall() {
  bumpInteraction();
  if (ballState !== 'none') return;
  // Ball starts at hand height in front of the fox, ready to be dragged and thrown.
  ball = {
    x: CANVAS_SIZE / 2 + 70,
    y: CANVAS_SIZE / 2 + 140,
    r: 16,
    vx: 0, vy: 0,
    held: false,
    startX: 0, startY: 0,
    alpha: 1
  };
  ballState = 'spawn';
  ballIdleT = 0;
  if (interactionTimer) { clearTimeout(interactionTimer); interactionTimer = null; }
}

function triggerPet() {
  bumpInteraction();
  if (!petting) startPet();
}

function triggerTail() {
  bumpInteraction();
  tailInteraction();
}

function triggerHug() {
  bumpInteraction();
  hugInteraction();
}

// Tail pool: the "puppy" variants (v2) join the rotation as soon
// as their sprites exist; the 360 slide stays one alternative among others.
const TAIL_CLIPS = ['chassequeue_v3', 'chassequeue_v2', 'chassequeue', 'tourne'];

function tailInteraction() {
  tailHits += 1;
  const pool = TAIL_CLIPS.filter((c) => availableActions.has(c));
  const action = pool.length ? pool[Math.floor(Math.random() * pool.length)] : 'tourne';
  queueInteraction(action, 3, () => {
    if (tailHits >= 2) { proc.dizzy = 1; tailHits = 0; }
  });
}

function hugInteraction() {
  proc.squash = 1.25; proc.stretch = 0.8;
  burstHearts(16);
  setTimeout(() => { proc.squash = 1; proc.stretch = 1; }, 250);
}

/** Touching the ears: he lays them back and wiggles them. */
function earsInteraction() {
  const action = availableActions.has('oreilles') ? 'oreilles' : 'cligne';
  queueInteraction(action, 2);
}

/** Boop sur le nez : surprise + petit squash. */
function noseInteraction() {
  proc.squash = 0.92; proc.stretch = 1.06;
  setTimeout(() => { proc.squash = 1; proc.stretch = 1; }, 150);
  const action = availableActions.has('surprise') ? 'surprise' : 'cligne';
  queueInteraction(action, 1.6);
}

/** Petting the head: he melts (hearts if the love clip is there). */
function headInteraction() {
  const action = availableActions.has('amour') ? 'amour' : 'salue';
  queueInteraction(action, 2.5);
  if (availableActions.has('amour')) burstHearts(6);
}

function startPet() {
  if (petting) return;
  petting = true; petTime = 0;
  const action = availableActions.has('chatouille') ? 'chatouille' : 'rit';
  queueInteraction(action, 999);
}

function stopPet() {
  petting = false; petTime = 0;
  proc.shake = 0; proc.wiggle = 0;
  if (interactionTimer) { clearTimeout(interactionTimer); interactionTimer = null; }
  queueAction(stateAction(state), 'state');
}

function pointInZone(x, y, zone) {
  const left = (CANVAS_SIZE - DRAW_SIZE) / 2 + zone.x * DRAW_SIZE;
  const top = (CANVAS_SIZE - DRAW_SIZE) / 2 + zone.y * DRAW_SIZE;
  return x >= left && x <= left + zone.w * DRAW_SIZE && y >= top && y <= top + zone.h * DRAW_SIZE;
}

function zoneAt(x, y) {
  for (const [name, z] of Object.entries(ZONES)) {
    if (pointInZone(x, y, z)) return name;
  }
  return null;
}

function getPointerPos(e) {
  const rect = canvas.getBoundingClientRect();
  const scaleX = CANVAS_SIZE / rect.width;
  const scaleY = CANVAS_SIZE / rect.height;
  return { x: (e.clientX - rect.left) * scaleX, y: (e.clientY - rect.top) * scaleY };
}

function onPointerDown(e) {
  const { x, y } = getPointerPos(e);
  if (ball && (ballState === 'spawn' || ballState === 'drag' || ballState === 'idle_ball' || ballState === 'nudge')) {
    const dx = x - ball.x, dy = y - ball.y;
    if (dx * dx + dy * dy <= (ball.r + 24) ** 2) {
      ball.held = true; ball.startX = ball.x; ball.startY = ball.y;
      ballState = 'drag'; proc.anticipBounce = 1;
      return;
    }
  }
  bumpInteraction();
  const z = zoneAt(x, y);
  if (z === 'tail') tailInteraction();
  else if (z === 'belly') startPet();
  else if (z === 'ears') earsInteraction();
  else if (z === 'nose') noseInteraction();
  else if (z === 'head') headInteraction();
  else {
    proc.squash = 0.9; proc.stretch = 1.05;
    setTimeout(() => { proc.squash = 1; proc.stretch = 1; }, 120);
  }
}

function onPointerMove(e) {
  const { x, y } = getPointerPos(e);
  if (ball && ball.held) { ball.x = x; ball.y = clamp(y, CANVAS_SIZE / 2 + 60, CANVAS_SIZE - 60); }
}

function onPointerUp(e) {
  if (ball && ball.held) {
    ball.held = false; proc.anticipBounce = 0;
    const { x, y } = getPointerPos(e);
    const dx = ball.startX - x;
    const dy = ball.startY - y;
    const power = Math.min(1, Math.hypot(dx, dy) / 160);
    ball.vx = Math.max(80, dx * 4);
    ball.vy = -Math.abs(dy * 4) - 180;
    if (Math.abs(ball.vx) < 120) ball.vx = 140;
    if (ball.vy > -220) ball.vy = -340;
    ball.vx = clamp(ball.vx, -700, 700);
    ball.vy = clamp(ball.vy, -520, -180);
    ballState = 'fly';
    return;
  }
  if (petting) stopPet();
}

function onDoubleClick(e) {
  const { x, y } = getPointerPos(e);
  bumpInteraction();
  if (zoneAt(x, y)) hugInteraction();
}

canvas.addEventListener('pointerdown', onPointerDown);
canvas.addEventListener('pointermove', onPointerMove);
window.addEventListener('pointerup', onPointerUp);
canvas.addEventListener('dblclick', onDoubleClick);

function spawnParticle(type, x, y) {
  particles.push({ type, x, y,
    vx: (Math.random() - 0.5) * 70,
    vy: -50 - Math.random() * 70,
    life: 1 + Math.random() * 0.5,
    size: 5 + Math.random() * 7 });
}

function burstHearts(count) {
  const cx = CANVAS_SIZE / 2;
  const cy = CANVAS_SIZE / 2 - 60;
  for (let i = 0; i < count; i++) {
    hearts.push({
      x: cx + (Math.random() - 0.5) * 100,
      y: cy + (Math.random() - 0.5) * 60,
      vx: (Math.random() - 0.5) * 100,
      vy: -70 - Math.random() * 80,
      life: 0,
      maxLife: 1.5 + Math.random(),
      size: 7 + Math.random() * 10
    });
  }
}

function addSpark() {
  const cx = CANVAS_SIZE / 2 + (Math.random() - 0.5) * 60;
  const cy = CANVAS_SIZE / 2 - 90 + (Math.random() - 0.5) * 20;
  sparks.push({ x: cx, y: cy,
    vx: (Math.random() - 0.5) * 25,
    vy: -25 - Math.random() * 30,
    life: 0, maxLife: 0.8 + Math.random() * 0.6,
    size: 2 + Math.random() * 3 });
}

function addZzz() {
  const headX = CANVAS_SIZE / 2 + 30;
  const headY = CANVAS_SIZE / 2 - 90;
  zzzs.push({ x: headX, y: headY,
    vx: 10 + Math.random() * 10,
    vy: -14 - Math.random() * 10,
    life: 0, maxLife: 2.5 + Math.random(),
    size: 14 + Math.random() * 7 });
}

function actionSpeed(name) {
  const cfg = ACTION_CONFIG[name];
  let s = cfg.speed * instanceSpeed;
  if (reducedMotion && s > 1) s = 1;
  if (petting && (name === 'chatouille' || name === 'rit')) {
    const esc = clamp(petTime / 4, 0, 1);
    s = lerp(s, 1.8, esc);
  }
  if (speakActive && speakMouthClip === 'joue' && name === 'joue') {
    s *= 0.7 + 0.9 * speakAmplitude;
  }
  if (health < 30) s *= 0.45 + rng() * 0.45;
  else if (health < 100) s *= 0.70 + rng() * 0.30;
  return s;
}

// Per-action flip decisions are deterministic for this instance.
const FLIP_DECISIONS = {};
function shouldFlip(name) {
  if (!ACTION_CONFIG[name]?.flip) return false;
  if (!(name in FLIP_DECISIONS)) FLIP_DECISIONS[name] = rng() < 0.5;
  return FLIP_DECISIONS[name];
}

function updateSpeak(dt) {
  if (!speakActive) {
    speakAmplitude = smoothReturn(speakAmplitude, 0, dt, 12);
    return;
  }
  if (speakAnalyser && speakDataArray) {
    speakAnalyser.getByteTimeDomainData(speakDataArray);
    let sum = 0;
    for (let i = 0; i < speakDataArray.length; i++) {
      const v = (speakDataArray[i] - 128) / 128;
      sum += v * v;
    }
    const rms = Math.sqrt(sum / speakDataArray.length);
    speakAmplitude = smoothReturn(speakAmplitude, Math.min(1, rms * 3.5), dt, 22);
  } else {
    speakAmplitude = smoothReturn(speakAmplitude, 0, dt, 12);
  }
  if (speakMouthClip && images[speakMouthClip]) {
    const len = images[speakMouthClip].length;
    const targetFrame = Math.floor(clamp(0.15 + 0.85 * speakAmplitude, 0, 1) * (len - 1));
    speakMouthFrame = lerp(speakMouthFrame, targetFrame, Math.min(1, dt * 18));
  }
}

function update(dt) {
  time += dt;
  updateSpeak(dt);

  const cfg = ACTION_CONFIG[current];
  // Persona clips often have fewer frames than the global config:
  // loop over the REAL loaded length, otherwise we trample frames
  // that do not exist (frozen image = looping stutter).
  const n = (images[current] && images[current].length) || cfg.frames;
  let speed = actionSpeed(current);
  if (!reducedMotion) speed *= GLOBAL_SPEED;

  // Fast transition: progressive ramp (accelerates then decelerates).
  if (transitionFast) {
    transitionRamp = Math.min(1, transitionRamp + dt * TRANSITION_RAMP_SPEED);
    const rampCurve = Math.sin(transitionRamp * Math.PI);
    speed *= 1 + rampCurve * (TRANSITION_PEAK_MULT - 1);
  } else if (transitionRamp > 0) {
    transitionRamp = Math.max(0, transitionRamp - dt * TRANSITION_RAMP_SPEED * 0.5);
  }

  frameIndex += cfg.fps * speed * dt * frameDir;

  // Fade with ease-in-out-cubic easing for a perceptually smooth transition.
  if (fadeT < 1) fadeT = Math.min(1, fadeT + dt / fadeDuration);

  if (cfg.pingpong) {
    if (frameIndex >= n - 1) { frameIndex = n - 1; frameDir = -1; }
    if (frameIndex <= 0) {
      frameIndex = 0; frameDir = 1;
      if (chainQueue.length > 0 && !pendingAction) {
        transitionFast = false;
        playNow(chainQueue.shift());
      } else if (pendingAction) {
        transitionFast = false;
        playNow(pendingAction);
        if (pendingReason === 'state' && state === 'sleeping') frameIndex = 35;
      }
    }
  } else {
    if (frameIndex >= n - 1) {
      // Fade at the loop seam: hides the jump when the generation
      // does not return exactly to frame zero.
      if (cfg.loop && !pendingAction && chainQueue.length === 0) {
        prevFrame = { name: current, idx: n - 1 };
        fadeT = 0;
      }
      frameIndex = 0;
      if (chainQueue.length > 0 && !pendingAction) {
        transitionFast = false;
        playNow(chainQueue.shift());
      } else if (pendingAction) { transitionFast = false; playNow(pendingAction); }
      else if (!cfg.loop) queueAction(stateAction(state), 'state');
    }
  }

  proc.dizzy = smoothReturn(proc.dizzy, 0, dt, 2.0);
  proc.dx = Math.sin(time * 2) * 2;
  proc.dy = Math.cos(time * 3) * 1.5;

  let targetLook = 0;
  if (ball && (ballState === 'drag' || ballState === 'fly' || ballState === 'spawn' || ballState === 'idle_ball' || ballState === 'nudge')) {
    targetLook = clamp((ball.x - CANVAS_SIZE / 2) / 180, -0.18, 0.18);
  }
  proc.look = smoothReturn(proc.look, targetLook, dt, 8.0);

  if (ballState === 'drag') {
    const b = 1 + Math.sin(time * 18) * 0.06 * proc.anticipBounce;
    proc.squash = smoothReturn(proc.squash, b, dt, 14.0);
    proc.stretch = smoothReturn(proc.stretch, 1 / b, dt, 14.0);
  } else if (current === 'respire' || current === 'joue') {
    const breath = 1 + Math.sin(time * (state === 'working' ? 1.5 : 3)) * 0.015;
    proc.squash = smoothReturn(proc.squash, breath, dt, 5.0);
    proc.stretch = smoothReturn(proc.stretch, 1 / breath, dt, 5.0);
  }
  if (state === 'failed') {
    proc.squash = smoothReturn(proc.squash, 0.86, dt, 4.0);
    proc.stretch = smoothReturn(proc.stretch, 1, dt, 4.0);
  }

  if (petting) {
    petTime += dt;
    const intensity = smoothstep(Math.min(petTime / 4, 1));
    proc.shake = 0.5 + intensity * 1.5;
    if (petTime > 4) {
      proc.wiggle = smoothReturn(proc.wiggle, 1, dt, 5.0);
      proc.wigglePhase += dt * 8;
    }
  } else {
    proc.shake = smoothReturn(proc.shake, 0, dt, 10.0);
    proc.wiggle = smoothReturn(proc.wiggle, 0, dt, 6.0);
  }

  if (state === 'working' && rng() < 0.08) addSpark();
  if (state === 'sleeping' && rng() < 0.03) addZzz();

  if (time - lastInteract > 30 && time >= nextBoredomAt && ballState === 'none' && !petting) {
    if (!boredomTimer) {
      boredomTimer = setTimeout(() => {
        // The fox breathes, static; clips are only occasional
        // reflexes. After each draw, a long random silence.
        const NANO_CLIPS = ['leche_babines', 'gratte_oreille', 'ebouriffe', 'renifle_air',
                            'patte_levee', 'sourire', 'cligne_double', 'oreilles',
                            'malicieux', 'etire', 'toilette'];
        // Grimaces: separate bank, rare draw for surprise.
        const GRIMACE_CLIPS = ['grimace_langue', 'grimace_sourcil', 'grimace_sourire_force',
                               'grimace_gonfle', 'grimace_plisse', 'grimace_nez',
                               'grimace_cote', 'grimace_moue'];
        // Occupations longues (clips en boucle, ~6 s) : encore plus rares.
        const LONG_CLIPS = ['machouille', 'yoyo'];
        // Signatures of the active persona: they color the boredom depending on the
        // model in charge (rebel/merica/zen).
        const PERSONA_NANO = {
          rebel:  ['tire_langue', 'nonchalance'],
          merica: ['aviateur', 'soda'],
          zen:    ['salut_zen', 'fruit'],
          artiste:   ['palette', 'toilette'],
          militaire: ['salut_militaire', 'radio']
        };
        const PERSONA_LONG = {
          rebel:  ['fume_clope', 'machouille'],
          merica: ['burger', 'flingues'],
          zen:    ['kata', 'yoga'],
          artiste:   ['peint', 'chef_oeuvre'],
          militaire: ['jumelles', 'patrouille']
        };
        const nanos = NANO_CLIPS.concat((currentPersona && PERSONA_NANO[currentPersona]) || [])
                                .filter((c) => availableActions.has(c));
        const grimaces = GRIMACE_CLIPS.filter((c) => availableActions.has(c));
        const longs = LONG_CLIPS.concat((currentPersona && PERSONA_LONG[currentPersona]) || [])
                                .filter((c) => availableActions.has(c));
        const roll = rng();
        if (roll < 0.45) {
          // Nothing: he keeps breathing. Next reflex in 10-25 s.
          nextBoredomAt = time + 10 + rng() * 15;
        } else {
          let opts = null;
          let dur = 2.5;
          if (longs.length && roll < 0.52) { opts = longs; dur = 6; }
          else if (grimaces.length && roll < 0.62) opts = grimaces;
          else if (nanos.length) opts = nanos;
          if (opts) queueInteraction(opts[Math.floor(rng() * opts.length)], dur);
          // Cooldown: 18-45 s of calm before the next reflex.
          nextBoredomAt = time + 18 + rng() * 27;
        }
        boredomTimer = null;
      }, 100);
    }
  }

  for (const p of particles) { p.x += p.vx * dt; p.y += p.vy * dt; p.vy += 60 * dt; p.life -= dt; }
  particles = particles.filter(p => p.life > 0);

  for (const h of hearts) {
    h.x += h.vx * dt; h.y += h.vy * dt;
    h.vx *= 0.98; h.vy *= 0.98; h.vy += 30 * dt;
    h.life += dt;
  }
  hearts = hearts.filter(h => h.life < h.maxLife);

  for (const s of sparks) { s.x += s.vx * dt; s.y += s.vy * dt; s.life += dt; }
  sparks = sparks.filter(s => s.life < s.maxLife);

  for (const z of zzzs) { z.x += z.vx * dt; z.y += z.vy * dt; z.life += dt; }
  zzzs = zzzs.filter(z => z.life < z.maxLife);

  updateBall(dt);
}

function updateBall(dt) {
  if (ballState === 'none') return;

  if (ballState === 'spawn' || ballState === 'idle_ball') {
    ballIdleT += dt;
    if (ballState === 'spawn' && ballIdleT < 0.3) {
      const t = easeOutQuad(ballIdleT / 0.3);
      ball.alpha = t;
      const scale = 0.6 + t * 0.4;
      ball.r = 16 * scale;
    } else {
      ball.alpha = 1;
      ball.r = 16;
    }
    if (ballState === 'idle_ball' && ballIdleT > 15) { ballState = 'nudge'; ballIdleT = 0; }
    return;
  }

  if (ballState === 'nudge') {
    ballIdleT += dt;
    const phase = ballIdleT / 1.2;
    if (phase < 0.33) {
      const t = easeInOutCubic(phase / 0.33);
      proc.squash = lerp(1, 0.85, t);
      proc.stretch = lerp(1, 1.12, t);
    } else if (phase < 0.67) {
      const t = easeInOutCubic((phase - 0.33) / 0.34);
      ball.x = lerp(ball.x, CANVAS_SIZE / 2 - 50, t * 0.15);
      proc.squash = lerp(0.85, 1.05, t);
      proc.stretch = lerp(1.12, 0.95, t);
    } else {
      const t = easeOutQuad((phase - 0.67) / 0.33);
      proc.squash = lerp(1.05, 1, t);
      proc.stretch = lerp(0.95, 1, t);
      if (ballIdleT > 1.2) { ballState = 'idle_ball'; ballIdleT = 0; }
    }
    return;
  }

  if (ballState === 'drag') return;

  if (ballState === 'fly') {
    ball.x += ball.vx * dt;
    ball.y += ball.vy * dt;
    ball.vy += 520 * dt;
    ball.vx *= 0.995;

    if (ball.y > CANVAS_SIZE - 55) {
      ball.y = CANVAS_SIZE - 55;
      ball.vy *= -0.5;
      ball.vx *= 0.7;
    }
    if (ball.x > CANVAS_SIZE - 25) { ball.x = CANVAS_SIZE - 25; ball.vx *= -0.55; }
    if (ball.x < 25) { ball.x = 25; ball.vx *= -0.55; }

    const outOfFrame = ball.x > CANVAS_SIZE + 20 || ball.x < -20;
    if (outOfFrame || (Math.abs(ball.vy) < 60 && ball.y > CANVAS_SIZE - 100)) {
      ballState = 'watch';
      ball.watchT = 0;
      ballReturnTarget = { x: clamp(ball.x, 50, CANVAS_SIZE - 50), y: CANVAS_SIZE / 2 + 140 };
    }
    return;
  }

  if (ballState === 'watch') {
    ball.watchT += dt;
    const rawT = clamp(ball.watchT / 0.25, 0, 1);
    const t = easeInOutCubic(rawT);
    proc.squash = lerp(1, 0.78, t);
    proc.stretch = lerp(1, 1.18, t);
    if (ball.watchT >= 0.25) {
      proc.squash = 1; proc.stretch = 1;
      ballState = 'depart';
      playNow(availableActions.has('ballepart') ? 'ballepart' : 'court');
    }
    return;
  }

  if (ballState === 'depart') {
    if ((availableActions.has('ballepart') && frameIndex >= ACTION_CONFIG.ballepart.frames - 1) ||
        (!availableActions.has('ballepart') && frameIndex >= ACTION_CONFIG.court.frames - 1)) {
      ballState = 'gone';
      ball.goneT = 0;
    }
    return;
  }

  if (ballState === 'gone') {
    ball.goneT += dt;
    ball.x += ball.vx * dt;
    ball.vx *= 0.92;
    if (ball.y < CANVAS_SIZE - 55) {
      ball.y += ball.vy * dt;
      ball.vy += 520 * dt;
      if (ball.y > CANVAS_SIZE - 55) { ball.y = CANVAS_SIZE - 55; ball.vy *= -0.4; }
    }
    if (ball.x > CANVAS_SIZE - 35) { ball.x = CANVAS_SIZE - 35; ball.vx *= -0.5; }
    if (ball.x < 35) { ball.x = 35; ball.vx *= -0.5; }

    ballReturnDelay = clamp(0.5 + Math.abs(ballReturnTarget.x - CANVAS_SIZE / 2) / 300, 0.5, 2.0);
    if (ball.goneT >= ballReturnDelay) {
      ballState = 'return';
      playNow(availableActions.has('balleretour') ? 'balleretour' : 'court');
    }
    return;
  }

  if (ballState === 'return') {
    if (availableActions.has('balleretour')) {
      const rawT = clamp((frameIndex - 10) / 60, 0, 1);
      const t = easeInOutCubic(rawT);
      const mouthX = CANVAS_SIZE + 80 - t * (CANVAS_SIZE + 80 - ballReturnTarget.x + 20);
      const mouthY = CANVAS_SIZE / 2 + 20 + Math.sin(frameIndex * 0.12) * 10;
      ball.x = mouthX + 20;
      ball.y = mouthY + 14;
      if (frameIndex >= ACTION_CONFIG.balleretour.frames - 1) {
        ballState = 'drop';
        ball.dropT = 0;
        ball.vx = -30;
        ball.vy = 0;
      }
    } else {
      const returnSpeed = easeOutQuad(clamp(ballIdleT / 0.8, 0, 1)) * 6 + 2;
      ball.x = lerp(ball.x, ballReturnTarget.x, dt * returnSpeed);
      ball.y = lerp(ball.y, ballReturnTarget.y - 30, dt * returnSpeed);
      ballIdleT += dt;
      if (Math.abs(ball.x - ballReturnTarget.x) < 15) { ballState = 'drop'; ball.dropT = 0; ballIdleT = 0; }
    }
    return;
  }

  if (ballState === 'drop') {
    ball.dropT += dt;
    const decayT = easeOutQuad(clamp(ball.dropT / 0.6, 0, 1));
    ball.vx = lerp(-30, 0, decayT);
    ball.x += ball.vx * dt;
    if (ball.dropT > 0.6) {
      ballState = 'idle_ball';
      ballIdleT = 0;
      ball.vx = 0;
      queueAction(stateAction(state), 'state');
    }
    return;
  }
}

function drawSprite(name, idx, alpha = 1, extraFilter = '') {
  const imgs = images[name];
  if (!imgs) return;
  const img = imgs[Math.floor(clamp(idx, 0, imgs.length - 1))];
  ctx.save();
  ctx.globalAlpha = alpha;
  const cx = CANVAS_SIZE / 2 + proc.dx + Math.sin(proc.wigglePhase) * proc.wiggle * 35;
  const cy = CANVAS_SIZE / 2 + proc.dy;
  const filter = extraFilter;
  if (filter) ctx.filter = filter.trim();
  ctx.translate(cx, cy);
  ctx.rotate(proc.look + proc.rot + Math.sin(time * 1.5) * proc.dizzy * 0.15);
  ctx.scale(proc.squash, proc.stretch);
  if (shouldFlip(name)) ctx.scale(-1, 1);
  if (proc.shake > 0) ctx.rotate((rng() - 0.5) * proc.shake * 0.08);
  ctx.drawImage(img, -DRAW_SIZE / 2, -DRAW_SIZE / 2, DRAW_SIZE, DRAW_SIZE);
  ctx.restore();
  drawHarnessPastille(cx, cy);
}

function drawBall() {
  if (!ball) return;
  ctx.save();
  ctx.globalAlpha = ball.alpha || 1;
  const shadowScale = 1 - clamp((ball.y - (CANVAS_SIZE / 2 + 120)) / 240, 0, 0.5);
  ctx.fillStyle = 'rgba(0,0,0,0.35)';
  ctx.beginPath();
  ctx.ellipse(ball.x, CANVAS_SIZE - 45, ball.r * 0.9 * shadowScale, ball.r * 0.25 * shadowScale, 0, 0, Math.PI * 2);
  ctx.fill();

  const grad = ctx.createRadialGradient(ball.x - 4, ball.y - 5, 2, ball.x, ball.y, ball.r);
  grad.addColorStop(0, '#FFD8A8');
  grad.addColorStop(0.4, '#FF8A47');
  grad.addColorStop(1, '#E85D04');
  ctx.fillStyle = grad;
  ctx.strokeStyle = '#FFFFFF';
  ctx.lineWidth = 2;
  ctx.beginPath();
  ctx.arc(ball.x, ball.y, ball.r, 0, Math.PI * 2);
  ctx.fill();
  ctx.stroke();

  ctx.fillStyle = 'rgba(255,255,255,0.35)';
  ctx.beginPath();
  ctx.arc(ball.x - 4, ball.y - 4, ball.r * 0.3, 0, Math.PI * 2);
  ctx.fill();
  ctx.restore();
}

function drawAimGuide() {
  if (!ball || !ball.held) return;
  const dx = ball.startX - ball.x;
  const dy = ball.startY - ball.y;
  const steps = 14;
  ctx.save();
  for (let i = 1; i <= steps; i++) {
    const t = i / steps;
    const px = ball.x + dx * t * 2.4;
    const py = ball.y + dy * t * 2.4 - 150 * t * t;
    const a = (1 - t * 0.6) * 0.85;
    ctx.globalAlpha = a;
    ctx.fillStyle = i % 2 === 0 ? '#FFFFFF' : '#FF8A47';
    const r = 3 + (1 - t) * 3;
    ctx.beginPath();
    ctx.arc(px, py, r, 0, Math.PI * 2);
    ctx.fill();
  }
  ctx.restore();
}

function drawParticles() {
  for (const p of particles) {
    ctx.save();
    ctx.globalAlpha = clamp(p.life, 0, 1);
    ctx.fillStyle = '#FF6B9D';
    ctx.beginPath();
    ctx.arc(p.x, p.y, p.size, 0, Math.PI * 2);
    ctx.fill();
    ctx.restore();
  }
  for (const h of hearts) {
    const a = 1 - h.life / h.maxLife;
    ctx.save();
    ctx.globalAlpha = clamp(a, 0, 1);
    ctx.fillStyle = '#FF4D8A';
    const s = h.size;
    ctx.translate(h.x, h.y);
    ctx.beginPath();
    ctx.moveTo(0, -s * 0.3);
    ctx.bezierCurveTo(-s, -s, -s, s * 0.3, 0, s);
    ctx.bezierCurveTo(s, s * 0.3, s, -s, 0, -s * 0.3);
    ctx.fill();
    ctx.restore();
  }
  for (const s of sparks) {
    const a = 1 - s.life / s.maxLife;
    ctx.save();
    ctx.globalAlpha = clamp(a, 0, 1);
    ctx.fillStyle = '#FFD166';
    ctx.shadowBlur = 8;
    ctx.shadowColor = '#FFD166';
    ctx.beginPath();
    ctx.arc(s.x, s.y, s.size, 0, Math.PI * 2);
    ctx.fill();
    ctx.restore();
  }
  for (const z of zzzs) {
    const a = 1 - z.life / z.maxLife;
    ctx.save();
    ctx.globalAlpha = clamp(a, 0, 1);
    ctx.fillStyle = '#B8B0C8';
    ctx.font = `${z.size}px sans-serif`;
    ctx.fillText('Z', z.x, z.y);
    ctx.restore();
  }
}

function drawShadow() {
  ctx.save();
  ctx.translate(CANVAS_SIZE / 2 + proc.dx, CANVAS_SIZE / 2 + DRAW_SIZE * 0.42 + proc.dy);
  ctx.scale(proc.squash, 1);
  ctx.shadowBlur = 14;
  ctx.shadowColor = 'rgba(0,0,0,0.45)';
  ctx.fillStyle = 'rgba(0,0,0,0.35)';
  ctx.beginPath();
  ctx.ellipse(0, 0, DRAW_SIZE * 0.35, DRAW_SIZE * 0.08, 0, 0, Math.PI * 2);
  ctx.fill();
  ctx.restore();
}

function draw() {
  ctx.clearRect(0, 0, CANVAS_SIZE, CANVAS_SIZE);

  if (!availableActions.has('respire')) {
    ctx.fillStyle = '#9E9AA8';
    ctx.font = '16px sans-serif';
    ctx.textAlign = 'center';
    ctx.fillText('Chargement de Swarmfox…', CANVAS_SIZE / 2, CANVAS_SIZE / 2);
    return;
  }

  drawAimGuide();
  drawShadow();

  // Fade with easing: avoids the double draw when almost finished (>0.92).
  const fadeAlpha = easeInOutCubic(fadeT);
  if (prevFrame && fadeT < 0.92 && images[prevFrame.name]) {
    drawSprite(prevFrame.name, prevFrame.idx, 1 - fadeAlpha, '');
    drawSprite(current, frameIndex, fadeAlpha, '');
  } else {
    drawSprite(current, frameIndex, 1, '');
    if (fadeT < 1) prevFrame = null;
  }

  // Lip-sync: overlay the speak clip (or play it as a fallback) modulated by amplitude.
  if (speakActive && !reducedMotion && speakMouthClip && images[speakMouthClip]) {
    const alpha = clamp(0.25 + 0.75 * speakAmplitude, 0, 1);
    drawSprite(speakMouthClip, Math.floor(speakMouthFrame), alpha, '');
  }

  if (ballState !== 'return') drawBall();
  drawParticles();
}

let harnessBboxCache = {};
let harnessCanvas = null;
let harnessCtx2d = null;

function getSpriteBBox(name, idx) {
  const key = `${name}:${Math.floor(idx)}`;
  if (harnessBboxCache[key]) return harnessBboxCache[key];
  const imgs = images[name];
  if (!imgs) return null;
  const img = imgs[Math.floor(clamp(idx, 0, imgs.length - 1))];
  if (!harnessCanvas) {
    harnessCanvas = document.createElement('canvas');
    harnessCanvas.width = DRAW_SIZE;
    harnessCanvas.height = DRAW_SIZE;
    harnessCtx2d = harnessCanvas.getContext('2d', { willReadFrequently: true });
  }
  harnessCtx2d.clearRect(0, 0, DRAW_SIZE, DRAW_SIZE);
  harnessCtx2d.drawImage(img, 0, 0, DRAW_SIZE, DRAW_SIZE);
  const data = harnessCtx2d.getImageData(0, 0, DRAW_SIZE, DRAW_SIZE).data;
  let minX = DRAW_SIZE, minY = DRAW_SIZE, maxX = 0, maxY = 0;
  for (let y = 0; y < DRAW_SIZE; y++) {
    const row = y * DRAW_SIZE;
    for (let x = 0; x < DRAW_SIZE; x++) {
      if (data[(row + x) * 4 + 3] > 12) {
        if (x < minX) minX = x;
        if (x > maxX) maxX = x;
        if (y < minY) minY = y;
        if (y > maxY) maxY = y;
      }
    }
  }
  if (maxX < minX) return null;
  const bbox = {
    left: minX - DRAW_SIZE / 2,
    top: minY - DRAW_SIZE / 2,
    right: maxX - DRAW_SIZE / 2,
    bottom: maxY - DRAW_SIZE / 2,
    width: maxX - minX,
    height: maxY - minY
  };
  harnessBboxCache[key] = bbox;
  return bbox;
}

function roundedRectPath(ctx, x, y, w, h, r) {
  const rr = Math.min(r, w / 2, h / 2);
  ctx.beginPath();
  ctx.moveTo(x + rr, y);
  ctx.lineTo(x + w - rr, y);
  ctx.quadraticCurveTo(x + w, y, x + w, y + rr);
  ctx.lineTo(x + w, y + h - rr);
  ctx.quadraticCurveTo(x + w, y + h, x + w - rr, y + h);
  ctx.lineTo(x + rr, y + h);
  ctx.quadraticCurveTo(x, y + h, x, y + h - rr);
  ctx.lineTo(x, y + rr);
  ctx.quadraticCurveTo(x, y, x + rr, y);
  ctx.closePath();
}

function drawHarnessPastille(cx, cy) {
  if (!harness) return;
  const colors = {
    local: '#E48A46',
    reflexion: '#E48A46',
    claude: '#D97B4F',
    kimi: '#8B7FD4'
  };
  const color = colors[harness];
  if (!color) return;

  const bbox = getSpriteBBox(current, frameIndex);
  if (!bbox) return;

  // Ground badge under the fox: same info as the harness, zero accessory
  // on the mascot. Follows horizontal movement, set under the paws.
  // The bbox is in DRAW_SIZE space centered on the sprite.
  const x = cx + (bbox.left + bbox.width / 2) * proc.squash;
  const y = cy + bbox.bottom * proc.stretch + 14;
  const r = 9;

  ctx.save();
  ctx.shadowColor = 'rgba(0,0,0,0.4)';
  ctx.shadowBlur = 5;
  ctx.shadowOffsetY = 1;
  ctx.fillStyle = color;
  ctx.beginPath();
  ctx.arc(x, y, r, 0, Math.PI * 2);
  ctx.fill();
  ctx.shadowColor = 'transparent';
  ctx.strokeStyle = 'rgba(255,255,255,0.55)';
  ctx.lineWidth = 1.5;
  ctx.stroke();
  ctx.restore();
}

function loop(now) {
  if (!paused) {
    const dt = Math.max(0.001, Math.min((now - lastTime) / 1000, 0.05));
    lastTime = now;
    update(dt);
    draw();
  }
  rafId = requestAnimationFrame(loop);
}

function stopSpeak() {
  if (!speakActive) return;
  speakActive = false;
  speakAmplitude = 0;
  speakMouthFrame = 0;
  speakMouthClip = null;
  if (speakSource) { try { speakSource.stop(); } catch {} speakSource = null; }
  if (speakAudioCtx) { try { speakAudioCtx.close(); } catch {} speakAudioCtx = null; }
  speakAnalyser = null;
  speakDataArray = null;
  if (current === 'joue' && speakReturnAction) {
    playNow(speakReturnAction);
  } else if (speakReturnAction && current !== speakReturnAction) {
    queueAction(speakReturnAction, 'speak-end');
  }
}

async function speakWith(audioUrl) {
  if (!audioUrl) return;
  stopSpeak();
  speakReturnAction = current;
  speakActive = true;
  speakAmplitude = 0;
  speakMouthFrame = 0;

  const hasParle = await loadAction('parle');
  speakMouthClip = hasParle ? 'parle' : (images['joue'] ? 'joue' : null);
  if (speakMouthClip === 'joue') {
    playNow('joue');
  }

  try {
    const AudioCtx = window.AudioContext || window.webkitAudioContext;
    if (!AudioCtx) throw new Error('Web Audio not supported');
    const audioCtx = new AudioCtx();
    const resp = await fetch(audioUrl);
    if (!resp.ok) throw new Error(`fetch ${audioUrl} ${resp.status}`);
    const buffer = await resp.arrayBuffer();
    const decoded = await audioCtx.decodeAudioData(buffer);
    const source = audioCtx.createBufferSource();
    source.buffer = decoded;
    const analyser = audioCtx.createAnalyser();
    analyser.fftSize = 256;
    source.connect(analyser);
    analyser.connect(audioCtx.destination);
    source.onended = () => stopSpeak();
    speakAudioCtx = audioCtx;
    speakAnalyser = analyser;
    speakSource = source;
    speakDataArray = new Uint8Array(analyser.fftSize);
    if (audioCtx.state === 'suspended') await audioCtx.resume();
    source.start(0);
    setTimeout(() => { if (speakActive) stopSpeak(); }, decoded.duration * 1000 + 300);
  } catch (e) {
    console.warn('[fox] speakWith failed, returning idle:', e);
    stopSpeak();
  }
}

/* Goupil contract implementation */
function installContract() {
  const api = {
    setPet: setPetInput,
    setHealth: setHealthInput,
    setActivity: setActivityInput,
    setOffline: setOfflineInput,
    setHarness: setHarnessInput,
    setPersona: choosePersona,
    setMood: () => {},
    setStale: () => {},
    setReducedMotion: (on) => { reducedMotion = !!on; },
    setEvent: (name) => {
      if (name === 'notify' || name === 'unlock') {
        bumpInteraction();
        queueInteraction('salue', 1.8);
      }
    },
    say: (text) => {
      const quip = document.getElementById('quip');
      const txt = document.getElementById('quip-text');
      if (!quip || !txt) return;
      if (text) { txt.textContent = text; quip.hidden = false; }
      else { quip.hidden = true; }
    },
    pause: () => { paused = true; },
    resume: () => { paused = false; lastTime = performance.now(); },
    speakWith,
    setLoad: () => {},
    setReasons: () => {},
    playAction: playNow,
    seekFrame: (action, idx) => { playNow(action); frameIndex = idx; },
    loadAction: loadAction
  };

  // Legacy global aliases used by some debug harnesses.
  window.setHealth = setHealthInput;
  window.setFoxState = setFoxState;
  window.spawnBall = spawnBall;
  window.triggerPet = triggerPet;
  window.triggerTail = triggerTail;
  window.triggerHug = triggerHug;

  if (!window.fox) window.fox = {};
  Object.defineProperty(window.fox, 'currentAction', { get: () => current });
  Object.defineProperty(window.fox, 'frameIndex', { get: () => frameIndex });
  const queue = window.fox.q || [];
  for (const [k, fn] of Object.entries(api)) window.fox[k] = fn;
  for (const [k, args] of queue) {
    if (window.fox[k]) window.fox[k](...args);
  }
  if (window.FoxHost && window.FoxHost.ready) window.FoxHost.ready();
}

loadBaseActions().then(() => {
  current = stateAction(state);
  installContract();
  if (!rafId) rafId = requestAnimationFrame(loop);
  loadOptionalActions();
}).catch((err) => {
  console.error('Failed to load fox sprites:', err);
});
