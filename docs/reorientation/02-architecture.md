# Phase 2 — Architecture cible appliquée à Inari

Date : 2026-11-02. Base : `00-inventaire.md` (existant prouvé), `01-evaluation.md` (choix de fusion validés par le « go phase 2 » du propriétaire). Aucune ligne de code n'est écrite en phase 2 : ce document et `03-plan.md` sont les seuls livrables.

## 1. Schéma des 4 couches appliqué à Inari

```
┌────────────────────────────────────────────────────────────────────┐
│ COUCHE 1 — FAÇADE (ne décide jamais : transmet et rapporte)        │
│  web app.js/kitsune.js · app Android · voix Kitsune                │
│  + à construire : déclencheur intention → workflow, rapports       │
│    relus depuis la trace, petit modèle de façade (type LFM3)       │
├────────────────────────────────────────────────────────────────────┤
│ COUCHE 2 — GRAPHE DÉTERMINISTE (le seul chef d'orchestre)          │
│  services/graphe/ (Python — fusion cloudfoxroom + modèle sssf)     │
│  phases · retries · gates · phases humaines · envelopes JSON       │
│  trace SQLite WAL (hérite des 8 tables cloudfoxroom + phases)      │
├────────────────────────────────────────────────────────────────────┤
│ COUCHE 3 — META-HARNESS : INARI (applique les politiques)          │
│  node.rs · openfox.rs · ACP (claude/codex/gemini/kimi)             │
│  approvals.rs étendu (ASK signé, niveaux, budget, porte sudo)      │
│  À CONSTRUIRE : sandbox bwrap + cgroup par session (lacune n°1)    │
├────────────────────────────────────────────────────────────────────┤
│ COUCHE 4 — HARNAIS + MODÈLES (travail borné dans une phase)        │
│  OpenFox (moteur local engine/ + service :10369) via openfox-acp   │
│  harnais externes ACP · modèles locaux vLLM/Ollama/llama.cpp       │
└────────────────────────────────────────────────────────────────────┘
   Trace transverse : capsule ssc/1 (preuve portable chaînée par
   hachage) + SQLite WAL des phases (lecture directe par façade et UI)
```

Doctrines adoptées (phase 1) : **ADR 0045** — full-access *dans* la bulle, la bulle isole la session du monde ; **doctrine foxroom** — jamais patcher l'upstream, adaptations par shims externes ; **« agent proposes, code disposes »** — aucun agent n'exécute commit, push ni commande privilégiée.

## 2. Positionnement de l'existant (gardé / adapté / remplacé / supprimé)

| Composant | Verdict | Rôle dans la cible |
|---|---|---|
| `capsule/` (ssc/1, timeline, journal chaîné) | **gardé** | preuve portable de la trace ; complète la WAL, ne lui est pas substituée |
| `mesh.rs`, `bus.rs`, `buslink.rs` | **gardé** | transport Tailnet entre cœurs, bus à sujets durables |
| `teleport.rs` | **gardé** | téléportation 2PC — valeur différenciante |
| `passes.rs` | **gardé** | passes familiaux de supervision (HITL social déjà livré) |
| `session.rs`, `signature.rs`, `enroll.rs` | **gardé** | sessions HttpOnly, verdicts signés par l'appareil, enrôlement |
| `openfox.rs` | **gardé** | conduite native OpenFox HTTP/WS |
| `engine/` (dont `workflow.rs`) | **gardé** | moteur local et machine à états **au niveau thread** — le graphe d'essaim se place au-dessus, ne le remplace pas |
| `kitsune/` (crate) + `services/kitsune-voice/` | **gardé** | façade vocale ; la coexistence Rust/Python reste à trancher (inconnue n°3 de l'inventaire, hors scope phase 2) |
| `android/` | **gardé** | façade mobile — surface d'approbation et de rapport |
| `aimemory.rs`, `personas.rs`, `livekit.rs`, `updates.rs` | **gardé** | mémoire partagée, mascotte, voix temps réel, mises à jour des harnais |
| `gateway.rs` | **adapté** | + endpoints `/api/workflows/*` (déclenchement, état, rapport) et lecture de la trace WAL des phases |
| `approvals.rs` | **adapté** | + politiques à **3 niveaux** (serveur, agent, session — la plus stricte d'abord), + **budget** (plafond dur + seuils soft), + **porte sudo** (commande exacte affichée, verdict hors LLM) |
| `usage.rs` | **adapté** | compteurs tokens → estimation de coût alimentant la politique budget |
| `node.rs` | **adapté** | spawn des sessions **dans le sandbox** (bwrap + cgroup de workload) ; drapeau de capacité par cœur (mindhub sans bwrap aujourd'hui) |
| `web/app.js`, `web/i18n.js` | **adapté** | onglet Workflows (état des phases, gates, verdicts), lecture de la trace |
| `schema/` | **étendu** | + `envelope.schema.json`, + schémas de politiques |
| `demo.rs`, `demo_agent.rs` | **supprimé du binaire de production** | déplacés vers un binaire d'outillage ou une feature cargo — la prod ne doit pas porter le mesh jouet |
| `.forgejo/workflows/lint-test.yml` | **supprimé en fin de migration** | décision du propriétaire : la forge interne disparaît quand le run est fini ; GitHub CI reste la seule |
| `openfox-acp` (script non versionné) | **absorbé** | → `services/openfox-acp/` dans ce repo, licence MIT déclarée, tests, CI (dette critique n°1) |
| `cloudfoxroom` (repo externe, MIT Ben) | **fusionné** | → `services/graphe/` : ses tables SQLite WAL, son idempotence request_id et ses rooms deviennent la couche 2 |
| Omnigent, cloudroom-core, sssf, cloudroom-gui | **références, jamais des dépendances** | modèles copiés, code non importé |

**Rien d'autre n'est supprimé** : la règle « ce qui marche est conservé » s'applique à tout le reste.

## 3. Format des envelopes JSON (contrat de phase)

Une envelope par phase terminée, écrite par l'agent comme **donnée** (jamais parsée comme instruction), validée par le code du graphe avant d'être injectée dans la phase suivante. Schéma déclaré dans `schema/envelope.schema.json` (brique 5), sérialisation stricte (champ inconnu = rejet).

```json
{
  "envelope": "essaim/1",
  "run_id": "01J…ULID",
  "phase_id": "01J…ULID",
  "lane": "agent | engineer | code",
  "agent": "builder",
  "status": "done | needs_retry | blocked | failed",
  "summary": "texte court, humain",
  "artifacts": [
    { "path": "src/foo.rs", "sha256": "…", "kind": "file | diff | report" }
  ],
  "changed_files": ["src/foo.rs"],
  "write_scope": ["crates/inari-core/src/**"],
  "notes_for_next_agent": "contexte utile, texte libre borné",
  "commit_message": "proposé par l'agent, appliqué par le code seul",
  "gate_results": [
    { "gate": "artifacts_exist", "ok": true, "detail": "" }
  ],
  "trace_cursor": 1234
}
```

Règles :

- **Lanes** (modèle sssf) : `agent` (un harnais borne son travail à sa `write_scope`), `engineer` (phase humaine : test UX, verdict sudo, merge — l'envelope porte le verdict signé, pas du code), `code` (commande connue : tests, lint, build, CI — aucune sortie libre, l'envelope est générée par le script).
- **Envelope invalide** (JSON qui ne parse pas, schéma violé) → redemande **dans la même session** ; jamais de redémarrage à froid.
- **Gates après coup** (une gate = une fonction, exécutées par le graphe, pas par l'agent) : `artifacts_exist`, `files_non_empty`, `json_parses`, `diff_matches_claims` (le diff réel ⊆ `changed_files` ⊆ `write_scope`), `tests_pass(...)` quand la lane est `code`.
- **Boucles retour explicites** : chaque échec possible (gate, test UX humain, CI) nomme la phase de retour dans la définition du graphe — jamais d'impasse.

Phases du pipeline métier de référence (exemple de la mission, adapté aux harnais d'Inari) :

| # | Phase | Lane | Exécuteur | Sortie attendue |
|---|---|---|---|---|
| 1 | Étude (style BMAD) | agent | harnais sur Qwen local | `report` + `notes_for_next_agent` |
| 2 | Faisabilité + proto léger | agent | harnais sur Kimi local | `artifacts` proto + `report` |
| 3 | **Test UX/UI/fonction** | engineer | humain (verdict signé) | verdict + remarques |
| 4 | Review, backend, sécurité | agent | harnais sur Claude | `diff` borné à `write_scope` |
| 5 | CI : tests, lint, scan | code | scripts du repo | verdict binaire du code |
| 6 | **Validation sudo/merge** | engineer | humain (commande exacte affichée) | verdict signé |
| 7 | commit | code | le graphe, jamais l'agent | hash de commit dans la trace |

## 4. Politiques : définition et points d'application

Toutes les politiques vivent dans la **couche 3 (Inari)** — le graphe (couche 2) séquence, il n'accorde rien ; la façade (couche 1) transmet, elle ne peut accorder un droit. Modèle à 3 niveaux (serveur < agent < session, la plus stricte d'abord) copié d'Omnigent, verdicts ASK signés par l'appareil (mécanisme `approvals.rs` existant : usage unique, expiration, silence = non).

| Politique | Point d'application | Mécanisme |
|---|---|---|
| **Porte sudo / élévation** | wrapper d'exécution du sandbox (couche 3), avant tout exec privilégié | command-guard intercepte, affiche la **commande exacte**, exige un verdict signé ; la vérification est du code (hook/politique), **jamais un modèle** ; la façade ne peut pas accorder |
| **push / commit** | fin de phase `code` + phase humaine 6 | l'agent propose `commit_message` dans son envelope ; seul le graphe applique, après CI verte **et** verdict humain signé |
| **budget** | comptage par événement dans la trace, `usage.rs` étendu | plafond dur (DENY) + seuils soft (ASK) en équivalent coût, à 3 niveaux ; les modèles locaux comptent en tokens/GPU-temps, pas en USD |
| **écriture hors périmètre** | ① préventif : montages bwrap (tout hors workdir en lecture seule) ② après coup : gate `diff_matches_claims` | toute modification hors `write_scope` est annulée et la phase échoue — doctrine ADR 0045 : full-access *dans* la bulle |
| **accès machine gradué** (inspiration cloudroom-gui) | déclaration de session | Off / Read-only / Ask first / Full par ressource, hérité par les harnais de la session |

**Sandbox (la lacune n°1, brique 3)** : bulle `bwrap` par session (obligatoire pour démarrer un thread, modèle Omnigent) + cgroup de workload par session (pattern cloudroom-core, permet l'arrêt propre et les limites) ; sleep après inactivité et replay après crash calqués sur le lifecycle cloudroom-core. Capacité relevée : bwrap 0.13 sur gamer et laptop, **absent de mindhub** (à installer, ou cœur marqué non éligible), cgroup v2 partout.

## 5. Décisions laissées au propriétaire (à trancher à la validation du plan)

1. **Brique 1 (PoC)** : la mission exige une « VM jetable » ; l'infra offre Docker 29.9 et systemd-nspawn sur gamer (KVM présent mais qemu non installé, disque à 95 %). Recommandation : **conteneur jetable Docker** + bwrap à l'intérieur — souverain, immédiat, suffisant pour meta-harness + un harnais + politique sudo. Une vraie VM exigerait d'installer qemu (espace serré).
2. **Dépendance npm `openfox`** (tiers, `co-l/openfox`) : pin de version (léger) ou fork sous `bengua/` (souveraineté maximale, coût de maintenance).
3. **mindhub** : installer bubblewrap (recommandé, un paquet) ou le déclarer cœur non éligible au sandbox.

---

**Stop Phase 2 (architecture).** Le plan en briques est dans `03-plan.md`. Aucune implémentation sans validation écrite de la brique concernée.
