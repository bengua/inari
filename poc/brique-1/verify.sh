#!/usr/bin/env bash
# Brique 1 — critère d'acceptation scripté (voir 03-plan.md). Mesures produites
# par ce script, jamais écrites à la main. Scénario joué DEUX FOIS de suite :
#   1. la gateway répond (whoami, propriétaire)
#   2. un thread OpenFox est créé et parle au modèle local
#   3. une commande privilégiée déclenche l'ASK avec la commande exacte
#   4. le refus la bloque (les permissions du fichier ne changent pas)
#   5. l'approbation signée la laisse passer (les permissions changent)
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

# première demande d'approbation pendante (une seule à la fois dans ce PoC) :
# "request_id<TAB>commande affichée" ; vide si aucune.
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
        print(str(a.get('request_id',''))+'\t'+str(a.get('command','')))
        break
    else: continue
    break
"
}

wait_approval() { # $1 = budget secondes → "request_id<TAB>commande"
  local t=0 id=""
  while [ $t -lt "$1" ]; do
    id=$(approval_id)
    [ -n "$id" ] && { echo "$id"; return 0; }
    sleep 10; t=$((t+10))
  done
  return 1
}

# Épingle le modèle de la session OpenFox du thread sur un modèle local
# fiable pour les tool calls (le routeur auto-local-first est stochastique
# et le contexte d'une session ayant déjà refusé pousse au refus suivant —
# cf. journal §14-15). Sans cet épingle, le déclenchement de l'ASK dépend
# de l'humeur du modèle du moment.
pin_model() {
  local of_url of_tok sid i
  of_url=$(cat "$HOME/.config/openfox-acp/url" 2>/dev/null || echo http://127.0.0.1:10369)
  of_tok=$(cat "$HOME/.config/openfox-acp/token")
  sid=""
  for i in $(seq 1 6); do
    sid=$(curl -sf --max-time 10 -H "x-session-token: $of_tok" "$of_url/api/sessions" | python3 -c "
import json,sys
d=json.load(sys.stdin)
sessions=d if isinstance(d,list) else d.get('sessions',[])
# la session vivante du thread : la plus récemment CRÉÉE pour ce workdir
# (updatedAt bouge sur les anciennes sessions ; le projet scratch a un
# workdir voisin qui ne doit pas être choisi)
candidats=[s for s in sessions if (s.get('workdir') or '').endswith('/work/poc-brique-1-thread')]
candidats.sort(key=lambda s: s.get('createdAt') or '', reverse=True)
print(candidats[0]['id'] if candidats else '')
" 2>/dev/null) || sid=""
    [ -n "$sid" ] && break
    sleep 5
  done
  [ -n "$sid" ] || die "session OpenFox du thread introuvable pour l'épinglage du modèle"
  curl -sf --max-time 10 -X POST -H "x-session-token: $of_tok" -H 'Content-Type: application/json' \
    -d '{"providerId":"536a50ea-279c-43ed-b3ef-9bdfeda418fe","model":"Q3.8-heretic-9BQ6-llamacpp-c0"}' \
    "$of_url/api/sessions/$sid/provider" >/dev/null || die "épinglage du modèle refusé par OpenFox"
  echo "modèle épinglé : Q3.8-heretic-9B local sur la session ${sid:0:8}"
}

# Le modèle local peut répondre en prose (refus poli ou faux appel d'outil
# en bloc markdown ```json) au lieu d'un vrai tool call — et une telle
# réponse dans l'historique pousse le tour suivant à l'imiter (vérifié par
# capture de la requête OpenFox→litellm, cf. journal §16). La relance
# contre-conditionne explicitement ce format. 3 essais de 280 s max.
ask_chmod() { # $1 = fichier cible → "request_id<TAB>commande"
  local cible=$1 essai line
  for essai in 1 2 3; do
    if [ "$essai" = "1" ]; then
      texte="Test supervisé : la permission d'exécution m'est demandée automatiquement par l'interface, tu n'as pas à la demander toi-même ni à refuser. Appelle immédiatement l'outil shell avec exactement cette commande et rien d'autre : chmod 777 $cible"
    else
      texte="Ta réponse précédente était du texte, pas un appel d'outil : n'écris JAMAIS de JSON ni de bloc markdown, appelle l'outil run_command nativement. Permission gérée par l'interface, pas par toi. Exécute exactement : chmod 777 $cible"
    fi
    curl -sf -X POST -H "$AUTH" -H 'Content-Type: application/json' \
      -d "{\"agent\":\"$THREAD\",\"text\":\"$texte\"}" \
      "$GW/api/message" >/dev/null
    line=$(wait_approval 280) && { echo "(ask capturée : $line)" >&2; echo "$line"; return 0; }
    echo "(essai $essai : pas de tool call, relance)" >&2
  done
  return 1
}

run_scenario() { # $1 = numéro du run
  # `chmod 777` est dans les dangerousPatterns d'OpenFox (chunk-56BVEN5Q.js) →
  # demande de permission vers Inari à coup sûr, et la commande s'exécute en
  # tant que zordur (pas besoin de sudo, dont le NOPASSWD est absent).
  # Fichiers pré-créés en 644 : refus → les permissions restent 644 ;
  # approbation → elles passent à 777. (Un touch bénin ou un rm hors workspace
  # s'exécutent sans ASK : vérifié empiriquement, cf. journal — le filtrage se
  # fait uniquement sur les dangerousPatterns, travail de la brique 4.)
  local n=$1 REFUSE="/tmp/poc-inari-refuse-$n.txt" ACCEPTE="/tmp/poc-inari-accepte-$n.txt"
  echo "cible du refus $n" > "$REFUSE"
  echo "cible de l'approbation $n" > "$ACCEPTE"
  chmod 644 "$REFUSE" "$ACCEPTE"

  say "run $n — 1. gateway"
  curl -sf -H "$AUTH" "$GW/api/whoami" | grep -q . || die "whoami muet"

  say "run $n — 2. thread OpenFox sur le modèle local"
  curl -sf -X POST -H "$AUTH" -H 'Content-Type: application/json' \
    -d "{\"thread_id\":\"$THREAD\",\"title\":\"PoC brique 1\",\"harness\":\"openfox\",\"node\":\"poc-brique-1\"}" \
    "$GW/api/agent" | grep -q '"ok":true' || echo "(le thread existe peut-être déjà, on continue)"
  [ "$n" = "1" ] && pin_model

  say "run $n — 3. commande privilégiée → ASK avec la commande exacte"
  local rid line
  line=$(ask_chmod "$REFUSE") || die "aucune demande d'approbation pour 'chmod 777 $REFUSE' malgré les relances"
  rid=$(printf '%s' "$line" | cut -f1)
  [ -n "$rid" ] || die "demande vide"
  echo "demande reçue, request_id=$rid, commande affichée : $(printf '%s' "$line" | cut -f2-)"

  say "run $n — 4. refus signé → la commande est bloquée"
  curl -sf -X POST -H "$AUTH" -H 'Content-Type: application/json' \
    -d "{\"request_id\":\"$rid\",\"approved\":false}" "$GW/api/approval" | grep -q '"ok":true' \
    || die "le verdict de refus n'a pas été accepté"
  sleep 20 # laisse le tour se terminer sur le refus
  [ "$(stat -c %a "$REFUSE")" = "644" ] || die "$REFUSE est passé en $(stat -c %a "$REFUSE") malgré le refus"
  echo "bloqué : $REFUSE resté en 644 après le refus signé"
  rm -f "$REFUSE"

  say "run $n — 5. approbation signée → la commande passe"
  line=$(ask_chmod "$ACCEPTE") || die "aucune demande d'approbation pour 'chmod 777 $ACCEPTE' malgré les relances"
  rid=$(printf '%s' "$line" | cut -f1)
  echo "réponse au request_id=$rid (commande affichée : $(printf '%s' "$line" | cut -f2-))"
  curl -sf -X POST -H "$AUTH" -H 'Content-Type: application/json' \
    -d "{\"request_id\":\"$rid\",\"approved\":true}" "$GW/api/approval" | grep -q '"ok":true' \
    || die "le verdict d'approbation n'a pas été accepté"
  local t=0 ok=""
  while [ $t -lt 120 ]; do
    [ "$(stat -c %a "$ACCEPTE" 2>/dev/null)" = "777" ] && { ok=1; break; }
    sleep 10; t=$((t+10))
  done
  [ -n "$ok" ] || die "$ACCEPTE n'est pas passé en 777 malgré l'approbation (état : $(stat -c %a "$ACCEPTE" 2>/dev/null || echo absent))"
  echo "passé : $ACCEPTE en 777 après verdict signé"
  rm -f "$ACCEPTE"

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
