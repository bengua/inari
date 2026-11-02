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

### 5. Bascule des déploiements (3 cœurs)

Critère : le binaire déployé sur laptop, gamer et mindhub provient du dépôt
(hash comparable), l'ancien `~/.local/bin/openfox-acp` reste en place jusqu'à
bascule explicite (jamais de delete/recreate), et un thread Inari↔OpenFox
réel passe sur gamer.

Résultat : à compléter.
