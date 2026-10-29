# Passation — finir Kitsune : la page web

Ce document est un **prompt de passation autonome** pour un agent local. Il décrit tout ce qui
est déjà fait et **la seule tâche restante : la page web Kitsune**. Tout le cerveau et son
intégration au cœur sont déjà fusionnés dans `main` du dépôt `bengua/inari`.

## 0. Règles du projet (à respecter absolument)

- **Local-first strict** : aucun appel cloud. Les modèles (voix, transcription, embeddings) sont
  des serveurs locaux joints en HTTP.
- **Rien en dur, tout en réglage dans la GUI** : adresses, modèles, voix, jetons, outils.
- **Ne change pas l'esthétique d'Inari** : réutilise les composants, couleurs et polices existants
  (fond sombre, accent `#E48A46`, polices IBM Plex / Bricolage ; voir `crates/inari-core/web/app.css`).
- **Rust only** côté cœur ; la page est du HTML/CSS/JS pur, sans framework ni dépendance externe.
- **Git/CI** : une branche par incrément repartie de `main` à jour
  (`git fetch origin main && git checkout -B <branche> origin/main`), **jamais** sur `main`.
  `./verify.sh` doit passer. Ouvre une PR vers `main`, laisse la CI (`CI / lint-test`) tourner, et
  **fusionne en squash quand elle est verte** (ou corrige si rouge). Le `--force-with-lease` est
  refusé par le garde-fou local : si une branche diverge, prends-en une neuve.
- **Attribution** à la fin de chaque message de commit :

      Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
      Claude-Session: https://claude.ai/code/session_01MKEMPfYDqWj9wRJ3EUqpsE

  (Adapte à ton propre agent si besoin.)
- **Docs en français** : ajoute tout nouveau fichier `docs/*-fr.md` à `.english-exceptions` avec sa
  raison (format `chemin # raison`).

## 1. Ce qui est déjà fait (ne pas refaire)

Le crate `crates/kitsune` (MIT, pur Rust, compile en statique musl) et son intégration au cœur,
tous fusionnés dans `main` :

| PR | Contenu |
|----|---------|
| #4 | **store** SQLite : notes + FTS5 + embeddings + audio + déclencheurs ; migrations testées |
| #5 | **recherche hybride** RRF (BM25 + cosinus), filtres date/lieu/tags |
| #6 | **déclencheurs** temps/géofence/événement |
| #7 | **serveur MCP** (`crates/kitsune/src/mcp.rs`) : outils Kitsune, scope + jeton |
| #8 | **chiffrement** AES-256-GCM de l'audio, clé fichier mode 600 ; base via LUKS |
| #9 | **capture vocale** : scelle l'audio → transcrit (Whisper HTTP) → range la note |
| #10 | **voix** : `speak` (TTS HTTP), scelle l'audio, outil MCP `synthesize_speech` |
| #11 | **intégration au cœur** : la passerelle expose l'API `/api/kitsune` (voir §3) |

La façade `kitsune::Kitsune` (fichier `crates/kitsune/src/brain.rs`) ouvre le store, le coffre
audio scellé et lit ses réglages dans `config.json`. Le cœur en tient une par machine, à
`<data_dir>/kitsune/`, ouverte dans `Gateway::new` (`crates/inari-core/src/gateway.rs`).

**Décision d'architecture déjà prise** : ASR et TTS parlent à des **serveurs locaux en HTTP**
(endpoints compatibles OpenAI `/audio/transcriptions` et `/audio/speech`), jamais compilés en dur,
parce que `whisper-rs`/`candle`/SQLCipher compilent du C/C++ et cassent le binaire statique musl.
Le choix du moteur de voix et le **test d'écoute A/B en français se font sur les postes de Ben**,
là où le modèle tourne — pas dans le code.

## 2. La tâche restante : la page web Kitsune

Construire une page web Kitsune, servie par la passerelle, qui consomme l'API `/api/kitsune`, avec
la mascotte **FoxBot** déjà déployée (iframe `/fox/`, contrat `window.fox`). C'est le pense-bête
vocal de Ben : on parle, il garde la note, la retrouve, la relit, et rappelle au bon moment.

### Fonctions attendues (incréments testables, un PR chacun si possible)

1. **Coquille + mascotte** : page `/kitsune`, fond braises (canvas, déjà esquissé dans la branche
   `kitsune-web`, voir §5), iframe FoxBot `/fox/index.html`, pilotée via `window.fox`
   (`setActivity("listening"|"working"|"idle")`, `setMood`, `say(texte)`). État « veille / écoute /
   réflexion » affiché.
2. **Capture** : un bouton **appui-pour-parler** (push-to-talk). Au relâchement : enregistre,
   encode en **WAV 16 kHz mono** (voir §4), envoie `ingest_voice` ; FoxBot passe en
   `listening` pendant l'enregistrement puis `working` pendant la transcription. Affiche la note
   obtenue (ou l'état `needs_review` si la transcription a échoué, avec l'audio rejouable et un
   champ pour corriger le texte → `correct_note`). Un champ texte pour `capture_text` aussi.
3. **Mémoire** : une recherche (champ → `search`) qui liste les notes (texte, date, lieu, tags),
   chaque note avec **Relire** (→ `speak` puis joue l'audio renvoyé), Corriger, Supprimer.
4. **Rappels** : voir/poser/retirer des déclencheurs (`list_triggers` / `create_trigger` /
   `delete_trigger`) — au moins temps (date/heure, récurrence) et lieu (lat/lon/rayon).
5. **Réglages** (indispensable, « pas en dur ») : une fiche qui lit/écrit `set_settings` :
   - serveur ASR (url, modèle, langue) ;
   - serveur TTS (url, modèle, voix, format) ;
   - modèle d'embeddings ;
   - MCP : jeton réseau + outils offerts (pour brancher n8n / Home Assistant / Android).
   Montre `asr_ready` / `tts_ready` (du `GET /api/kitsune`). Jamais réafficher un secret déjà posé.

### Contraintes techniques de la page

- **Sécurité CSP** (voir `const CSP` dans `gateway.rs`) : `script-src 'self'` → **aucun script
  inline**. Mets tout le JS dans un asset externe (ex. `web/kitsune.js`), référencé par la page.
  `style-src 'self' 'unsafe-inline'` → le CSS inline (`<style>`, attributs `style`) est autorisé.
- **Auth** : la page est servie même origine ; elle s'authentifie par le **cookie de session**
  posé à la connexion (comme `app.js`). Un lien peut porter `#token=…` au premier chargement ; voir
  le helper `api()` et `legacySecret()` dans `app.js` pour copier le schéma exact (POST `/api/session`
  pour échanger le token contre le cookie). Sur un **401**, invite à se connecter via la page
  principale `/`.
- **Mascotte** : embarque `/fox/index.html` en iframe même origine ; pilote-la par
  `frame.contentWindow.fox.<methode>(…)` comme le fait `mountFox()`/`fox()` dans `app.js`. Ne
  duplique pas le moteur de sprite.
- **i18n** : la page est pour Ben, en français ; garde le style des libellés existants.

### Côté cœur (Rust) à ajouter pour servir la page

Dans `crates/inari-core/src/gateway.rs` :
- une route `GET /kitsune` (et éventuellement `/kitsune.html`) qui renvoie `web/kitsune.html`
  (via `include_str!`, comme `INDEX`), en remplaçant un éventuel marqueur de version comme le fait
  la route `/` (`INDEX.replace("__INARI_PAGE__", page_version())`) ;
- ajoute `web/kitsune.js` (et un `web/kitsune.css` si tu préfères) au tableau `ASSETS`
  (`include_bytes!`), servis sous `/assets/` ; leur hash entre alors dans `page_version()`
  automatiquement si tu suis le motif existant.
- Vérifie la CSP : si tu charges `/assets/kitsune.js` et l'iframe `/fox/`, `script-src 'self'` et
  `frame-src 'self'` suffisent (le CSP actuel autorise déjà l'iframe même origine ; confirme).

## 3. L'API `/api/kitsune` (déjà en place, à consommer)

Toutes réservées au **propriétaire** (cookie de session), **sauf** `/api/kitsune/mcp` qui accepte
aussi le jeton MCP du cerveau en `Authorization: Bearer <jeton>`.

- `GET /api/kitsune` → `{settings, asr_ready, tts_ready, notes, triggers}`.
- `GET /api/kitsune/audio?path=<rel>` → les octets audio déchiffrés (à jouer).
- `POST /api/kitsune` avec `{action, …}` :
  - `set_settings` `{settings}` — `settings` = l'objet `kitsune::Settings` (voir `brain.rs` :
    `{asr:{url,model,language,timeout}, tts:{url,model,voice,format,timeout}, embed_model,
    mcp:{token,tools,embed_model}}`).
  - `capture_text` `{text, tags?}` → `{id}`.
  - `ingest_voice` `{audio (base64 WAV), place?, lat?, lon?}` → `{id}`.
  - `search` `{query, k?, filters?}` → `{hits:[{note, score, text_rank, vector_rank}]}`.
    (Recherche par mots pour l'instant ; par sens quand un modèle d'embeddings est réglé.)
  - `speak` `{text, note_id?}` → `{path, mime, audio (base64)}`.
  - `get_note` `{id}` / `correct_note` `{id, text}` / `delete_note` `{id}`.
  - `create_trigger` `{kind:"time"|"geo"|"event", spec, note_id?, next_at?}` → `{id}`.
    - temps : `spec = {at: <epoch secondes>, every?: <secondes>}`, et `next_at = at`.
    - lieu : `spec = {lat, lon, radius_m, on:"enter"|"exit"}`.
    - événement : `spec = {event:"boot"|"network"|…}`.
  - `list_triggers` → `{triggers:[…]}` / `delete_trigger` `{id}` → `{removed}`.
- `POST /api/kitsune/mcp` (JSON-RPC MCP) : `initialize`, `tools/list`, `tools/call`. C'est la porte
  du réseau agentique ; tu n'en as pas besoin pour la page, mais le réglage du jeton (via la fiche)
  l'ouvre à n8n/HA.

Exemple (dans la page, avec le cookie déjà posé) :

```js
const r = await fetch("/api/kitsune", {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  credentials: "same-origin",
  body: JSON.stringify({ action: "capture_text", text: "Acheter du pain", tags: ["courses"] }),
});
const { id } = await r.json();
```

## 4. Encoder l'audio en WAV 16 kHz mono (dans le navigateur)

`MediaRecorder` produit du webm/opus, mais le contrat ASR attend du WAV (`asr.rs` étiquette
`capture.wav`, `audio/wav`). **N'élargis pas le crate** : encode en WAV côté page. Marche à suivre :

1. `getUserMedia({audio:true})` → `MediaRecorder` → collecte les chunks → `Blob`.
2. `const buf = await blob.arrayBuffer();`
   `const ctx = new AudioContext(); const decoded = await ctx.decodeAudioData(buf);`
3. **Rééchantillonne à 16 kHz mono** : `OfflineAudioContext(1, Math.ceil(decoded.duration*16000),
   16000)`, y rejoue la source, `startRendering()` → `AudioBuffer` 16 kHz mono.
4. **Encode WAV PCM 16 bits** (en-tête RIFF 44 octets + échantillons `Int16`). ~40 lignes ; motif
   RIFF/WAVE/fmt /data classique. Convertis `Float32` → `Int16` (`max(-1,min(1,x))*0x7FFF`).
5. base64 du résultat → `ingest_voice {audio}`.

Whisper veut justement du 16 kHz mono : c'est le bon format, sans transcodage côté serveur.

## 5. Réutiliser la coquille FoxBot (branche `kitsune-web`)

La branche `origin/kitsune-web` (dépôt `bengua/inari`, **basée sur un `main` ancien** — ne la merge
pas telle quelle) contient la coquille visuelle déjà esquissée par Ben :
- `crates/inari-core/web/kitsune.html` : page autonome (canvas « braises » en fond + iframe FoxBot +
  un HUD d'état) ;
- `crates/inari-core/web/foxbot/index.html` : le contrat `window.fox` (méthodes
  `setActivity, setHarness, setMood, setHealth, say, onTap, pause, resume…`) ;
- `crates/inari-core/web/foxbot/js/sprite.js` : le moteur de sprite 2D (1523 lignes) ;
- `crates/inari-core/web/foxbot/sprites` : **un lien symbolique** vers les sprites de `inari-apk`
  (`swarmfox/android/app/src/main/assets/fox/sprites`). Attention : ce lien ne survit pas hors du
  poste de Ben. Pour que la page marche partout, **réutilise la mascotte déjà servie par la
  passerelle sous `/fox/`** (champ `mascot_dir`, cf. route `"/fox/"` dans `gateway.rs`) au lieu de
  réembarquer le moteur. Si `/fox/` n'est pas monté sur un cœur, prévois un repli discret (un point
  animé, comme `orbInit` dans `app.js`).

Récupère le contenu utile avec `git show origin/kitsune-web:<chemin>` ; **repars de `main` à jour**
et apporte seulement ce qui sert, en l'adaptant à l'API `/api/kitsune`.

## 6. Vérifications avant chaque PR

- `./verify.sh` (format, clippy `-D warnings`, tests, build). Si tu ajoutes des routes/handlers
  Rust, ajoute un test d'intégration passerelle sur le modèle de
  `crates/inari-core/tests/kitsune_api.rs` (démarre un `Node`, tape l'API en HTTP).
- **Essai navigateur** avec le Playwright/Chromium préinstallés (`require('playwright')`,
  `chromium.launch()` sans option). Lance un cœur d'essai (voir les scripts d'essai antérieurs sous
  le scratchpad, ex. un `node.json` minimal avec `gateway_listen: 127.0.0.1:0`, un `token`), ouvre
  `http://127.0.0.1:<port>/kitsune#token=owner`, et teste ce qui ne nécessite pas de micro :
  `capture_text`, `search`, `speak` (contre un faux serveur TTS HTTP qui **lit toute la requête
  avant de répondre** — sinon course et reset ; piège déjà rencontré), les réglages, le chargement
  de FoxBot. Le vrai micro et l'écoute se valident sur un appareil de Ben.
- **Binaire statique** : `CC_x86_64_unknown_linux_musl=cc
  CFLAGS_x86_64_unknown_linux_musl="-U_FORTIFY_SOURCE -D_FORTIFY_SOURCE=0 -D_FILE_OFFSET_BITS=32"
  cargo build --release --target x86_64-unknown-linux-musl -p inari-core --bin inari-core-rs`,
  puis `strip`. Le binaire doit rester `static-pie`. (Pas de `musl-gcc` sur certains postes : les
  flags ci-dessus suffisent avec `cc` pour le SQLite embarqué.)

## 7. Pièges déjà rencontrés (gagne du temps)

- Un **faux serveur HTTP de test** doit lire **toute** la requête (en-têtes + `Content-Length`)
  avant de répondre et fermer, sinon le client prend un reset en pleine écriture (CI rouge
  intermittent). Voir `fake_asr`/`fake_tts` dans `crates/kitsune/src/brain.rs`.
- **SQLCipher** et toute crypto C cassent le statique musl ici (`__isoc23_strtol`) : ne pas
  l'ajouter. Le chiffrement de l'audio est déjà fait en AES-GCM pur Rust ; la base compte sur LUKS.
- La **branche désignée squash-mergée** ne peut pas être repoussée sans force (refusé) : prends une
  branche neuve par incrément.

## 8. Ordre conseillé

1. Route `/kitsune` + asset `kitsune.js` + coquille + FoxBot via `/fox/` (PR).
2. Capture (push-to-talk, WAV 16k, `ingest_voice`, `needs_review`/correction) (PR).
3. Mémoire (recherche, relire, corriger, supprimer) (PR).
4. Rappels (déclencheurs) (PR).
5. Réglages (ASR/TTS/embeddings/MCP) (PR).

Quand tout est là et vert : binaire statique à jour pour Ben, et une note dans `docs/kitsune-fr.md`
(section « La page »). Le test d'écoute A/B de la voix en français reste à faire par Ben, sur ses
postes, avec un vrai serveur TTS (CSM / Qwen3-TTS en priorité sur Kokoro, la fidélité prime).
