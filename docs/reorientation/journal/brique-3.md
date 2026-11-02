# Journal — Brique 3 : bulle d'exécution par session (bubblewrap)

Délégation écrite du propriétaire : « je me fis à toi, fais bien le job ».
Livrable en français, jamais de delete/recreate.

## Dette visée

Brique 3 de `docs/reorientation/03-plan.md` : jusqu'ici un thread d'agent
tournait avec les pleins droits du compte `zordur` sur le cœur — lecture et
écriture possibles partout où l'utilisateur peut écrire. Une instruction
malencontreuse (ou un modèle qui déraille) pouvait toucher n'importe quel
fichier du poste, bien au-delà du workdir du thread.

## Choix d'implémentation

- **bubblewrap (bwrap)** comme frontière : tout le système de fichiers est
  remonté **en lecture seule**, seuls le workdir du thread et des chemins
  supplémentaires explicites (`sandbox_rw` dans la spec du harnais) restent
  inscriptibles. Le réseau reste partagé (le harnais parle HTTP à ses
  backends), les namespaces mount/IPC/UTS sont privés.
- **Le namespace PID reste partagé, `--new-session` retiré.** C'est le
  diagnostic central de la brique : le cœur termine un tour en tuant le
  **groupe de processus** du processus lancé, et les tests (comme
  l'exploitation) lisent `/proc`. Un namespace PID privé fait vivre l'agent
  dans une numérotation propre — l'orphelin écrivait son PID « 3 », qui
  désigne sur l'hôte un thread noyau toujours vivant : évasion du kill de
  groupe **et** impossibilité de vérifier. Le confinement retenu est celui
  du système de fichiers ; le cycle de vie reste celui du groupe de
  processus, comme avant la brique.
- **Chemins absolus résolus via PATH** : le harnais est lancé avec un
  environnement vide (`env_clear`), un `bwrap` nu ne serait pas résolu —
  `sandbox::wrap` résout le binaire sur le PATH du cœur et sème le chemin
  absolu dans l'argv.
- **Refus propre sans bwrap** : un cœur sans bubblewrap ne démarre pas le
  thread et dit pourquoi (`cannot start the agent: sandbox: bwrap not
  found — this node refuses to start the thread until bubblewrap is
  installed`), plutôt que de lancer une session sans frontière. Un
  `eprintln` au démarrage du nœud signale aussi l'absence.
- **Scope systemd-run conservé quand il existe** (bus utilisateur présent) :
  comptabilité cgroup par session, jamais une condition de démarrage.
- `HarnessSpec.sandbox_rw: Vec<String>` (serde `default`) : les configs
  existantes restent valides ; un harnais qui doit écrire ailleurs que dans
  le workdir (le `fake` des tests, qui tient son journal à côté) le déclare.

## Fichiers

- `crates/inari-core/src/sandbox.rs` (nouveau) : `available()`,
  `wrap()`/`wrap_with()`, `scope_prefix()`, `Unavailable`.
- `crates/inari-core/src/agent.rs` : `AcpHarness.sandbox_rw`, `spawn()`
  enveloppé par `sandbox::wrap` (échec → erreur remontée au propriétaire).
- `crates/inari-core/src/node.rs` : `HarnessSpec.sandbox_rw`, câblage,
  avertissement au démarrage si bwrap est absent.
- `crates/inari-core/src/lib.rs` : `pub mod sandbox;`.
- `crates/inari-core/tests/sandbox.rs` (nouveau) : 4 tests.
- `crates/inari-core/tests/agent.rs`, `tests/node.rs` : le `fake` déclare
  `sandbox_rw` vers le répertoire de son journal.
- `docs/security.md` : section « Per-session sandbox ».

## Résultats mesurés

- `tests/sandbox.rs` — **4/4** : écriture hors workdir impossible / dans le
  workdir OK ; `sandbox_rw` inscriptible, reste en lecture seule ; refus
  explicite sans bwrap ; argv conforme (chemins absolus).
- `tests/agent.rs` — **25/25**, dont `whatever_the_agent_started_ends_with_it`
  (l'orphelin du thread meurt avec le tour, vérifié par `/proc`).
- `tests/node.rs` — vert (deux cœurs à travers la gateway, harnais dans la
  bulle).
- `./verify.sh` — **PASSED** (rustfmt, clippy, tests, build).
- Reproduction manuelle : `bwrap` + orphelin `sleep 300` tué par le kill de
  groupe ; lecture de `/proc/<pid>/stat` cohérente depuis l'hôte.
- bubblewrap installé sur mindhub (0.12.0) — laptop et gamer l'avaient déjà.

## Écueil CI : Ubuntu 24.04 et les user namespaces

Premier essai CI : 21 tests `agent` rouges — le runner GitHub (Ubuntu 24.04)
a `kernel.apparmor_restrict_unprivileged_userns=1` et le paquet `bubblewrap`
n'y livre **aucun profil AppArmor** pour `bwrap` : l'écriture de l'uid_map est
refusée (`bwrap: setting up uid map: Permission denied`), l'agent meurt au
lancement. Mesuré sur le runner par une étape de diagnostic (sysctl, profils,
essai direct), retirée ensuite.

Cure, identique en CI et consignée pour les cœurs dans `docs/security.md` :
`sudo chmod u+s /usr/bin/bwrap` — bwrap est conçu setuid et redescend ses
privilèges dans la bulle. Le job `verify` installe bubblewrap puis applique le
setuid ; la bulle testée en CI est donc la bulle réelle, pas un skip.

## Critère réel après fusion : rempli (2026-11-02)

Binaire release construit depuis `654ed34` et déployé sur les trois cœurs
(`~/.local/share/inari-core/bin/inari-core-rs`, ancien binaire conservé en
`.prev-brique3`), service `inari-core` redémarré partout.

- **gamer** (cette machine) : thread `brique3-verif` (harnais openfox) —
  bulle observée en cours de tour :
  `bwrap --die-with-parent --unshare-user --unshare-ipc --unshare-uts
  --unshare-cgroup-try --share-net --ro-bind / / --bind …/work/brique3-verif
  … -- openfox-acp`, scope systemd `inari-session-*` actif.
- **gamer sans bwrap** (nœud lancé avec un PATH sans bwrap) : avertissement
  au démarrage (`…will refuse to start agent threads…`), puis le tour finit
  `stopped:error` avec
  `cannot start the agent: sandbox: bwrap not found — this node refuses to
  start the thread until bubblewrap is installed` — refus propre, jamais de
  repli à nu. Service rendu ensuite.
- **laptop** (omarchy-elitebook, arbre de travail sale — build fait dans un
  worktree détaché `/tmp/inari-main` sur `origin/main`, l'arbre de Ben n'a
  pas été touché) : thread créé, tour `done` sans erreur, bulle bwrap d'une
  session observée sous le nouveau nœud.
- **mindhub** (arbre propre, build local) : thread `brique3-verif`, tour
  `done` sans erreur.
- Threads de vérification archivés sur les trois cœurs, fichiers temporaires
  supprimés.

Note d'exploitation : la création de thread via la gateway exige
`"node":"<node_id>"` dans le payload — sans lui aucun nœud ne répond et la
gateway répond `thread creation timed out` au bout de 30 s.

**Brique 3 : critère d'acceptation entièrement rempli, trois cœurs
conformes.**
