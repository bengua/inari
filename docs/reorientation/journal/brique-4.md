# Journal — Brique 4 : politiques (niveaux, budget, porte sudo)

Délégation écrite du propriétaire : « je me fis à toi, fais bien le job ».
Livrable en français, jamais de delete/recreate.

## Dette visée

Brique 4 de `docs/reorientation/03-plan.md` : l'approbation était un ASK
ponctuel — tout ce qui n'était pas en mode auto partait vers le téléphone,
sans aucune notion de budget ni de traitement particulier pour sudo. Le
constat de la brique 1 (OpenFox laisse passer les commandes bénignes, filtre
sur les dangereuses) demandait un vrai moteur, pas un filtre dispersé.

## Choix d'implémentation

- **`policy.rs`, trois niveaux, le plus strict d'abord** : serveur (config du
  nœud), agent (spec du harnais), session (labels `policy.*` du thread).
  Chaque niveau peut interdire l'auto, accorder les commandes bénignes, et
  fixer des budgets (seuil souple ASK, plafond dur DENY) en tokens et en
  GPU-secondes. Un `Deny` n'importe où bat tout le reste ; un `Ask` bat
  `Allow`.
- **Sémantique des absents, mesurée par les tests** : pour l'auto, un niveau
  non renseigné ne change rien (le mode auto d'hier continue de marcher) ;
  pour les commandes bénignes, un niveau non renseigné n'assouplit rien — il
  faut un `allow_benign: true` explicite à au moins un niveau et aucun
  `false`. Aucune config existante ne voit son comportement changer.
- **Porte sudo** : classée sur la commande seule (`sudo`/`doas`, chemin
  absolu accepté), jamais auto-approuvée, jamais refusée par les budgets —
  toujours un ASK portant la **commande exacte**, dont le verdict signé
  revient au propriétaire (le LLM n'est jamais dans la boucle de décision).
- **Classification** sans LLM : lecture du préfixe d'outil ACP
  (`"bash: …"`), liste blanche de commandes en lecture seule, `git` limité à
  ses sous-commandes de lecture, et **tout métacaractère shell rétrograde**
  la commande en `regular` (`ls; rm -rf x` n'est pas bénigne).
- **Budget réel, pas décoratif** : `usage.rs` gagne un `SpendBook` — les
  tokens s'accumulent à chaque `usage_update` (borne haute documentée : un
  backend qui relit son contexte le retraite à chaque tour) et les
  GPU-secondes mesurent le temps de tour des threads sur modèle local
  (moteur local, ou modèle épinglé sans indice cloud dans le nom). Le moteur
  lit l'instantané à chaque demande de permission.
- **Traçabilité** : chaque décision Ask/Deny est journalisée au format
  `schema/policy-decision.schema.json` (agent, nœud, commande, classe,
  verdict, raison, dépense au moment de la décision).

## Fichiers (liste du plan, respectée)

- `crates/inari-core/src/policy.rs` (nouveau) — moteur.
- `crates/inari-core/src/approvals.rs` — inchangé en fait : le branchement
  des verdicts se fait dans `Runner::approve` (agent.rs), qui appelle déjà
  `NodeApprovals::ask` ; le test d'expiration/usage unique de la brique le
  couvre via `NodeApprovals` directement.
- `crates/inari-core/src/usage.rs` — `SpendBook`.
- `schema/policy-decision.schema.json` (nouveau).
- `crates/inari-core/tests/policy.rs` (nouveau) — 8 tests.
- `crates/inari-core/src/agent.rs`, `node.rs`, `lib.rs` — câblage
  (`policy` dans `NodeConfig` et `HarnessSpec`, serde `default`).

## Résultats mesurés

- `tests/policy.rs` — **8/8** : précédence du plus strict, DENY au plafond,
  ASK au seuil (y compris en auto), ASK sudo avec commande exacte (même en
  auto, même avec budget dépassé), session depuis les labels, enregistrement
  conforme au schéma, verdict signé comptant une fois et expirant.
- Suites existantes intactes : agent 25/25 (auto et approbation comprises),
  sandbox 4/4, node 2/2. `./verify.sh` — **PASSED**.
- Comportement par défaut inchangé : sans aucune clé `policy`, un thread en
  auto approuve, tout le reste demande, rien n'est refusé — prouvé par
  `without_any_limit_the_old_behavior_is_kept`.
