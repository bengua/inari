# Installer la branche `claude/laughing-maxwell-sc9qz4` pour la tester (instructions pour un agent)

> Écrit en français, pour l'agent local de Ben. Listé dans `.english-exceptions`.
> Ces instructions installent la branche **sans la fusionner** dans `main`. Tout peut être annulé (étape 9).

## Règles à respecter pendant toute l'installation

1. Exécute les étapes **dans l'ordre**, une par une.
2. Après chaque commande, compare le résultat avec la ligne « Attendu ». Si ça ne correspond pas : **arrête-toi**,
   n'essaie pas de corriger toi-même, et envoie à Ben la commande, sa sortie complète et le numéro de l'étape.
3. **N'installe rien en Python** (ni `pip`, ni `uv`, ni service Python). Inari est maintenant 100 % Rust.
4. **Ne touche jamais à la branche `main`** : pas de `merge`, pas de `push` sur `main`.
5. Ne supprime aucun fichier. Les anciennes versions sont gardées avec le suffixe `.prev`, pour revenir en arrière.
6. Les commandes sont pour `bash`. Les variables définies à une étape (`BIN`, `CONF`…) servent aux étapes suivantes :
   garde le même terminal du début à la fin. Si tu dois en ouvrir un autre, refais l'étape 1.

---

## Étape 1 — Trouver où Inari est installé sur cette machine

```bash
systemctl --user cat inari-core.service
```
Attendu : un fichier qui contient une ligne `ExecStart=/chemin/vers/inari-core-rs node /chemin/vers/config.json`.

- Si la commande répond `No files found` : cherche le processus avec
  ```bash
  ps -eo pid,args | grep "[i]nari-core-rs node"
  ```
  puis **arrête-toi** et envoie le résultat à Ben (le service n'est pas installé comme prévu).

Mets les deux chemins dans des variables :

```bash
EXEC=$(systemctl --user show -p ExecStart --value inari-core.service | grep -o 'path=[^ ;]*' | cut -d= -f2)
BIN="$EXEC"
CONF=$(systemctl --user cat inari-core.service | grep '^ExecStart=' | awk '{print $3}')
echo "BIN=$BIN"; echo "CONF=$CONF"
ls -l "$BIN" "$CONF"
```
Attendu : deux lignes `BIN=…` et `CONF=…` avec des chemins non vides, puis `ls` qui montre les deux fichiers.
Si l'un est vide ou introuvable : arrête-toi et envoie la sortie de `systemctl --user cat inari-core.service`.

Lis les réglages utiles de la configuration :

```bash
grep -E '"(node_id|data_dir|mascot_dir|gateway_listen|gateway_token_file)"' "$CONF"
```
Attendu : au moins `node_id` et `data_dir`. Note la valeur de `gateway_listen` (par exemple `127.0.0.1:7487`) et
celle de `mascot_dir` si elle existe. Mets-les en variables (remplace par les valeurs lues, sans guillemets) :

```bash
GW="127.0.0.1:7487"                         # la valeur de gateway_listen ; laisse vide s'il n'y en a pas
TOKEN_FILE="/chemin/lu/dans/gateway_token_file"   # laisse vide s'il n'y en a pas
MASCOT="/chemin/lu/dans/mascot_dir"         # laisse vide s'il n'y en a pas
```

## Étape 2 — Récupérer le code d'Inari (dépôt `bengua/inari`)

```bash
if [ -d ~/Work/inari/.git ]; then cd ~/Work/inari; else git clone https://github.com/bengua/inari ~/Work/inari && cd ~/Work/inari; fi
git status --short
```
Attendu : `git status --short` n'affiche **rien**. S'il affiche des fichiers, arrête-toi et envoie la liste à Ben.

```bash
git fetch origin claude/laughing-maxwell-sc9qz4
git checkout claude/laughing-maxwell-sc9qz4
git pull --ff-only origin claude/laughing-maxwell-sc9qz4
git log --oneline -1
```
Attendu : la dernière ligne est le dernier commit de la branche (le même identifiant que `git ls-remote origin claude/laughing-maxwell-sc9qz4`), avec un message
en anglais. Si `checkout` ou `pull` échoue, arrête-toi.

## Étape 3 — Vérifier et construire

```bash
./verify.sh 2>&1 | tail -5
```
Attendu : la dernière ligne est exactement `PASSED`. Ça peut prendre plusieurs minutes. Si tu vois `FAILED`, arrête-toi
et envoie les 40 dernières lignes (`./verify.sh 2>&1 | tail -40`).

```bash
cargo build --release
ls -l target/release/inari-core-rs
target/release/inari-core-rs 2>&1 | grep -c "demo"
```
Attendu : le fichier `target/release/inari-core-rs` existe, et la dernière commande affiche un nombre **supérieur à 0**.

```bash
NEW="$PWD/target/release/inari-core-rs"
```

## Étape 4 — Récupérer FoxBot (dépôt `bengua/inari-apk`, alias `swarmfox`)

```bash
[ -d ~/Work/swarmfox/.git ] || git clone https://github.com/bengua/inari-apk ~/Work/swarmfox
cd ~/Work/swarmfox
git status --short
```
Attendu : rien. Si des fichiers apparaissent (par exemple des sprites en cours de génération), **arrête-toi** et
demande à Ben : ne fais ni `stash` ni `checkout` sans son accord.

```bash
git fetch origin claude/laughing-maxwell-sc9qz4
git checkout claude/laughing-maxwell-sc9qz4
git pull --ff-only origin claude/laughing-maxwell-sc9qz4
grep -c "choosePersona" android/app/src/main/assets/fox/js/sprite.js
```
Attendu : la dernière commande affiche un nombre **supérieur à 0**.

## Étape 5 — Préparer le dossier de FoxBot que le cœur sert

Le cœur sert FoxBot depuis `mascot_dir`. Pour que le choix d'avatar marche, ce dossier doit contenir le FoxBot à jour
**et** un vrai dossier `sprites_persona/` (pas un lien symbolique vers l'extérieur : le cœur le refuse).

On utilise un dossier dédié, hors de tout dépôt git :

```bash
FOXDIR="$HOME/.local/share/inari/foxbot"
mkdir -p "$FOXDIR"
rsync -a --delete --exclude sprites_persona ~/Work/swarmfox/android/app/src/main/assets/fox/ "$FOXDIR/"
rsync -a --delete ~/Work/swarmfox/fox-sprites-persona/ "$FOXDIR/sprites_persona/"
ls "$FOXDIR"
ls "$FOXDIR/sprites_persona" | wc -l
```
Attendu : `ls` montre au moins `index.html`, `js`, `sprites` et `sprites_persona` ; le nombre de personas est **18**.
Si `rsync` n'existe pas : arrête-toi et demande à Ben de l'installer.

Les images de la branche ont les yeux réparés (avant, le détourage rendait les pupilles transparentes et le texte se
voyait à travers). Vérifie qu'il ne reste aucune image à réparer dans ce dossier (par exemple des séquences générées
sur cette machine et pas encore sur GitHub). Le script n'installe rien : il utilise le Python, Pillow et numpy qui
servent déjà à générer les sprites.

```bash
cd ~/Work/swarmfox/android/app/src/main/assets/fox
python3 seal_sprites.py "$FOXDIR/sprites" "$FOXDIR/sprites_persona" | tail -3
python3 seal_sprites.py --dry-run "$FOXDIR/sprites" "$FOXDIR/sprites_persona" | grep "would be sealed"
```
Attendu : la première commande affiche `N of M frames sealed` (N vaut 0 si tout venait de GitHub), et peut citer
`sprites/parle/f_37.webp` comme « not an image » (ce fichier est vide dans le dépôt, c'est connu). La seconde affiche
`0 of M frames would be sealed`. Les nouvelles séquences générées avec `process_action.py` de cette branche sont
réparées automatiquement.

Si `MASCOT` (étape 1) vaut déjà exactement `$FOXDIR`, passe à l'étape 6. Sinon, il faut que la configuration pointe
sur `$FOXDIR`. Garde d'abord une copie :

```bash
cp "$CONF" "$CONF.prev"
```

- Si la configuration contient déjà une ligne `"mascot_dir"`, remplace sa valeur :
  ```bash
  sed -i "s|\"mascot_dir\": *\"[^\"]*\"|\"mascot_dir\": \"$FOXDIR\"|" "$CONF"
  ```
- Si elle n'en contient pas, **arrête-toi** et demande à Ben s'il veut afficher FoxBot sur ce cœur (sans `mascot_dir`,
  le cœur affiche l'orbe et le choix d'avatar n'apparaît pas).

Vérifie que le fichier est toujours un JSON valide :

```bash
grep '"mascot_dir"' "$CONF"
if command -v jq >/dev/null; then jq empty "$CONF" && echo "json ok"; else python3 -c "import json,sys; json.load(open(sys.argv[1])); print('json ok')" "$CONF"; fi
```
Attendu : la ligne `"mascot_dir": "/home/…/.local/share/inari/foxbot"`, puis `json ok`. Si tu n'as ni python3 ni jq,
ouvre le fichier et vérifie à l'œil que la ligne modifiée a gardé sa virgule à la fin. Si le JSON est cassé, remets
la copie (`cp "$CONF.prev" "$CONF"`) et arrête-toi.

## Étape 6 — Remplacer le binaire et redémarrer

```bash
cp "$BIN" "$BIN.prev"
systemctl --user stop inari-core.service
cp "$NEW" "$BIN"
systemctl --user start inari-core.service
sleep 5
systemctl --user is-active inari-core.service
journalctl --user -u inari-core.service -n 20 --no-pager
```
Attendu : `active`, et dans le journal une ligne `node <nom> up: endpoint …` après le redémarrage.
Si c'est `failed` ou `activating` au bout de 30 s : fais l'étape 9 (retour en arrière) puis envoie le journal à Ben.

## Étape 7 — Vérifier que la nouvelle version répond

À faire seulement si `GW` n'est pas vide (cette machine sert l'interface).

```bash
curl -s -o /dev/null -w "%{http_code}\n" "http://$GW/"
curl -s "http://$GW/assets/i18n.js" | head -c 13; echo
```
Attendu, dans l'ordre : `200`, puis `"use strict";` (le fichier des traductions, qui n'existait pas avant : c'est la
preuve que la nouvelle version tourne).

```bash
curl -s -H "Authorization: Bearer $(cat "$TOKEN_FILE")" "http://$GW/api/personas" | head -c 300; echo
```
Attendu : un JSON qui commence par `{"current":` ou `{"personas":` et qui cite des noms comme `"artiste"`,
`"rebel"`, `"zen"`. Si la liste est vide (`"personas":[]`), l'étape 5 n'a pas pris : vérifie que `mascot_dir` vaut
bien `$FOXDIR` et que le service a redémarré.

## Étape 8 — Les autres machines

Refais les étapes 1, 2, 3 et 6 sur **chaque** autre machine où tourne `inari-core.service`. Fais aussi les étapes 4
et 5 si cette machine a un `mascot_dir` (elle sert l'interface avec FoxBot). Toutes les machines doivent avoir le
même binaire.

## Étape 9 — Revenir en arrière (seulement si quelque chose ne va pas)

```bash
systemctl --user stop inari-core.service
cp "$BIN.prev" "$BIN"
[ -f "$CONF.prev" ] && cp "$CONF.prev" "$CONF"
systemctl --user start inari-core.service
systemctl --user is-active inari-core.service
```
Attendu : `active`. L'ancienne version tourne de nouveau. Les dépôts restent sur la branche : ça n'a pas d'effet
sur ce qui tourne.

## Étape 10 — Ce que Ben vérifie lui-même (à lui transmettre)

Ouvrir la webapp, puis :
1. La page se recharge seule. La première fois, elle peut redemander le jeton.
2. Volet de gauche : deux petites flèches ‹ › autour de FoxBot. Les cliquer change l'avatar, et le nom s'affiche
   un instant.
3. Ouvrir un fil, puis replier le volet de gauche : FoxBot flotte en haut au centre de la discussion, à la taille
   qu'il a dans le volet, dans une bande à lui ; le fil défile en dessous (aucun texte ne passe derrière lui) et
   les nouveaux messages arrivent en bas, sans être masqués. Ses yeux sont pleins : on ne voit plus le texte à
   travers (il faut avoir refait les étapes 4 et 5, qui recopient les images réparées).
4. Les onglets Artefacts / Terminal / Visionneuse / Journal du volet de droite se cliquent.
5. Glisser un fil : les tuiles « Le résumer », « L'endormir » et « Supprimer… » (rouge) apparaissent. « Supprimer »
   demande une confirmation.
6. Réglages (l'engrenage) → Langue : bascule entre français et anglais.

## Étape 11 — Le compte rendu à envoyer à Ben

Envoie un message court avec :
- pour chaque machine : son nom, `active` ou non, et la ligne `node … up` du journal ;
- le résultat des commandes de l'étape 7 ;
- toute étape où tu t'es arrêté, avec la sortie complète ;
- la confirmation que les fichiers `.prev` existent (`ls -l "$BIN.prev" "$CONF.prev"`).

**Ne pas faire** : installer l'APK Android (il n'est pas compilé et pas nécessaire pour ce test), fusionner la
branche, supprimer les `.prev`.
