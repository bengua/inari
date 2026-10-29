# Intégrer AI-Memory dans Inari — plan et passation

Prompt de passation autonome pour un agent local : **intégrer `akitaonrails/ai-memory` (Rust, MIT)
dans Inari**. Tout ce qu'il faut savoir et faire est ici.

## 0. Ce qu'est AI-Memory (le projet à intégrer)

`github.com/akitaonrails/ai-memory` — mémoire long terme pour agents de code, un seul binaire Rust
qui possède un dossier de données (`wiki/` markdown git, `raw/`, `db/` SQLite FTS5 + embeddings,
`models/`, `logs/`). Capture silencieuse par hooks, recall par recherche (RRF : FTS5 + entités +
graphe + vecteurs optionnels), vieillissement de la mémoire, et surtout **handoff entre
fournisseurs** : quitter Claude Code en plein travail, reprendre avec Codex dans le même dossier
sans tout réexpliquer. MIT. Loopback par défaut.

Interfaces :
- **Serveur MCP** (+ pont stdio `ai-memory mcp-bridge`). Outils connus : `memory_briefing`,
  `memory_query`, `memory_handoff_list`, `memory_read_session_observations` (liste non exhaustive).
- **CLI** : `setup, init, install-mcp, install-hooks, run, continue, resume, status, serve,
  reindex, doctor…`. `ai-memory run <harness>` lance un agent avec le workstream partagé.
- **HTTP** : `ai-memory serve --transport http` (+ `--enable-web`, `--enable-management-ui`…).
- Réglages : `config.toml`, marqueur `.ai-memory.toml` par projet, variables
  `AI_MEMORY_LLM_PROVIDER`, `AI_MEMORY_EMBEDDING_PROVIDER`, `AI_MEMORY_AUTH_TOKEN`, etc.

## 1. Décision d'architecture (déjà tranchée — ne pas revenir dessus)

**On N'intègre PAS le code d'AI-Memory dans `inari-core`.** C'est un projet tiers de 14k★, très
actif, avec son propre binaire et sa propre base. Le vendoriser serait un fardeau de maintenance et
de licence, et gonflerait le cœur.

**On intègre par câblage : AI-Memory reste un service local, et Inari y branche ses agents via
MCP** — exactement le patron déjà en place dans Inari :
- le **moteur local** a déjà un client MCP (`crates/inari-core/src/engine/mcp.rs`) qui joint des
  serveurs MCP en **stdio** (une commande) ou **http** ;
- Kitsune et les serveurs ASR/TTS sont joints en HTTP, réglés dans la GUI, jamais en dur.

Donc AI-Memory devient **un serveur MCP que les agents d'Inari utilisent**, plus (optionnel) un
**service que le cœur sait démarrer et régler depuis la GUI**. Zéro dépendance C ajoutée au cœur,
local-first, tout en réglage.

**AI-Memory vs Kitsune** : distincts et complémentaires. Kitsune = le second cerveau **vocal
personnel de Ben** (notes dictées). AI-Memory = la mémoire **de code des agents** (connaissance
projet + handoff entre fournisseurs). Ne pas les fusionner ; les deux exposent du MCP.

## 2. Ce qui marche déjà sans rien coder

Le moteur local peut **dès aujourd'hui** utiliser AI-Memory : dans Réglages → Agents et
abonnements → le moteur local → Réglages → Serveurs MCP → « Ajouter un serveur MCP », type
« commande sur ce cœur », commande `ai-memory mcp-bridge` (le binaire AI-Memory doit être installé
sur le cœur). Ses outils `memory_*` deviennent disponibles aux agents du moteur local.

L'intégration à coder ci-dessous rend ça **premier-classe** (préréglage, démarrage géré, câblage
automatique des harnais frontier, briefing au début de fil).

## 3. Le travail à faire (incréments, un PR chacun)

Branche par incrément repartie de `main` à jour, `./verify.sh` vert, PR → fusion squash sur CI
verte. Attribution de commit comme le reste du projet. Docs FR → `.english-exceptions`.

### Incrément A — Préréglage MCP « AI-Memory » (petit, haute valeur)

But : ajouter AI-Memory en un clic comme serveur MCP du moteur local.
- Côté GUI (`crates/inari-core/web/app.js`, fonction `foxMcp`) : un bouton/préréglage qui
  pré-remplit le formulaire d'ajout MCP avec transport stdio et commande `ai-memory mcp-bridge`
  (nom `ai-memory`). Rien en dur au-delà du préréglage éditable.
- Côté moteur (`crates/inari-core/src/engine/mcp.rs`) : rien à changer (le client MCP stdio existe).
- Test : étendre `tests/engine.rs` (un faux serveur MCP, déjà fait pour `fake_echo`) pour vérifier
  qu'un serveur nommé `ai-memory` expose ses outils. i18n : ajoute les libellés FR à `web/i18n.js`
  et fais passer `tests/i18n.rs`.

### Incrément B — Le cœur sait gérer le service AI-Memory (optionnel mais recommandé)

But : démarrer/arrêter/régler AI-Memory depuis la GUI, comme un serveur de modèle.
- Un réglage `AiMemoryConfig { binary_path, data_dir, http_url, auth_token (mode 600), embedding_provider, enabled }`
  gardé dans `<data_dir>/ai-memory.json` (+ le token à part, mode 600, comme `keys.json`/`forge_keys.json`).
- Le node (`crates/inari-core/src/node.rs`) lance `ai-memory serve --transport http` sur loopback
  quand `enabled`, via le même patron de processus géré que le serveur de dev de l'engine
  (`crates/inari-core/src/engine/project.rs`, struct `Proc`/`start`). Jamais d'hôte/chemin en dur.
- Une fiche GUI (Réglages) : chemin du binaire, dossier de données, fournisseur d'embeddings,
  jeton (jamais réaffiché), on/off, un bouton « Vérifier » (ping `GET /` ou `doctor`).
- Quand le service tourne en HTTP, le préréglage MCP de l'incrément A peut pointer dessus
  (transport http) au lieu du stdio, au choix.
- Tests d'intégration passerelle sur le modèle de `tests/kitsune_api.rs`.

### Incrément C — Câbler les harnais frontier (Claude Code, Codex, Gemini…)

But : que les agents ACP (pas seulement le moteur local) utilisent AI-Memory.
- AI-Memory s'installe par harnais via `ai-memory install-mcp <harness>` et/ou
  `ai-memory install-hooks <harness>`. Ajoute, dans la fiche d'un harnais
  (`harnessSheet`/`harnessAct` dans `app.js`, actions node `harness.*` dans `node.rs`), une action
  « Brancher la mémoire partagée » qui lance la commande d'installation adéquate sur le cœur et dit
  comment ça s'est passé (même patron que `harness_action`/`openfox_action`).
- Alternative plus simple et portable : pour les harnais ACP qui acceptent une config MCP, écrire
  leur config pour pointer sur le serveur MCP d'AI-Memory. Vérifie au cas par cas (Claude Code :
  `.mcp.json` / réglages ; Codex/Gemini : selon leur format).
- Respecte la vie privée annoncée par AI-Memory (le serveur distant ne reçoit que l'identité de
  session/harnais, pas le transcript). Ne contourne pas ça.

### Incrément D — Briefing au début de fil (confort)

But : au début d'un fil du moteur local, injecter un bref `memory_briefing` dans le contexte.
- Dans `crates/inari-core/src/engine/run.rs` (`conversation`/`system_for`), si un serveur MCP
  nommé `ai-memory` est présent et offre `memory_briefing`, appelle-le une fois et ajoute le
  résultat (borné) au prompt système, derrière un réglage « Mémoire partagée au démarrage » (on/off,
  pas en dur). Facultatif ; à faire après A/B.

### Incrément E — Docs

- `docs/ai-memory-fr.md` (ajouté à `.english-exceptions`) : ce qu'est AI-Memory, comment l'installer
  sur un cœur, comment le brancher (moteur local, harnais frontier), le handoff entre fournisseurs,
  et la distinction avec Kitsune. Mets à jour `docs/INSTALLER-BRANCHE-fr.md` (étapes de contrôle) et
  le `CHANGELOG.md`.

## 4. Contraintes (comme tout le projet)

- **Local-first** : AI-Memory en loopback, aucun secret/transcript vers le cloud sans opt-in.
- **Rien en dur** : chemins, URL, fournisseur d'embeddings, jeton — tout réglable dans la GUI, gardé
  dans `<data_dir>/…json` (secrets mode 600, convention `keys.json`).
- **Zéro C dans le cœur** : on parle à AI-Memory par MCP/HTTP, on ne lie pas son code ni des crates
  C (garde le binaire statique musl :
  `CC_x86_64_unknown_linux_musl=cc CFLAGS_x86_64_unknown_linux_musl="-U_FORTIFY_SOURCE -D_FORTIFY_SOURCE=0 -D_FILE_OFFSET_BITS=32" cargo build --release --target x86_64-unknown-linux-musl -p inari-core --bin inari-core-rs`).
- **Esthétique** inchangée (réutilise `foxMcp`, `foxForm`, `harnessSheet`, le style existant).
- **Licences MIT/Apache** seulement ; AI-Memory est MIT : on peut s'y appuyer, et son binaire reste
  séparé. Ne copie pas son code dans le dépôt.

## 5. Vérifications

- `./verify.sh` à chaque PR ; tests d'intégration (faux serveur MCP / passerelle) sur les modèles
  `tests/engine.rs` et `tests/kitsune_api.rs` ; `tests/i18n.rs` pour les libellés FR.
- Essai navigateur Playwright/Chromium préinstallé sur un cœur d'essai : ajouter le serveur MCP
  `ai-memory` depuis la fiche du moteur local et voir ses outils listés (contre un faux binaire
  `ai-memory` qui répond au protocole MCP sur stdin/stdout, suffisant pour le test GUI).
- Piège connu : un faux serveur de test doit lire TOUTE la requête avant de répondre (cf.
  `fake_asr`/`fake_tts` dans `crates/kitsune/src/brain.rs`).

## 6. Ordre conseillé

A (préréglage MCP) → B (service géré + fiche GUI) → C (harnais frontier) → D (briefing) → E (docs).
A seul apporte déjà l'essentiel : les agents du moteur local utilisent AI-Memory.
