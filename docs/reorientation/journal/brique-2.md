# Journal — Brique 2 : absorption d'openfox-acp

Délégation écrite du propriétaire : « je me fis à toi, fais bien le job ».
Livrable en français, jamais de delete/recreate, licences permissives.

## Dette visée

Dette critique n°1 de `docs/reorientation/01-evaluation.md` : le pont
ACP↔OpenFox vivait dans `~/.local/bin/openfox-acp` (712 lignes) sur chaque
cœur — non versionné, sans tests, sans CI, déployé à la main. Toute
régression du pont cassait tous les threads Inari↔OpenFox sans qu'aucun
contrôle ne s'en aperçoive.

## Étapes

### 1. Déplacement sans réécriture

Le script est copié tel quel dans `services/openfox-acp/openfox_acp.py`
(seul le nom change : module importable). `pyproject.toml` : aucune
dépendance (bibliothèque standard uniquement), script console
`openfox-acp = openfox_acp:main`. `pip install .` rend la même commande
qu'avant — le déploiement bascule du fichier copié à la main vers le paquet
du dépôt.

### 2. Tests unitaires (22)

`tests/test_openfox_acp.py`, `unittest` standard :

- fonctions pures du traducteur : `tool_kind`, `tool_title`,
  `result_text`, `prompt_text`, `model_catalog` (catalogue ACP et modèle
  courant), `config_dir` / `openfox_url` (env + défaut) ;
- cadrage WebSocket RFC 6455 contre un mini-serveur socket local :
  handshake (clé accept vérifiée), texte fragmenté réassemblé, ping → pong,
  fermeture propre → `None`, handshake refusé → `OpenFoxError`.

Résultat : `python -m unittest discover -s tests` — **22 tests, OK**
(vérifié d'abord contre le script déployé, avant la création de la branche).

### 3. CI

Job `openfox-acp` ajouté à `.github/workflows/ci.yml`, copie conforme du
patron `voice` : setup-python 3.12, `pip install .`,
`python -m unittest discover -s tests -v`, répertoire
`services/openfox-acp`.

### 4. Notice

`THIRD-PARTY.md` : section précisant que `services/openfox-acp` est du code
original du projet (MIT), qu'il parle à l'API HTTP/WS d'OpenFox mais ne
contient aucun code d'OpenFox.

### 5. Bascule des déploiements (3 cœurs) : faite (2026-11-02)

Après fusion (commit `d848e5d`) :

- **laptop** : `git pull` sur `~/inari` puis `deploy.sh ~/inari` →
  `~/.local/bin/openfox-acp` sha256 `8b75ece8…` = fichier du dépôt, commit
  `d848e5d` ; `openfox-acp check` OK (45 projets).
- **mindhub** : même procédure sur `~/Work/inari` → même sha256,
  `openfox-acp check` OK (24 projets).
- **gamer** : l'arbre de travail est sur une autre branche avec des
  modifications en cours (`android/update-toast`) — **non touché**. Bascule
  par `git fetch` + `git show origin/main:…` : le binaire déployé était déjà
  octet pour octet celui du dépôt (même sha256), `openfox-acp check` OK
  (25 projets). Procédure consignée ici pour tout cœur dont l'arbre est sale.

Les trois cœurs exécutent donc le pont **provenant du dépôt** (sha256
`8b75ece812aea2b2c2d12da39c326f01c8d406e3bb50e88bcb1422a1013072d3`,
comparable au blob git du commit fusionné). Le déploiement était déjà
conforme partout — preuve que le script non versionné n'avait pas divergé —
mais il est désormais **traçable et testé**.

### 6. Thread réel sur gamer : PASS

Via la gateway de gamer (`127.0.0.1:7487`) : création du thread
`brique2-verif` (harnais `openfox`), message « Réponds exactement :
BRIQUE2-OK » → réponse `BRIQUE2-OK` visible dans l'instantané en 10 s.

Anomalie notée pour la brique 5 : le `node_id` de gamer dans
`~/.config/inari-core/node.json` vaut `laptop` (fichier visiblement copié
d'un cœur à l'autre) — à corriger lors de la reprise du graphe de cœurs.

### 7. CI

Job `openfox-acp` vert sur la PR #47 (11 s), aux côtés de `verify`,
`voice` et `android` — 4/4.

**Brique 2 : critère d'acceptation entièrement rempli.**
