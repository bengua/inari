# Journal — Brique 5 : graphe déterministe (fusion cloudfoxroom + modèle sssf)

Délégation écrite du propriétaire : « je me fis à toi, fais bien le job ».
Livrable en français, jamais de delete/recreate.

## Dette visée

Dette « couche 2 » de `docs/reorientation/01-evaluation.md` : entre le
propriétaire et les harnais, rien ne possédait le séquencement. Un enchaînement
plan → build → test → merge vivait dans des prompts et de la bonne volonté :
pas de frontière de phase nommable, pas de critère d'acceptation vérifiable,
une reprise = un redémarrage à froid qui jette tout ce que l'agent venait
d'apprendre, aucune trace requêtable.

## Sources et licences

- **Modèle sssf** (Super Simple Software Factory, IndyDevDan/disler, MIT) :
  adopté, **code non importé** (il pilote Pi ; réécrit contre les harnais
  Inari). Notice dans `THIRD-PARTY.md`. Ce qui est retenu : le code possède
  séquencement, retries et acceptation ; trois lanes ; envelopes JSON typées ;
  gates après coup ; correction **dans la même session**.
- **Forme de trace cloudfoxroom** (code du propriétaire, MIT) : SQLite WAL,
  table d'événements numérotés, lecture par curseur — reprise dans
  `graphe/store.py` sans copie de fichier.

## Ce que fait `services/graphe/`

- **Trois lanes, une primitive** (`runner.py`) : chaque phase est le même
  gestionnaire de contexte. `agent` : un caller répond aux prompts, borné à sa
  `write_scope` ; `engineer` : phase humaine, l'envelope porte le verdict, pas
  de code ; `code` : commande connue, l'envelope est générée par le script.
  **Le succès se mérite** : chaque phase naît `fail`, une sortie propre la
  bascule, une phase agent exige en plus une envelope valide et des gates
  vertes. `run.finish(accepted=…)` tranche l'acceptation du run à part.
- **Envelopes strictes** (`envelope.py` + `schema/envelope.schema.json`,
  contrat §3 de `02-architecture.md`) : version `essaim/1`, champ inconnu =
  rejet, validation pilotée par le fichier de schéma. Le code complète ce
  qu'il possède (ids, lane, scope, curseur) — l'agent propose, le code
  dispose.
- **Gates après coup** (`gates.py`) : `artifacts_exist`, `files_non_empty`,
  `write_scope_respected` (globs `**`, rien n'échappe au workdir). Une gate
  verte dit *ce qu'elle a vérifié*.
- **Correction dans la même session** : envelope invalide ou gate rouge →
  re-prompt du même `session_id` avec une correction nommant exactement le
  défaut, jusqu'à `max_corrections`. Chaque tentative — y compris l'échec —
  est enregistrée (envelope + gate_results), sinon la trace ne montrerait pas
  pourquoi la correction était nécessaire.
- **Roster YAML** (`agents.yaml`, `roster.py`) : un agent = nom, harnais,
  modèle, but, write scope. Les call sites nomment un agent, jamais un
  modèle ; `require()` fait échouer un roster incomplet **avant** que quoi que
  ce soit ne démarre.
- **Trace WAL** (`store.py`) : `sessions`, `phases`, `events`,
  `envelopes`, `gate_results` ; lecture par curseur
  (`events_after(run_id, cursor)`), WAL ouvert à chaque connexion — les
  lectures ne bloquent jamais les écritures.

## Critère d'acceptation : rempli

Le run de démonstration (`graphe.demo`, scripté — aucun modèle requis) exécute
le pipeline §3 : `intention` (engineer) → `build` (agent) → `report` (code) →
`merge` (engineer). Le builder déclare d'abord un artefact **qu'il n'a pas
écrit** — la gate `artifacts_exist` échoue (`ok=0` dans `gate_results`) — puis,
corrigé **dans la même session**, écrit le fichier et répond : tout passe au
vert. La WAL contient les cinq tables, lisibles par curseur
(`test_the_demo_pipeline`).

## Fichiers (liste du plan, respectée)

`services/graphe/` (`graphe/` 6 modules, `tests/` 5 fichiers,
`pyproject.toml`, `agents.yaml`), `schema/envelope.schema.json`,
`.github/workflows/ci.yml` (+ job `graphe` : tests puis `graphe-demo`),
`THIRD-PARTY.md` (notice sssf), ce journal.

## Résultats mesurés

- Tests du service : **31/31 OK** (envelope 7, gates 5, store 6, roster 5,
  runner 5, démo 2) en 17 ms.
- `graphe-demo` : run accepté ; `gate_results` en base :
  `artifacts_exist|0, files_non_empty|0, artifacts_exist|1, files_non_empty|1`
  — l'échec volontaire puis la correction, même session.
- `./verify.sh` : PASSED (Rust non touché par cette brique).
- Écueil corrigé au passage : `ensure_session` émettait son événement **sous**
  le verrou du store (non réentrant) — blocage sec au premier run, attrapé par
  la première exécution des tests, corrigé en émettant hors du verrou.
