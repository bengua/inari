#!/usr/bin/env bash
# Brique 1 — construit l'image, prépare un répertoire d'exécution jetable et
# lance le conteneur du PoC. Aucun service de production n'est touché :
# ports 17xxx, données sous /tmp, conteneur nommé inari-poc-brique-1.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
RUN=${RUN_DIR:-/tmp/inari-poc-brique-1}
IMG=inari-poc-brique-1:local
NAME=inari-poc-brique-1
GW_PORT=17487

echo "== build de l'image (rust musl, long au premier run) =="
docker build -t "$IMG" -f "$HERE/Dockerfile" "$ROOT"

echo "== répertoire d'exécution : $RUN =="
docker rm -f "$NAME" >/dev/null 2>&1 || true
# des fichiers du run précédent peuvent appartenir à root (écrits par le conteneur)
rm -rf "$RUN" 2>/dev/null || docker run --rm -v "$(dirname "$RUN"):/c" alpine:3.21 sh -c "rm -rf '/c/$(basename "$RUN")'"
mkdir -p "$RUN/data"

# clés de 20 octets, même forme que la production (mesh.key / device.key / token)
openssl rand -hex 10 > "$RUN/data/mesh.key"
openssl rand -hex 10 > "$RUN/data/device.key"
openssl rand -hex 10 > "$RUN/data/token"

# Le répertoire de données est monté AU MÊME CHEMIN ABSOLU dans le conteneur et
# sur l'hôte : l'OpenFox local (hors conteneur) crée le workspace du thread à ce
# chemin — un chemin interne au conteneur lui est invisible (HTTP 403 au premier run).
cat > "$RUN/data/node.json" <<JSON
{
  "node_id": "poc-brique-1",
  "role": "heavy",
  "data_dir": "$RUN/data",
  "key_file": "$RUN/data/mesh.key",
  "device_key_file": "$RUN/data/device.key",
  "bind_host": "127.0.0.1",
  "advertise_host": "127.0.0.1",
  "bus_port": 17400,
  "endpoint_port": 17411,
  "hub": "",
  "approval_ttl": 300,
  "gateway_listen": "127.0.0.1:$GW_PORT",
  "gateway_token_file": "$RUN/data/token",
  "gateway_trust_loopback": true,
  "gateway_ui": {"brand": "Inari PoC brique 1", "owner": "Ben"},
  "harnesses": {
    "openfox": {
      "command": ["/usr/local/bin/openfox-acp"],
      "timeout": 900,
      "env": {"OPENFOX_ACP_DIR": "/ofac", "HOME": "/tmp"}
    }
  }
}
JSON

echo "== conteneur : réseau hôte (joindre l'OpenFox local), UID de l'hôte (OpenFox, hors conteneur, écrit dans le même workspace), données au chemin partagé =="
docker run -d --name "$NAME" --network host \
  --user "$(id -u):$(id -g)" -e HOME=/tmp -e OPENFOX_ACP_DIR=/ofac \
  -v "$RUN/data:$RUN/data" \
  -v "$HOME/.local/bin/openfox-acp:/usr/local/bin/openfox-acp:ro" \
  -v "$HOME/.config/openfox-acp:/ofac:ro" \
  "$IMG" node "$RUN/data/node.json"

echo "== attente de la gateway sur 127.0.0.1:$GW_PORT =="
TOKEN=$(cat "$RUN/data/token")
for i in $(seq 1 60); do
  if curl -sf -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$GW_PORT/api/whoami" >/dev/null 2>&1; then
    echo "gateway OK (tentative $i)"
    echo "preuve de capacité sandbox dans le conteneur :"
    docker exec "$NAME" bwrap --version
    echo "PoC prêt. Vérification : $HERE/verify.sh"
    exit 0
  fi
  sleep 2
done
echo "ÉCHEC : la gateway n'a pas répondu en 120 s" >&2
docker logs "$NAME" 2>&1 | tail -30 >&2
exit 1
