# Phase 0 — Inventaire d'Inari (lecture seule)

Date : 2026-11-02. Repo audité : `/home/zordur/inari-swarm-coordination` (origin : `github.com/bengua/inari`), commit `6cf4974` (main). Méthode : lecture seule du code, des docs, des tests et de la CI ; aucune modification du code, aucune installation. Chaque affirmation renvoie à une preuve (fichier, test, run). Ce qui n'a pas pu être vérifié est marqué **inconnue**, jamais supposé.

## Cadrage donné par le propriétaire (2026-11-02)

Les références théoriques du prompt de mission (Omnigent, Cloudroom core, super-simple-software-factory) restent valables **comme références**, mais les briques concrètes à évaluer sont celles de l'infra :

- **Inari joue la couche 3 (meta-harness).** Ce n'est pas Cloudroom qui la joue.
- **OpenFox joue la couche 4 (harnais).** Ce n'est pas Pi.
- Les « cloudroom-style » à évaluer en Phase 1 sont les sources présentes sur l'infra (`~/Work/cloudroom-core-src`, `~/Work/cloudfoxroom`, `~/Work/foxroom`, `~/Work/cloudroom-gui`, `~/Work/bb-plugin-provider-openfox`), pas seulement le repo upstream.

## 1. Objectif actuel du projet (tel qu'il ressort du code et de la doc)

D'après `README.md` : un **réseau local-first d'agents IA pour les machines qu'on possède déjà**. L'idée centrale : l'état d'un agent est un **fichier portable** (la capsule `ssc/1`, un tar de JSON et Markdown avec journal chaîné par hachage, lisible sans Inari — `docs/ssc.md`), pas une ligne dans la base d'un fournisseur. Tout le reste en découle : changement de modèle ou de provider au milieu d'un fil, déplacement d'un agent en cours entre machines (téléportation 2PC, `docs/teleport.md`), téléphone de supervision sans calcul, fonctionnement hors-ligne, passes familiaux (supervision d'un enfant, collaboration entre adultes), aucun compte ni télémétrie, licence MIT partout.

## 2. Arborescence, langages, dépendances

```
├── Cargo.toml            workspace Rust, 2 crates, licence MIT (workspace)
├── verify.sh             contrat du repo : forme + licence MIT, rustfmt, clippy -D warnings, tests, build (+ english-check si installé)
├── crates/
│   ├── inari-core/       le cœur : mesh, capsule, gateway HTTP/SSE, moteur local OpenFox, conduite des harnais
│   │   ├── src/          27 fichiers + capsule/ (6) + engine/ (16) — 30 757 lignes Rust (avec kitsune)
│   │   ├── tests/        22 fichiers de tests d'intégration — 7 681 lignes
│   │   └── web/          l'interface, embarquée dans le binaire : app.js (1 431 l.), kitsune.js (1 374 l.), CSS, i18n — 4 145 lignes, non bundlée
│   └── kitsune/          second cerveau vocal : notes, index, déclencheurs, audio, en une base SQLite locale (brain.db)
├── android/              l'app Android (Kotlin/Gradle), versionName 0.40.7, versionCode 85
├── schema/               6 schémas JSON des contrats fil (snapshot, timeline-event, approval-*, manifest, environment)
├── services/kitsune-voice/  service Python (pyproject) avec ses tests — hors workspace, hors verify.sh
├── docs/                 17 documents + adr/0001-clean-break.md
└── THIRD-PARTY.md        notices : OpenFox MIT (prompts/outils de engine/), livekit-client vendorisé (596 K)
```

Langages : Rust (cœur), JavaScript/CSS/HTML (interface, sans bundler), Kotlin (Android), Python (kitsune-voice).

### Dépendances et licences

85 paquets Rust uniques en dépendances normales (`cargo tree -e normal`). Licences relevées : `MIT OR Apache-2.0` (majorité), `MIT`, `Apache-2.0`, `BSD-2-Clause`, `BSD-3-Clause`, `Unlicense OR MIT`, `Unicode-3.0` (composant de `MIT OR Apache-2.0 AND Unicode-3.0`), `Apache-2.0 WITH LLVM-exception`. **Toutes sont permissives ; aucune copyleft détectée.** Les deux crates locales déclarent `license.workspace = true` → MIT. Composants tiers adoptés : OpenFox (MIT, notice dans THIRD-PARTY.md), livekit-client (vendorisé dans `web/vendor/`), AI-Memory (MIT, service externe piloté, jamais lié — `aimemory.rs`).

## 3. Ce qui fonctionne réellement (preuves) vs ébauches

### Prouvé par test ou CI

- `./verify.sh` : **PASSED** le 2026-11-01 (rustfmt, clippy sans warning, tous les tests, build).
- Tests : **228 passés, 3 ignorés**, 0 échoué (run local du 2026-11-01, toutes suites confondues : unit, capsule, teleport, gestures, harnesses, i18n, kitsune_api, openfox, passes, personas, sessions, updates…).
- CI GitHub (3 jobs : `verify`, `android`, `voice`) : verte sur les PR #40 → #44 (2026-10-31 → 2026-11-01).
- 4 tests i18n dédiés imposent que tout texte de l'interface ait son français et que les placeholders `{n}` concordent.

### Observé en exploitation (preuve : session de travail du 2026-10-31 → 2026-11-01)

- Binaire `inari-core-rs` déployé et actif (systemd user) sur 3 cœurs — gamer (cachyos-x8664), laptop (omarchy-elitebook), mindhub — servant l'interface sur `:7487`, 24 à 28 agents visibles dans le mesh.
- Téléportation entre cœurs, multi-sélection de fils, réglages Kitsune servis par l'API (`/api/kitsune`), utilisés en conditions réelles.

### Ébauches, outillage ou hors contrat

- `demo.rs` / `demo_agent.rs` : un mesh de démonstration compilé **dans** le binaire de production (trois nœuds loopback + agent ACP jouet). Utile, mais c'est de l'outillage.
- `services/kitsune-voice/` (Python) : couvert par le job CI `voice` (`.github/workflows/ci.yml` : `pip install .` + `python -m unittest`, Python 3.12), mais **pas** par `verify.sh` — asymétrie assumée dans le fichier (« Rust only »), donc deux mondes de vérification dont un seul tourne en local.
- Capsules écrites par l'ancienne référence Python : conservées comme fixtures de test (`tests/python_capsules.rs`, lues hachage pour hachage) — la référence elle-même est retirée (ADR 0001, « clean break »).
- Docs : mélange EN/FR ; 9 exceptions déclarées dans `.english-exceptions` (les FR sont assumés « pour le propriétaire »).

## 4. Ce qui joue déjà un rôle de façade, d'orchestration, d'exécution, de politique ou de trace

| Couche cible | Ce qui existe dans Inari | Preuve | Lacune face à la vision |
|---|---|---|---|
| **1. Façade** (intention → workflow, rapports humains) | Interface web servie par `gateway.rs` (snapshot SSE, 41 endpoints `/api/*`), page Kitsune (voix : ASR/TTS/LiveKit), app Android | `gateway.rs`, `kitsune/`, `android/` | Pas de couche « intention structurée » dédiée : la façade actuelle parle aux fils, elle ne déclenche pas de workflow multi-phases. Petit modèle de façade (type LFM3) : **inexistant**. |
| **2. Graphe déterministe** (phases, retries, gates, phases humaines) | `engine/workflow.rs` : machine à états (agent, sous-agent, commande shell, **étape pour la personne**, enfants en parallèle) — au niveau d'un thread OpenFox | `engine/workflow.rs` (docstring), onglet Plan de l'UI | Rien au niveau essaim : pas de graphe cross-agents possédé par du code, pas d'envelopes JSON typées entre phases, pas de gates « après coup » vérifiant artefacts/diff/tests, pas de boucles retour explicites. |
| **3. Meta-harness** (sessions isolées, API uniforme, historique, politiques, sandbox) | C'est le cœur même d'Inari : `node.rs` (un processus = des agents + leurs harnais), conduite uniforme par ACP (`claude-agent-acp`, Gemini, Kimi) + OpenFox en natif HTTP/WS (`openfox.rs`), un workdir isolé par thread, historique = capsule, **politiques allow/block/ask = `approvals.rs`** (verdict signé par l'appareil, usage unique, expiration, silence = non) + mode auto par fil + passes de supervision (`passes.rs`) | `node.rs`, `approvals.rs`, `passes.rs`, `docs/node.md` | **Sandbox : inexistant.** Les commandes s'exécutent dans le workdir du fil, sans bulle (bubblewrap) ni périmètre d'écriture forcé — l'invariant « toute modification hors périmètre est annulée » est aujourd'hui impossible. Politiques budget : inexistantes. |
| **4. Harnais + modèles** | OpenFox, le moteur local **dans** le binaire (`engine/` : boucle modèle+outils, serveur compatible OpenAI sur machine ou réseau privé — vLLM, Ollama, llama.cpp ; brain, library, prompts, MCP, panel) + harnais externes Claude Code, Codex, Gemini CLI, Kimi | `engine/mod.rs`, `docs/moteur-local-fr.md`, `enroll.rs` | Couvert — c'est la couche la plus complète. |
| **Trace** | Capsule `ssc/1` (journal chaîné par hachage, endpoint timeline, 6 schémas JSON), bus à sujets durables avec outbox et dédup (`bus.rs`, `buslink.rs`) | `docs/ssc.md`, `capsule/timeline.rs`, `schema/` | Pas de base **SQLite WAL** de trace des phases événementielle lisible en direct (la vision en exige une) ; `brain.db` de Kitsune trace les notes vocales, pas les phases des agents. |

Rôles annexes déjà présents : quotas providers (`usage.rs`), mises à jour des harnais depuis l'interface (`updates.rs`), mémoire partagée des agents (`aimemory.rs`, service MIT), mémoire vocale avec porte MCP (`kitsune/brain.rs`, `mcp.rs`), mascotte/personas (`personas.rs`), sessions navigateur HttpOnly + SameSite=Strict (`session.rs`).

## 5. Points de friction et dette visibles

1. **Pas de sandbox** : l'exécution se fait à nu dans le workdir du fil. Bloquant pour 3 invariants de la vision (périmètre d'écriture, gates sur diff, porte sudo).
2. **Porte sudo absente** : `approvals.rs` couvre l'approbation d'actions d'agent depuis un téléphone (verdict signé), mais il n'existe pas d'élévation privilégiée déterministe montrant la commande exacte, vérifiée par une politique/hook hors LLM.
3. **Interface monolithique** : `web/app.js` 1 431 lignes et `kitsune.js` 1 374 lignes en module-scope, sans bundler ni tests unitaires JS (seuls les tests i18n Rust la couvrent indirectement).
4. **Binaire de prod chargé d'outillage** : demo mesh + agent jouet compilés dans `inari-core-rs`.
5. **Gouvernance documentaire** : 17 docs dont une moitié en français sous exceptions ; le contrat english-check est contourné par nature (docs « pour le propriétaire ») — à assumer ou à résoudre, pas à laisser en tension.
6. **kitsune-voice (Python)** hors du contrat `verify.sh` : deux mondes de vérification.
7. Constats mineurs d'exploitation : 404 favicon permanente ; artefact de composition du renard en Chromium **headless** (capture seulement, non reproduit sur appareil réel) ; stabilité des runs et conflit de configuration MCP « bili » signalés par le propriétaire (hors repo, à traiter séparément).

## 6. Inconnues

1. Contenu et maturité réels des sources Cloudroom de l'infra (`~/Work/cloudroom-core-src`, `cloudfoxroom`, `foxroom`, `cloudroom-gui`, `bb-plugin-provider-openfox`) : non auditées — c'est l'objet de la Phase 1.
2. Omnigent et super-simple-software-factory : non évalués (Phase 1, sur sources primaires citées).
3. Rôle prévu de `services/kitsune-voice` (Python) face au Kitsune Rust : coexistence ou succession — non tranché ici.
4. Le placeholder `<CHEMIN_DU_REPO_INARI>` du prompt de mission n'était pas rempli ; j'ai audité le worktree actif du repo `bengua/inari`. À confirmer.

Inconnues levées après coup (2026-11-02) : le job CI `voice` couvre bien `services/kitsune-voice` (`.github/workflows/ci.yml`) ; aucune mesure de couverture configurée (ni tarpaulin, ni llvm-cov — constat, pas une lacune à combler d'office). Découverte au passage : le repo porte **deux CI** — GitHub (`.github/workflows/ci.yml`, 3 jobs) et la forge interne (`.forgejo/workflows/lint-test.yml`, byte-identique à la flotte, contexte requis `CI / lint-test` sur main) ; la seconde est marquée par le propriétaire comme devant disparaître en fin de run. Gouvernance english-check, état réel : l'outil est **absent des images CI** (le contrat l'y tolère : « SKIP »), donc main porte 36 constats non cliquetés (fixtures FR de tests, majoritairement) qui n'apparaissent qu'en exécution manuelle locale ; le présent livrable est couvert par une ligne dédiée dans `.english-exceptions` (0 constat sur `docs/reorientation/`).

---

**Stop Phase 0.** Aucune écriture hors de ce fichier. En attente de la validation écrite de Ben avant d'ouvrir la Phase 1 (évaluation adopter / fusionner / construire, sur les sources de l'infra).
