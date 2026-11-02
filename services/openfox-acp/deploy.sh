#!/usr/bin/env bash
# Déploie openfox-acp depuis un checkout du dépôt vers ~/.local/bin/openfox-acp
# (remplacement de fichier, jamais de delete/recreate : copie atomique par
# fichier temporaire + rename). Vérifie ensuite que le binaire installé est
# identique au fichier du dépôt (hash sha256 comparable).
#
#   deploy.sh [chemin-du-checkout]   (défaut : ~/inari-swarm-coordination)
set -euo pipefail

CHECKOUT=${1:-$HOME/inari-swarm-coordination}
SRC="$CHECKOUT/services/openfox-acp/openfox_acp.py"
DEST="$HOME/.local/bin/openfox-acp"

[ -f "$SRC" ] || { echo "introuvable : $SRC" >&2; exit 1; }
[ -d "$CHECKOUT/.git" ] || { echo "pas un checkout git : $CHECKOUT" >&2; exit 1; }

mkdir -p "$HOME/.local/bin"
TMP=$(mktemp "$HOME/.local/bin/.openfox-acp.XXXXXX")
cp "$SRC" "$TMP"
chmod 755 "$TMP"
mv "$TMP" "$DEST"

H_SRC=$(sha256sum "$SRC" | cut -d' ' -f1)
H_DEST=$(sha256sum "$DEST" | cut -d' ' -f1)
[ "$H_SRC" = "$H_DEST" ] || { echo "hash différents après copie !" >&2; exit 1; }
echo "déployé : $DEST"
echo "sha256  : $H_DEST"
echo "commit  : $(git -C "$CHECKOUT" rev-parse HEAD)"
"$DEST" check || echo "(check : OpenFox ne répond pas sur cette machine — à voir)"
