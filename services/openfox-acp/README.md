# openfox-acp — OpenFox comme agent ACP pour Inari

Inari parle aux agents de code en **Agent Client Protocol** (JSON-RPC, un
message par ligne, sur stdin/stdout). OpenFox n'a pas d'ACP propre : il a une
API HTTP et un WebSocket qui diffuse ce que fait une session. Ce pont traduit
entre les deux : **une session OpenFox par thread Inari**, sur l'OpenFox de la
machine, avec les modèles, outils et confirmations d'OpenFox.

Bibliothèque standard uniquement — aucune dépendance. stdout ne porte que le
protocole ; tous les diagnostics vont sur stderr, qu'Inari garde dans ses
journaux.

## Commandes

```
openfox-acp            parle ACP sur stdin/stdout (ce qu'Inari lance)
openfox-acp login      lit le mot de passe OpenFox sur stdin, garde le token
openfox-acp check      dit si OpenFox répond avec le token gardé
```

## Configuration (par machine)

Dans `~/.config/openfox-acp/` (ou `$OPENFOX_ACP_DIR`) :

- `url` — adresse d'OpenFox (défaut `http://127.0.0.1:10369`,
  surchargeable par `$OPENFOX_URL`) ;
- `token` — ce que `login` a écrit (mode 600). OpenFox le dérive de son mot
  de passe : il reste valide jusqu'au prochain changement de mot de passe.

`$OPENFOX_ACP_MODE` (`builder` par défaut, `planner` en lecture seule) choisit
le mode de la session. OpenFox demande toujours confirmation pour les
commandes dangereuses et les écritures hors du dossier : ces demandes
arrivent à Inari comme des demandes de permission ACP (verdict signé par la
porte d'approbation d'Inari).

## Déploiement sur un cœur

```
pip install .            # rend la commande `openfox-acp`
openfox-acp login        # une fois, mot de passe OpenFox sur stdin
openfox-acp check        # doit répondre ok
```

Puis déclarer le harnais dans le `node.json` d'Inari (champ `commande` :
`openfox-acp`). Le binaire déployé doit provenir de ce dépôt : comparer le
hash de `openfox_acp.py` avec celui du service installé.

## Tests

```
python -m unittest discover -s tests -v
```
