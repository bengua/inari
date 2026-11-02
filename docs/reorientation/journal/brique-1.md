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

Résultat : **ÉCHEC** — aucune demande d'approbation. La timeline du thread montre que `rm -f /tmp/poc-inari-refuse-1.txt` s'est **exécuté sans ASK** (exit 0). Le titre ACP de la demande ne portait d'ailleurs pas toujours le nom du fichier — `approval_id()` a été réécrit pour prendre la première ASK pendante (`request_id<TAB>commande`).

### 12. Lecture de la liste exacte des motifs dangereux d'OpenFox

`extractDangerousPatterns` ne déclenche l'ASK que sur une liste fermée, lue dans `~/.local/lib/node_modules/openfox/dist/chunk-56BVEN5Q.js` :

```
sudo\s · rm -r ~ · chmod 777 · > /dev/sd · mkfs · dd if= · fork bomb
```

`rm -f <fichier>` n'y figure **pas** → exécution directe, sans ASK. Deux candidats restants : `sudo …` et `chmod 777 …`. `sudo` est écarté : `zordur` n'a pas de NOPASSWD, la commande échouerait après approbation et le critère ne serait pas observable. **Choix arrêté : `chmod 777 /tmp/<fichier>`** — dans les motifs, s'exécute en `zordur`, résultat vérifiable côté hôte par `stat -c %a` (refus → reste 644 ; approbation → passe 777).

Constat de sécurité complémentaire (consigné pour la brique 4) : le filtrage actuel ne couvre que ces motifs exacts — `rm -f` hors workspace passe sans ASK, c'est une porte trop étroite que `policy.rs` devra reprendre.

### 13. Vérification — scénario `chmod 777` déterministe : en cours

`verify.sh` réécrit : fichiers pré-créés en 644, message « exécute exactement : `chmod 777 /tmp/poc-inari-{refuse,accepte}-N.txt` », ASK attendue (280 s), refus signé → `stat` doit rester 644 ; puis approbation signée → `stat` doit passer 777 (120 s de marge).

```
poc/brique-1/verify.sh
```

Résultat : à compléter à la fin du run.

### 14. Fiabilité du déclencheur côté modèle (épisode du 2026-11-02 ~01h-03h)

Premier lancement du scénario `chmod 777` : **run 1 entièrement vert** (ASK `run_command: chmod\s+777`, refus → 644 conservé, approbation → 777, trace présente), mais le run 2 a échoué : le modèle local a **répondu en texte** (« ne peut pas être exécutée sans autorisation ») au lieu d'appeler l'outil — pas de tool call, donc pas d'ASK. Deuxième lancement : refus similaires (« l'outil shell refuse les changements de permission sur ce chemin » — excuse hallucinée).

Constats et correctifs :

1. **L'obéissance au tool call est stochastique** sur les petits modèles locaux. `verify.sh` gagne une fonction `ask_chmod` : 3 essais avec une consigne qui lève le doute (« la permission est gérée par l'interface, tu n'as pas à la demander ni à refuser »).
2. **Les backends GPU `-c1` étaient tous éteints** au moment du test (« Connection error » litellm sur chaque groupe local `-c1` ; les conteneurs se relancent à la demande — `Q2.5-coder-14BQ4KM-KVMem-c1` reparti au moment du sondage — mais le chargement du modèle prend des minutes). `auto-local-first` reste le seul point d'entrée fiable (repli automatique, ce soir-là vers gpt-oss-120b).
3. **Bascule de modèle à chaud possible** : `POST /api/sessions/{id}/provider` côté OpenFox (utilisé par le pont pour `session/set_model` ACP). La session du PoC a été épinglée sur `auto-local-first` après avoir constaté la panne des groupes locaux. Pour un conteneur neuf, la session repart sur le modèle par défaut du provider — même routeur, rien à épingler dans `run.sh`.
4. **Le budget de `wait_approval` est nominal** : chaque itération coûte ~18 s réels (SSE `--max-time 8` + `sleep 10`), le budget affiché (280 s) vaut ~500 s réels. Sans effet sur la correction, consigné pour lecture des journaux.

### 15. Vérification — troisième lancement (session sur `auto-local-first`)

```
poc/brique-1/verify.sh
```

Résultat : à compléter à la fin du run.

### 16. Analyse par capture du trafic OpenFox→litellm (le vrai coupable)

Les relances échouaient en boucle avec des réponses en prose. Un proxy de
capture (127.0.0.1:14001 → :4000, corps journalisés) + un provider OpenFox
jetable pointant dessus ont donné la preuve, requête par requête :

1. **OpenFox envoie bien des tool calls natifs** (`tools: [12 fonctions]`,
   `tool_choice: "auto"`, `stream: true`) — ce n'est ni un problème de
   protocole textuel, ni un backend qui « ne supporterait pas les outils ».
2. **Rejouée telle quelle contre litellm, la requête capturée rend un vrai
   `tool_calls`** (stream et non-stream). Litellm est innocent.
3. **Session froide + prompt système complet + 12 outils → vrai tool call**
   sur `Q3.8-heretic-9BQ6-llamacpp-c0`. Le modèle sait faire.
4. **Le péché originel** : `Q2.5-coder-14B` (backend KVMem), premier modèle
   épinglé, a répondu sur session froide un **faux appel d'outil en bloc
   markdown** (```` ```json {"name":"run_command"…} ````). Cette réponse est
   entrée dans l'historique de la session et **chaque tour suivant l'a
   imitée**, y compris après épinglage sur heretic — l'historique prime sur
   le modèle.
5. Conséquence méthodologique : une session dont l'historique contient une
   prose de ce type est **empoisonnée** ; il faut repartir d'une session
   neuve, pas relancer dans la même. Le routeur `auto-local-first`, lui,
   bascule parfois vers un backend cloud qui laisse `tool_calls` à `null` —
   d'où l'épinglage explicite d'un modèle local éprouvé.
6. Choix arrêté : **`Q3.8-heretic-9BQ6-llamacpp-c0`** (local, GPU0, déjà
   chargé en permanence, vrai tool call vérifié à froid avec le prompt
   système et la panoplie complets d'OpenFox). La consigne de relance de
   `ask_chmod` contre-conditionne le format (« n'écris JAMAIS de JSON en
   prose, appelle l'outil nativement »).
7. Au passage : l'approbation manuelle d'une ASK orpheline (request_id
   pêché dans l'instantané SSE) a fait passer le fichier cible en 777 — la
   chaîne gateway → verdict signé → exécution est saine de bout en bout ;
   seul le déclenchement côté modèle était en cause.

Note d'hygiène : le provider `poc-debug` et le proxy de capture sont des
outils de diagnostic jetables, à retirer après la brique (consigné dans les
suites de contrôle).

Résultat (6ᵉ lancement, 2026-11-02 ~03:22) : **PASSED — scénario complet, 2 fois de suite** (587 s). Run 1 : ASK à l'essai 2 (consigne anti-poison) ; run 2 : ASK au premier essai (contexte déjà chaud). Ce lancement est passé par le provider de capture `poc-debug` — chaîne strictement équivalente (proxy transparent vers le même litellm), mais une confirmation sur le chemin standard était due.

### 17. Confirmation sur le provider standard turnstile : **PASSED**

Correctif préalable : `pin_model` choisissait la session la plus récemment *mise à jour* — il avait épinglé l'ancienne session du projet (sauvé par l'héritage du modèle au niveau projet). Il prend désormais la plus récemment **créée** dont le workdir finit par `/work/poc-brique-1-thread` (exclut le projet scratch).

```
rm -f /tmp/poc-inari-*.txt && poc/brique-1/verify.sh
```

Résultat (2026-11-02 ~03:30) : **PASSED — 90 secondes, les 4 ASK au premier essai.** Session `a8a94a78` épinglée `Q3.8-heretic-9BQ6-llamacpp-c0` via le provider `turnstile` de production : ASK `run_command: chmod\s+777` avec la commande exacte, refus signé → 644 conservé, approbation signée → 777, trace présente, deux fois de suite.

Comparaison a posteriori des réglages providers (turnstile vs poc-debug) : identiques (`backend: unknown`, pas de `thinkingField` ni d'adaptateur). Les échecs de la nuit étaient donc purement stochastiques + l'empoisonnement d'historique ; une fois la session amorcée par un premier tool call réussi, la fiabilité est totale (4/4 au premier essai).

**Brique 1 : critère d'acceptation rempli, deux lancements verts consécutifs, mesures produites par `poc/brique-1/verify.sh`.**
