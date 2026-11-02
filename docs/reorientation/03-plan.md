# Phase 2 — Plan de migration en briques livrables indépendantes

Date : 2026-11-02. Compagnon de `02-architecture.md`. Ordre strict : une brique à la fois, sur une branche dédiée, PR vers la forge ; mesures et résultats **produits par script, jamais écrits à la main** ; journal `docs/reorientation/journal/<brique>.md` (commandes exactes, sorties, erreurs — ne rien effacer). Chaque brique est réversible seule.

Vue d'ensemble :

| # | Brique | Dépend de | Couche |
|---|---|---|---|
| 1 | PoC jetable : meta-harness + OpenFox + modèle local + porte sudo | — | 3+4 |
| 2 | Absorption d'openfox-acp (dette critique) | — | 3↔4 |
| 3 | Sandbox bwrap + cgroup par session | 1 | 3 |
| 4 | Politiques : niveaux, budget, porte sudo dans Inari | 3 | 3 |
| 5 | Couche 2 : graphe déterministe (fusion cloudfoxroom + sssf) | 2, 4 | 2 |
| 6 | Façade : déclencheur intention → workflow + rapports | 5 | 1 |
| 7 | Nettoyage : binaire prod allégé, CI forge retirée, docs | 3–6 | transverse |

---

## Brique 1 — Preuve de concept sur environnement jetable

- **Objectif** : prouver la chaîne complète minimale — le meta-harness (Inari) conduit **un seul harnais (OpenFox)** avec **un modèle local** dans un **conteneur jetable**, avec **une politique sudo testée** (demande affichant la commande exacte, verdict signé, refus effectif).
- **Cadre** : conteneur Docker jetable sur gamer (décision recommandée, cf. §5 de `02-architecture.md`) ; bwrap à l'intérieur du conteneur ; aucun service de production n'est touché.
- **Fichiers touchés (liste fermée)** :
  - `poc/brique-1/Dockerfile` (nouveau)
  - `poc/brique-1/policy.yaml` (nouveau — une politique : sudo = ASK signé)
  - `poc/brique-1/run.sh` (nouveau — build, lance, scénarise)
  - `poc/brique-1/verify.sh` (nouveau — critère d'acceptation)
  - `docs/reorientation/journal/brique-1.md` (nouveau)
- **Critère d'acceptation (scripté, `verify.sh` exit 0)** : ① le conteneur démarre, inari-core et OpenFox s'y parlent ; ② un thread exécute une tâche triviale avec le modèle local ; ③ une commande privilégiée déclenche l'ASK avec la **commande exacte** affichée, le refus la bloque, l'approbation signée la laisse passer ; ④ la trace contient les événements (phase, gate, verdict) ; ⑤ le tout tourne deux fois de suite avec le même résultat.
- **Retour arrière** : `docker rm` du conteneur + suppression de `poc/brique-1/` — rien d'autre n'existe.

## Brique 2 — Absorption d'openfox-acp (dette critique)

- **Objectif** : sortir le pont ACP↔OpenFox de `~/.local/bin` (non versionné, sans tests, sans CI) pour en faire un service du repo, licence MIT déclarée par le propriétaire.
- **Fichiers touchés (liste fermée)** :
  - `services/openfox-acp/` (nouveau : script déplacé, `pyproject.toml`, `README.md`, tests unitaires du traducteur JSON-RPC ↔ HTTP/WS)
  - `.github/workflows/ci.yml` (+ job `openfox-acp`, copie du patron `voice`)
  - `THIRD-PARTY.md` (notice)
  - `docs/reorientation/journal/brique-2.md` (nouveau)
- **Critère d'acceptation** : `python -m unittest` vert dans `services/openfox-acp/` ; job CI vert ; le binaire déployé sur les 3 cœurs provient du repo (hash comparable) ; un thread Inari↔OpenFox réel passe sur gamer.
- **Retour arrière** : revert de la PR ; l'ancien script reste en place sur les cœurs jusqu'à bascule explicite (jamais de delete/recreate).

## Brique 3 — Sandbox bwrap + cgroup par session

- **Objectif** : toute session d'agent s'exécute dans une bulle (workdir en écriture, reste en lecture seule) avec un cgroup de workload dédié ; un cœur sans bwrap **refuse de démarrer le thread** et le dit (modèle Omnigent).
- **Fichiers touchés (liste fermée)** :
  - `crates/inari-core/src/sandbox.rs` (nouveau)
  - `crates/inari-core/src/node.rs` (spawn dans la bulle)
  - `crates/inari-core/tests/sandbox.rs` (nouveau)
  - `docs/security.md` (mise à jour du modèle)
  - `docs/reorientation/journal/brique-3.md` (nouveau)
  - hors repo : installation de `bubblewrap` sur mindhub (opération déclarée au journal)
- **Critère d'acceptation** : `tests/sandbox.rs` prouve qu'une écriture hors workdir est impossible (errno) et qu'une écriture dedans passe ; `./verify.sh` PASSED ; un thread réel tourne dans sa bulle sur gamer et laptop ; le démarrage sur un cœur sans bwrap échoue **proprement** (message explicite, pas de crash).
- **Retour arrière** : revert de la PR — les sessions repartent à nu comme aujourd'hui ; le cgroup disparaît avec la session.

## Brique 4 — Politiques : niveaux, budget, porte sudo

- **Objectif** : `approvals.rs` passe du ASK ponctuel au modèle complet — 3 niveaux (serveur, agent, session), budget (plafond dur DENY, seuils soft ASK, en tokens/GPU-temps pour les modèles locaux), porte sudo (commande exacte, verdict hors LLM).
- **Fichiers touchés (liste fermée)** :
  - `crates/inari-core/src/policy.rs` (nouveau — moteur de politiques, la plus stricte d'abord)
  - `crates/inari-core/src/approvals.rs` (branchement des verdicts)
  - `crates/inari-core/src/usage.rs` (compteurs → estimation budget)
  - `schema/policy-decision.schema.json` (nouveau)
  - `crates/inari-core/tests/policy.rs` (nouveau)
  - `docs/reorientation/journal/brique-4.md` (nouveau)
- **Critère d'acceptation** : `tests/policy.rs` couvre : précédence du niveau le plus strict, DENY au plafond, ASK au seuil, ASK sudo avec commande exacte, expiration et usage unique du verdict ; `./verify.sh` PASSED.
- **Retour arrière** : revert de la PR ; `approvals.rs` retrouve son comportement actuel (rien d'autre ne dépend encore de `policy.rs`).

## Brique 5 — Couche 2 : graphe déterministe (fusion cloudfoxroom + modèle sssf)

- **Objectif** : le graphe possède séquencement, retries, gates, phases humaines ; envelopes JSON typées validées par schéma ; trace SQLite WAL des phases (héritée des tables cloudfoxroom, étendue : phases, envelopes, gate_results) ; roster YAML d'agents (nom ≠ modèle).
- **Fichiers touchés (liste fermée)** :
  - `services/graphe/` (nouveau : code fusionné depuis cloudfoxroom — MIT propriétaire — + modèle sssf ; `pyproject.toml`, roster `agents.yaml`, tests)
  - `schema/envelope.schema.json` (nouveau)
  - `.github/workflows/ci.yml` (+ job `graphe`)
  - `THIRD-PARTY.md` (notice sssf MIT — modèle copié, attribution)
  - `docs/reorientation/journal/brique-5.md` (nouveau)
- **Critère d'acceptation** : un run de démonstration exécute le pipeline §3 de `02-architecture.md` avec au moins une gate qui **échoue volontairement** puis réussit après correction **dans la même session** ; la WAL contient sessions, phases, events, envelopes, gate_results lisibles par curseur ; tests du service verts ; job CI vert.
- **Retour arrière** : revert de la PR ; cloudfoxroom externe reste en l'état (rien ne le supprime) ; la base WAL du service est un fichier neuf — suppression simple.

## Brique 6 — Façade : intention → workflow + rapports

- **Objectif** : depuis l'interface (web, puis Android) et la voix (Kitsune), déclencher un workflow du graphe et suivre ses phases ; la façade relit la trace WAL pour produire des rapports en langage humain ; accès machine gradué (Off / Read-only / Ask first / Full) déclaré à la session.
- **Fichiers touchés (liste fermée)** :
  - `crates/inari-core/src/gateway.rs` (+ `/api/workflows/*`)
  - `crates/inari-core/web/app.js`, `crates/inari-core/web/i18n.js`, `crates/inari-core/web/app.css` (onglet Workflows)
  - `crates/inari-core/tests/gateway.rs` ou nouveau `tests/workflows_api.rs`
  - `docs/reorientation/journal/brique-6.md` (nouveau)
- **Critère d'acceptation** : un workflow déclenché depuis l'onglet se voit phase par phase (lecture WAL par curseur, pas de WebSocket) ; le rapport relu correspond aux événements tracés ; tests i18n verts (les 4 garde-fous existants) ; `./verify.sh` PASSED.
- **Retour arrière** : revert de la PR ; l'interface perd l'onglet, le reste est inchangé.

## Brique 7 — Nettoyage de fin de migration

- **Objectif** : le binaire de production ne porte plus l'outillage démo ; la CI forge interne disparaît (décision du propriétaire) ; la documentation est alignée.
- **Fichiers touchés (liste fermée)** :
  - `crates/inari-core/src/demo.rs`, `demo_agent.rs` (déplacés hors binaire prod : feature cargo ou binaire outil)
  - `.forgejo/workflows/lint-test.yml` (supprimé — en dernier, quand tout est vert ailleurs)
  - `docs/architecture.md`, `README.md` (alignement sur l'architecture cible)
  - `docs/reorientation/journal/brique-7.md` (nouveau)
- **Critère d'acceptation** : `./verify.sh` PASSED ; le binaire prod ne contient plus les symboles demo (vérifié par script) ; CI GitHub seule et verte sur main.
- **Retour arrière** : revert de la PR — la dernière étape (CI forge) n'est jouée que sur validation explicite séparée.

---

## Règles d'exécution (rappel de la mission)

- Une brique = une branche = une PR ; validation écrite du propriétaire **par brique** avant implémentation.
- Mesures par script uniquement ; journal par brique, rien n'est effacé.
- Jamais de delete/recreate ; ce qui marche est conservé ; licences permissives uniquement pour tout composant adopté.
- Conditions d'arrêt immédiat inchangées (droits élevés non validés, licence non permissive, contradiction structurante, information introuvable).

**Stop Phase 2.** En attente de la validation écrite de la **brique 1** (et, si possible, des 3 décisions du §5 de `02-architecture.md`) pour ouvrir la phase 3.
