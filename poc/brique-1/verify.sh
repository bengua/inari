#!/usr/bin/env bash
# Brique 1 — critère d'acceptation scripté (voir 03-plan.md). Mesures produites
# par ce script, jamais écrites à la main. Scénario joué DEUX FOIS de suite :
#   1. la gateway répond (whoami, propriétaire)
#   2. un thread OpenFox est créé et parle au modèle local
#   3. une commande privilégiée déclenche l'ASK avec la commande exacte
#   4. le refus la bloque (le fichier n'existe pas)
#   5. l'approbation signée la laisse passer (le fichier existe)
#   6. la trace (capsule du thread) contient les événements
set -euo pipefail

RUN=${RUN_DIR:-/tmp/inari-poc-brique-1}
NAME=inari-poc-brique-1
GW=http://127.0.0.1:17487
TOKEN=$(cat "$RUN/data/token")
AUTH="Authorization: Bearer $TOKEN"
THREAD=poc-brique-1-thread

say() { printf '\n### %s\n' "$*"; }
die() { echo "ÉCHEC : $*" >&2; exit 1; }

# Un instantané SSE frais (la gateway le pousse à l'ouverture du flux).
snap() { curl -s -N --max-time 8 -H "$AUTH" "$GW/events" 2>/dev/null || true; }

# request_id de la demande d'approbation portant la commande exacte $1 ; vide si absente.
approval_id() {
  snap | python3 -c "
import json,sys
for line in sys.stdin:
    line=line.strip()
    if not line.startswith('data:'): continue
    try: ev=json.loads(line[5:])
    except Exception: continue
    snap=ev.get('snapshot',ev)
    for a in snap.get('approvals',[]):
        if '$1' in str(a.get('command','')):
            print(a.get('request_id','')); break
    else: continue
    break
"
}

wait_approval() { # $1 = marqueur de commande, $2 = budget secondes
  local t=0 id=""
  while [ $t -lt "$2" ]; do
    id=$(approval_id "$1")
    [ -n "$id" ] && { echo "$id"; return 0; }
    sleep 10; t=$((t+10))
  done
  return 1
}

run_scenario() { # $1 = numéro du run
  # `rm` est dans les dangerousPatterns d'OpenFox → demande de permission vers
  # Inari à coup sûr. Fichiers pré-créés : refus → le fichier survit ;
  # approbation → il disparaît. (Un touch bénin dans le workspace s'exécute
  # sans ASK : vérifié empiriquement, cf. journal.)
  local n=$1 REFUSE="/tmp/poc-inari-refuse-$n.txt" ACCEPTE="/tmp/poc-inari-accepte-$n.txt"
  echo "cible du refus $n" > "$REFUSE"
  echo "cible de l'approbation $n" > "$ACCEPTE"

  say "run $n — 1. gateway"
  curl -sf -H "$AUTH" "$GW/api/whoami" | grep -q . || die "whoami muet"

  say "run $n — 2. thread OpenFox sur le modèle local"
  curl -sf -X POST -H "$AUTH" -H 'Content-Type: application/json' \
    -d "{\"thread_id\":\"$THREAD\",\"title\":\"PoC brique 1\",\"harness\":\"openfox\",\"node\":\"poc-brique-1\"}" \
    "$GW/api/agent" | grep -q '"ok":true' || echo "(le thread existe peut-être déjà, on continue)"

  say "run $n — 3. commande privilégiée → ASK avec la commande exacte"
  curl -sf -X POST -H "$AUTH" -H 'Content-Type: application/json' \
    -d "{\"agent\":\"$THREAD\",\"text\":\"Utilise ton outil shell pour exécuter exactement cette commande : rm -f $REFUSE — rien d'autre.\"}" \
    "$GW/api/message" >/dev/null
  local rid
  rid=$(wait_approval "poc-inari-refuse-$n" 280) || die "aucune demande d'approbation pour '$REFUSE' en 280 s"
  [ -n "$rid" ] || die "demande vide"
  echo "demande reçue, request_id=$rid — la commande exacte était visible dans la demande"

  say "run $n — 4. refus signé → la commande est bloquée"
  curl -sf -X POST -H "$AUTH" -H 'Content-Type: application/json' \
    -d "{\"request_id\":\"$rid\",\"approved\":false}" "$GW/api/approval" | grep -q '"ok":true' \
    || die "le verdict de refus n'a pas été accepté"
  sleep 20 # laisse le tour se terminer sur le refus
  [ -e "$REFUSE" ] || die "le fichier $REFUSE a disparu malgré le refus"
  rm -f "$REFUSE"
  echo "bloqué : $REFUSE a survécu au refus signé"

  say "run $n — 5. approbation signée → la commande passe"
  curl -sf -X POST -H "$AUTH" -H 'Content-Type: application/json' \
    -d "{\"agent\":\"$THREAD\",\"text\":\"Utilise ton outil shell pour exécuter exactement cette commande : rm -f $ACCEPTE — rien d'autre.\"}" \
    "$GW/api/message" >/dev/null
  rid=$(wait_approval "poc-inari-accepte-$n" 280) || die "aucune demande d'approbation pour '$ACCEPTE' en 280 s"
  curl -sf -X POST -H "$AUTH" -H 'Content-Type: application/json' \
    -d "{\"request_id\":\"$rid\",\"approved\":true}" "$GW/api/approval" | grep -q '"ok":true' \
    || die "le verdict d'approbation n'a pas été accepté"
  local t=0 ok=""
  while [ $t -lt 120 ]; do
    [ ! -e "$ACCEPTE" ] && { ok=1; break; }
    sleep 10; t=$((t+10))
  done
  [ -n "$ok" ] || die "le fichier $ACCEPTE existe encore malgré l'approbation"
  echo "passé : $ACCEPTE supprimé après verdict signé"

  say "run $n — 6. trace"
  docker exec "$NAME" sh -c "find "$RUN/data" -type f -newer "$RUN/data/node.json" | grep -q ." \
    || die "aucune trace écrite dans /data"
  snap | grep -q "$THREAD" || die "le thread n'apparaît plus dans l'instantané"
  echo "trace présente : fichiers du thread + événements visibles dans l'instantané"
}

for n in 1 2; do
  run_scenario "$n"
done

say "BRIQUE 1 : PASSED (scénario complet, 2 fois de suite)"
