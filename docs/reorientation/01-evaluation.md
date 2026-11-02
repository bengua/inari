# Phase 1 — Évaluation adopter / fusionner / construire

Date : 2026-11-02. Méthode : lecture seule des sources primaires (repos, README, docs officielles, ADR — cités à chaque section), aucune modification de code, aucune installation. Cadrage du propriétaire (2026-11-02, rappelé dans `00-inventaire.md`) : **Inari joue la couche 3 (meta-harness), OpenFox joue la couche 4 (harnais)** ; les références théoriques (Omnigent, Cloudroom core, sssf) sont évaluées comme références, les briques concrètes sont celles de l'infra. Ce qui n'a pas pu être vérifié est marqué **inconnue**.

## Synthèse

| Brique | Licence | Couche(s) visée(s) | Verdict |
|---|---|---|---|
| Omnigent (référence) | Apache 2.0 (alpha) | 3 | **Modèle à copier, runtime à ne pas adopter** |
| cloudroom-core (référence, `~/Work/cloudroom-core-src`) | Apache 2.0 | 3 | **Référence sandbox/lifecycle à transposer dans Inari** |
| sssf (disler, référence) | MIT | 2 | **Modèle à adopter, code à réécrire** (Pi-only) |
| cloudfoxroom (Ben) | MIT | 2 + trace | **Base de fusion pour le graphe déterministe** |
| foxroom (Ben) | Apache 2.0 | méthode | **Doctrine à garder : shims externes, jamais patcher l'upstream** |
| bb-plugin-provider-openfox (Ben) | MIT | 4 | **À garder tel quel** (pont OpenFox↔ACP livré) |
| openfox-acp (Ben, non versionné) | inconnue (aucun repo) | 3↔4 | **À absorber dans un repo — dette critique** |
| OpenFox (tiers, co-l/openfox) | MIT | 4 | **Harnais retenu ; pin de version ou fork à décider** |
| cloudroom-gui → inari-gui (fork Ben) | Apache 2.0 | 1 (référence) | **Inspiration façade ; pas d'adoption** |

---

## 1. Omnigent — référence politiques + sandbox

Sources : `https://github.com/omnigent-ai/omnigent` (README), `docs/POLICIES.md` du même repo.

- **Correspondance couches** : meta-harness (couche 3) au-dessus de Claude Code, Codex, Cursor, OpenCode, Hermes, Pi et agents custom ; UI web + app desktop macOS + collaboration multi-device.
- **Maturité** : statut **alpha** déclaré, publié sur PyPI. Prérequis lourds : Python 3.12+, uv, Node 22 LTS + pnpm, tmux, git.
- **Licence** : Apache 2.0 — permissive, conforme.
- **Modèles locaux** : passerelles LiteLLM, Ollama, vLLM — compatible souveraineté ; providers cloud (modal, e2b, daytona…) **optionnels**, donc non obligatoires.
- **Sudo/HITL** : c'est sa valeur. Politiques déclaratives **ALLOW / DENY / ASK** à des enforcement points, **3 niveaux** (server, agent, session — la plus stricte d'abord) ; builtins `ask_on_os_tools`, `max_tool_calls_per_session`, `cost_budget` (plafond dur USD + seuils soft). Sandbox **bubblewrap obligatoire sur Linux** (refus de démarrer sans bwrap), seatbelt sur macOS.
- **Coût d'intégration avec Inari** : élevé et sans objet — doublon complet avec la couche 3 qu'Inari joue déjà (sessions, API uniforme, historique, approvals signés). Le verdict `approvals.rs` d'Inari couvre ASK signé/usage unique/expiration, mais **pas** les niveaux, **pas** le budget, **pas** le sandbox.
- **Verdict** : ne pas adopter le runtime. **Copier le modèle** : politiques à 3 niveaux + budget + bwrap obligatoire sont la spécification exacte des lacunes d'Inari (sandbox inexistant, budget inexistant — `00-inventaire.md` §4).

## 2. cloudroom-core — référence sandbox et cycle de vie

Sources : `~/Work/cloudroom-core-src` (src/, docs/, ADR), `https://github.com/davidondrej/cloudroom-core`.

- **Correspondance couches** : runtime de sessions par VM + historique (couche 3). Le propriétaire a tranché : **cette couche est jouée par Inari** — cloudroom-core n'est évalué que comme référence technique.
- **Maturité** : v0.1.61, pré-release, Rust (24 796 lignes). Implémenté (preuve : `src/`) : adaptateurs harnais claude/codex/pi/hermes/cursor/devin/opencode (chacun possède son protocole natif), lifecycle complet des sessions (close idempotent par request_id, **sleep après 30 min d'inactivité**, replay après crash borné à ~100k records, teleport, sous-threads sandboxés), confinement Linux (`runtime/linux.rs`), garde de commandes (`session/guard.rs` + `runtime/command-guard.mjs`), arrêt du **cgroup** de workload de la session, secrets séparés, credentials non hérités par les harnais. L'**ADR 0045** tranche le débat du périmètre d'écriture : les agents tournent en **full-access dans leur sandbox** — la bulle isole la session du monde, pas l'agent de sa propre workspace.
- **Licence** : Apache 2.0 — permissive, conforme.
- **Modèles locaux** : via les harnais qu'il conduit ; pas de passerelle propre.
- **Sudo/HITL** : command-guard + containment = la moitié technique de la porte ; pas de politique ASK signée équivalente à `approvals.rs`.
- **Coût d'intégration** : nul en adoption (décision du propriétaire), faible en transposition — les patterns (cgroup de workload, command-guard, sleep/replay, ADR 0045) sont des doctrines directement ré-applicables au futur sandbox d'Inari.
- **Verdict** : **référence à transposer** pour le sandbox et le cycle de vie d'Inari ; ADR 0045 adopté comme doctrine de périmètre d'écriture.

## 3. super-simple-software-factory (disler) — référence graphe déterministe

Source : `https://github.com/disler/super-simple-software-factory` (MIT, IndyDevDan 2026 — 1 commit, skill à tamponner dans un repo).

- **Correspondance couches** : **couche 2 exacte** — un script Python ADW possède séquencement, retries et acceptation (« agent proposes, code disposes »). 3 lanes de phases : engineer (humain), agent, code, avec la même primitive pour les trois.
- **Maturité** : v1 assumée minimaliste (« small enough to read in one sitting ») : **Pi seul** (claude_code stubbé), pas de sandbox, pas de branche par run, **pas de phase d'approbation humaine** — ces limites sont explicitement revendiquées.
- **Licence** : MIT — permissive, conforme.
- **Modèles locaux** : via Pi en v1 ; pour nous : **OpenFox** (cadrage propriétaire).
- **Sudo/HITL** : lane « engineer » = la phase humaine existe comme primitive, mais aucune approbation signée ; Inari apporte déjà ça (`approvals.rs`, `passes.rs`).
- **Ce qui fait sa valeur** : **envelopes JSON typées Pydantic** (status, summary, artifacts[], notes_for_next_agent, changed_files[], commit_message — si ça ne parse pas, on redemande **dans la même session**) ; **gates après coup** sur les déclarations (artifacts_exist, files_non_empty, json_parses, diff_matches_claims, tests_pass — une gate = une fonction) ; **trace SQLite WAL 7 tables** (sessions, phases, events, envelopes, gate_results, agent_sessions, processes) lisible par un seul curseur `rowid > ?`, sans WebSocket ; **roster YAML** d'agents (nom ≠ modèle ; chacun son modèle, son thinking, ses prompts, ses outils, ses droits d'écriture) ; « pas de tester agent : running a suite is a known command and therefore code ».
- **Coût d'intégration** : le code est Pi-only et volontairement jetable — **adopter le modèle, réécrire le code**. Ses limites (sandbox, HITL) sont précisément ce qu'Inari et cloudfoxroom possèdent déjà.
- **Verdict** : **modèle adopté pour la couche 2**, fusionné avec cloudfoxroom (§4) ; OpenFox remplace Pi.

## 4. cloudfoxroom (Ben) — base de la couche 2 concrète

Source : `~/Work/cloudfoxroom` (MIT, Copyright 2026 Zordur — propre au propriétaire).

- **Correspondance couches** : contrôleur de « rooms » durable **au-dessus d'instances OpenFox** — déjà un embryon de couche 2 orienté OpenFox.
- **Maturité** : actif (commits 2026-10-15, api v0.4.0, ~2 689 lignes Python, 9 fichiers de tests). **SQLite WAL** : rooms, dispatches, events, queue items, command receipts (8 tables) ; **idempotence par request_id** ; client par instance avec erreurs isolées ; FoxPaws : le téléphone comme instance, driver « paws » sur llama.cpp local.
- **Licence** : MIT — permissive, conforme, et déjà au propriétaire.
- **Modèles locaux** : natif — pilote OpenFox et llama.cpp local.
- **Sudo/HITL** : command receipts + file d'attente = la moitié « exécution » de la porte ; pas de politique ASK (c'est le rôle d'Inari, couche 3).
- **Coût d'intégration** : faible — c'est la brique la plus proche de la cible : la trace SQLite WAL de la vision y est **déjà écrite**, avec l'idempotence en plus.
- **Verdict** : **base de fusion de la couche 2** — y greffer le modèle sssf (envelopes, gates, roster) ; sa trace SQLite WAL devient la trace des phases.

## 5. foxroom (Ben) — doctrine d'intégration

Source : `~/Work/foxroom` (Apache 2.0).

Fork cosmétique minimal de cloudroom (core+GUI), self-hébergé sur le Tailnet. Doctrine explicite : **ne jamais patcher le core ; toute adaptation (Kimi, OpenFox) par shims externes parlant le protocole harness natif**. foxcore : harnesses locaux zimzilla + sentinelle (commits 2026-10-20/21).

- **Verdict** : pas une brique à fusionner — une **règle de conduite à inscrire dans l'architecture cible** : toute intégration future (modèles, harnais) passe par des shims externes, l'upstream reste pur. C'est aussi ce qui rend la dépendance OpenFox (§8) sûre.

## 6. bb-plugin-provider-openfox (Ben) — pont livré

Source : `~/Work/bb-plugin-provider-openfox` (MIT, Bengua ; 72 lignes + skills).

OpenFox comme **provider d'agent ACP** : les threads tournent sur l'OpenFox de la machine, avec ses modèles, outils et confirmations. Plugin room-cli, CI avec verify.sh étendu. Preuve existante qu'**OpenFox parle ACP comme provider** — réutilisable tel quel pour la couche 4.

- **Verdict** : **garder** ; c'est le patron de câblage OpenFox↔meta-harness déjà livré et testé en CI.

## 7. openfox-acp — dette critique à résorber

Source : `~/.local/bin/openfox-acp` (script Python de 31,9 Ko, relevé le 2026-11-02).

Traduit l'ACP d'Inari (JSON-RPC, une ligne par message sur stdio) vers l'API HTTP + WebSocket d'OpenFox ; **une session OpenFox par thread Inari** ; config par machine dans `~/.config/openfox-acp/` (`url`, `token`) ; mode « builder » par défaut. **Non versionné** : absent de tout repo (inari, cloudfoxroom, swarmfox) ; seul bb-plugin-provider-openfox le référence. Licence : **inconnue** (aucun dépôt, aucun en-tête).

- **Verdict** : maillon critique de la chaîne couche 3 ↔ couche 4 **sans repo, sans tests, sans CI**. Première dette à résorber en phase 3 : l'absorber dans le repo inari (ou un crate dédié), avec licence MIT déclarée et tests.

## 8. OpenFox — harnais retenu (couche 4)

Sources : paquet npm `openfox` v2.0.161 installé (`~/.local/lib/node_modules/openfox`, README + package.json) ; repo `github.com/co-l/openfox` (tiers).

- **Correspondance couches** : couche 4 (cadrage propriétaire). « Local-LLM-first agentic coding assistant » : détection auto vLLM/sglang/ollama/llamacpp ; v2.0 = moteur unifié avec **EventStore** comme source de vérité (modes builder/planner/verifier/sous-agents/compaction), workflows contract-driven, retry patterns configurables, fallback vision.
- **Maturité** : v2.0.161 en exploitation réelle sur l'infra (process actif depuis le 2026-10-24, Tailnet `:10369`).
- **Licence** : MIT — permissive, conforme.
- **Modèles locaux** : c'est sa raison d'être (local-LLM-first).
- **Sudo/HITL** : confirmations natives ; l'élévation reste du ressort de la couche 3 (Inari).
- **Coût d'intégration** : déjà intégré (moteur local **dans** le binaire Inari, `engine/` + conduite native HTTP/WS `openfox.rs` + pont ACP §7). **Dépendance à un repo tiers** : pin de version ou fork à trancher en phase 2 — la doctrine foxroom (§5) s'applique.
- **Verdict** : **retenu comme harnais de la couche 4**.

## 9. cloudroom-gui / inari-gui — inspiration façade

Sources : `~/Work/cloudroom-gui` (Apache 2.0, NOTICE BB/MIT), fork miroir `github.com/bengua/inari-gui`.

Application desktop (BB) pour Claude Code/Codex/Pi : teleport local↔cloud, sandbox et branche Git par thread cloud, **accès gradué à la machine locale : Off / Read-only / Ask first / Full**, computer use, multi-comptes. Ce modèle d'**accès gradué** est une forme de politique HITL par ressource, complémentaire des verdicts ASK ponctuels d'Omnigent et d'Inari.

- **Verdict** : **inspiration pour la couche 1** (façade) et pour la définition des politiques (phase 2) ; pas d'adoption — la façade d'Inari (web + Kitsune + Android) existe et couvre déjà plus de surfaces.

---

## Recommandation de fusion

| Couche cible | Brique retenue | Action |
|---|---|---|
| **1. Façade** | Interface web + Kitsune + Android **d'Inari** | Garder ; ajouter (phase 2+) le déclencheur « intention → workflow » et l'accès gradué inspiré de cloudroom-gui ; petit modèle de façade type LFM3 : à construire, pas avant la phase 2 |
| **2. Graphe déterministe** | **cloudfoxroom + modèle sssf** | Fusionner : envelopes JSON typées, gates après coup, roster YAML, boucles retour — greffés sur la base SQLite WAL et l'idempotence déjà écrites ; OpenFox remplace Pi |
| **3. Meta-harness** | **Inari** (décision du propriétaire) | Garder ; transposer depuis Omnigent (politiques 3 niveaux, budget, bwrap obligatoire) et cloudroom-core (cgroup de workload, command-guard, sleep/replay, ADR 0045) ; **construire le sandbox — la lacune n°1** |
| **4. Harnais + modèles** | **OpenFox** via bb-plugin-provider-openfox + openfox-acp | Garder ; **versionner openfox-acp (dette critique)** ; pin ou fork du paquet npm tiers |
| **Trace** | SQLite WAL de cloudfoxroom, étendue aux phases | Fusionner avec la capsule ssc/1 (preuve portable) plutôt que la remplacer |

**Ce qu'Inari garde en propre (valeur différenciante, aucune brique évaluée ne l'a)** : la capsule `ssc/1` portable et chaînée par hachage, la téléportation 2PC entre machines, les passes familiaux de supervision, les approvals signés par l'appareil, le mesh Tailnet sans compte, Kitsune (voix + mémoire), l'app Android. C'est le socle sur lequel tout se greffe.

**Ce qui est abandonné** : Omnigent comme runtime (alpha, doublon de la couche 3, prérequis lourds) ; cloudroom-core comme couche 3 (tranché : c'est Inari) ; sssf comme code (Pi-only, sans sandbox ni HITL — modèle seulement) ; toute dépendance obligatoire à un provider cloud.

**Dettes et risques à traiter en phase 2/3** : ① openfox-acp non versionné (critique) ; ② sandbox Inari à construire (bloque 3 invariants de la vision : périmètre d'écriture, gates sur diff, porte sudo) ; ③ dépendance npm `openfox` tiers à pinner ou forker ; ④ politiques budget et niveaux à ajouter à `approvals.rs`.

**Conformité mission** : toutes les briques retenues sont MIT ou Apache 2.0 (permissives) ; la seule licence **inconnue** est celle d'openfox-acp, faute de dépôt — à déclarer MIT par le propriétaire lors de son absorption. Aucune dépendance cloud obligatoire introduite.

---

**Stop Phase 1.** Aucune écriture hors de ce fichier. En attente de la validation écrite de Ben avant d'ouvrir la Phase 2 (`02-architecture.md` + `03-plan.md`).
