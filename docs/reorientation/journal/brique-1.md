# Journal — Brique 1 : PoC meta-harness + OpenFox + modèle local + porte sudo

Ouvert le 2026-11-02. Branche `reorientation/brique-1`. Règle du journal : commandes exactes, sorties, erreurs — ne rien effacer.

## Décisions appliquées (délégation écrite du propriétaire : « je me fis à toi »)

1. PoC en **conteneur Docker jetable** sur gamer (recommandation du §5 de `02-architecture.md`), réseau hôte pour joindre l'OpenFox local.
2. openfox npm : **pin de version** (v2.0.161 installée) — fork différé, doctrine shims inchangée.
3. mindhub : **bubblewrap à installer** (opération reportée à la brique 3, le PoC ne tourne que sur gamer).

## Étapes et résultats

### 1. Livrables phases 0-2 sécurisés (prérequis)

```
git checkout -b reorientation/livrables && git add .english-exceptions docs/reorientation/ && git commit
git push -u origin reorientation/livrables
gh pr create --head reorientation/livrables --base main   # PR #45
```

PR #45 : CI 3/3 SUCCESS (verify, android, voice) → fusion squash `e9b30e4` le 2026-11-02T00:10:35Z, branche distante supprimée.

### 2. Fichiers du PoC

`poc/brique-1/` : `Dockerfile`, `policy.yaml`, `run.sh`, `verify.sh` (liste fermée du `03-plan.md`). Branche `reorientation/brique-1` créée depuis `origin/main` (`6cf4974`), rebasée sur `e9b30e4` après fusion de la PR #45.

### 3. Premier build — ÉCHEC (consigné, rien effacé)

```
poc/brique-1/run.sh
```

Échec à l'étape `cargo build --release -p inari-core` (image `rust:1.98-alpine`) :

```
error: failed to run custom build command for `sqlite-vec v0.1.9`
sqlite-vec.c:68:9: error: unknown type name 'u_int8_t'; did you mean 'uint8_t'?
sqlite-vec.c:69:9: error: unknown type name 'u_int16_t'
sqlite-vec.c:70:9: error: unknown type name 'u_int64_t'
```

Cause : `sqlite-vec.c` emploie les types BSD `u_int*_t`, que les en-têtes **musl** (Alpine) n'exposent pas par défaut — glibc les expose, c'est pourquoi le build hôte passe.

Correctif appliqué au `Dockerfile` (étape build) :

```
RUN CFLAGS="-D_GNU_SOURCE -include sys/types.h" cargo build --release -p inari-core
```

### 4. Second build — RÉUSSI

```
poc/brique-1/run.sh   # relancé avec le correctif CFLAGS
```

Sortie (extraits) :

```
#12 110.9     Finished `release` profile [optimized] target(s) in 1m 50s
#14 naming to docker.io/library/inari-poc-brique-1:local done
== conteneur : réseau hôte (joindre l'OpenFox local), pont et config montés en lecture seule ==
== attente de la gateway sur 127.0.0.1:17487 ==
gateway OK (tentative 1)
preuve de capacité sandbox dans le conteneur :
bubblewrap 0.12.0
```

Compilation musl complète en 1 min 50 s ; la gateway du conteneur répond dès la première tentative ; **bwrap fonctionne dans le conteneur** (0.12.0) — la capacité sandbox de la brique 3 est prouvée au passage.

### 5. Vérification scriptée — première tentative : ÉCHEC (consigné)

```
poc/brique-1/verify.sh
```

Le scénario restait muet à l'étape 3 (attente de la demande d'approbation). Sonde manuelle :

```
curl -X POST … -d '{"thread_id":"poc-sonde","title":"sonde","harness":"openfox"}' http://127.0.0.1:17487/api/agent
→ {"error":"thread creation timed out"}   (504 après 30 s)
```

Cause (lecture du code, `crates/inari-core/src/node.rs:1273`) : le gestionnaire `agent.create` n'écoute que les demandes dont le champ `node` vaut exactement le `node_id` du cœur ; sans ce champ, personne ne répond et la gateway échoue sur timeout. Correctif dans `verify.sh` : ajout de `"node":"poc-brique-1"` à l'appel `/api/agent`. Vérification immédiate :

```
curl -X POST … -d '{"thread_id":"poc-brique-1-thread","title":"PoC brique 1","harness":"openfox","node":"poc-brique-1"}' …/api/agent
→ {"ok":true,"thread_id":"poc-brique-1-thread"}
```

### 6. Vérification scriptée — seconde tentative : en cours

```
poc/brique-1/verify.sh   # avec le champ node
```

Constat d'exécution pendant l'attente de la première demande d'approbation : le tour LLM s'exécute réellement — GPU 1 (4070 Ti S, instance `Q3.8-RVN-27BIQ3XS-llamacpp-c1`) à 98 % d'utilisation ; GPU 0 (5080, heretic `-c0`) chargé mais inactif. Le modèle par défaut d'OpenFox a résolu sur l'instance `-c1`. Levier de vitesse pour plus tard : split `-x2` (les deux cartes), déjà prévu dans les labels du `node.json` de production.

Résultat : **ÉCHEC** — `aucune demande d'approbation pour 'poc-refuse-1.txt' en 280 s`.

Diagnostic dans la trace du thread elle-même (`/data/host/agents/poc-brique-1-thread/memory/timeline_events.jsonl`, événement 3) :

```
{"kind":"checkpoint","data":{"reason":"harness_error",
 "detail":"POST /api/projects: HTTP 403 Permission denied: cannot create directory at /data/host/work/poc-brique-1-thread"}}
```

Cause racine : le workspace du thread est annoncé à l'OpenFox local (qui tourne **hors** conteneur) avec un chemin **interne au conteneur** (`/data/host/work/…`), inexistant et inscriptible pour lui → 403 avant tout appel d'outil, donc jamais d'ASK. (L'activité GPU à 98 % observée pendant l'attente était un autre flux de l'infra, pas ce tour.)

Correctif (`run.sh` + `verify.sh`) : le répertoire de données est monté **au même chemin absolu** des deux côtés (`-v "$RUN/data:$RUN/data"`, `data_dir` absolu) ; le nettoyage gère les fichiers root laissés par le conteneur.

### 7. Cycle complet — troisième tentative : en cours

```
poc/brique-1/run.sh && poc/brique-1/verify.sh
```

Résultat : **ÉCHEC** — une étape plus loin. Le workspace est bien créé au chemin partagé, mais :

```
{"reason":"harness_error","detail":"POST /api/projects: HTTP 403 Failed to initialize git: Command failed: git init …"}
```

Cause racine : le conteneur écrivait en `root` (`/tmp/inari-poc-brique-1/data/host/*` en `root:root`) alors que l'OpenFox local tourne en `zordur` sur l'hôte → `git init` refuse d'écrire. Correctif (`run.sh`) : conteneur lancé avec `--user "$(id -u):$(id -g)"`, `HOME=/tmp`, `OPENFOX_ACP_DIR=/ofac` (config montée en lecture seule).

### 8. Cycle complet — quatrième tentative : en cours

```
poc/brique-1/run.sh && poc/brique-1/verify.sh
```

Résultat : **ÉCHEC** — encore une couche plus loin. Le workspace et `git init` passent, mais :

```
{"reason":"harness_error","detail":"GET /api/projects: OpenFox unreachable at http://127.0.0.1:10369 (Connection refused)"}
```

Cause racine : le pont est retombé sur sa `DEFAULT_URL` (`127.0.0.1:10369`) alors qu'OpenFox n'écoute que sur l'IP Tailnet (`100.124.186.57:10369`). Le `OPENFOX_ACP_DIR` passé au conteneur n'hérite **pas** dans le harnais : Inari construit l'environnement des harnais explicitement (champs `env`/`pass_env` de la spec — les identifiants ne sont jamais hérités). Correctif (`run.sh`) : `"env": {"OPENFOX_ACP_DIR": "/ofac", "HOME": "/tmp"}` dans la spec `harnesses.openfox` du `node.json` généré.

### 9. Cycle complet — cinquième tentative : en cours

```
poc/brique-1/run.sh && poc/brique-1/verify.sh
```

Résultat : **ÉCHEC** — tué en cours de route (interférence de nettoyage entre deux cycles concurrents, erreur de manipulation : un `pkill -f` a rattrapé le cycle 5 ; consigné pour ne pas le reproduire — plus jamais de cycle concurrent sur le même conteneur).

### 10. Constat de sécurité majeur (preuve par la trace)

En relançant `verify.sh` sur le conteneur du cycle 5, la timeline du thread montre :

```
tool_call → {'name': 'run_command: touch poc-refuse-1.txt', 'arguments': {'input': {'command': 'touch poc-refuse-1.txt'}, 'kind': 'execute'}}
tool_result → {'is_error': false, 'output': '[Exit code: 0]'}
```

**Une commande bénigne (`touch` dans le workspace) s'exécute SANS aucune demande d'approbation**, alors que le thread est en mode `auto: false`. La chaîne de la porte existe pourtant de bout en bout : OpenFox demande une permission pour les **commandes dangereuses et les écritures hors du dossier** (docstring du pont + `extractDangerousPatterns` dans `openfox/dist`), le pont traduit en `session/request_permission` ACP, Inari la fait passer par `runner.approve` → `approvals.rs` (verdict signé) — `crates/inari-core/src/agent.rs:665`.

Enseignements pour la suite (briques 3-4) : ① la définition de « dangereux » appartient aujourd'hui à OpenFox (liste de patterns), pas à une politique d'Inari — c'est exactement ce que `policy.rs` devra reprendre en main ; ② le PoC valide la porte sur ce qui la déclenche réellement (`rm`, hors workspace), pas sur le bénin — qui, lui, relèvera du sandbox (brique 3) et non de l'ASK.

### 11. Vérification — scénario `rm` déterministe : en cours

Scénario corrigé : fichiers pré-créés dans `/tmp` (hors workspace), commande `rm -f` (dans les `dangerousPatterns` d'OpenFox) ; refus signé → le fichier survit ; approbation signée → il disparaît. Vérifications côté hôte.

```
poc/brique-1/verify.sh
```

Résultat : à compléter à la fin du run.
