# Kitsune : le second cerveau vocal d'Inari

Kitsune est le pense-bête vocal d'Inari : on lui parle, il garde la note, et la
retrouve plus tard par les mots ou par le sens, ou la ramène à une heure ou à un
lieu. Le téléphone (inari-apk) porte la page, le micro, la mascotte et les actions
du téléphone ; le cerveau — la mémoire, la transcription, la voix, le serveur MCP —
vit sur un cœur Inari, près du GPU.

## Local-first, strict

Rien ne sort de la maison sans un accord explicite et écrit : ni une note, ni un
audio, ni la voix synthétisée. Aucun appel à un service cloud. Les modèles (voix,
transcription, embeddings) tournent en local. C'est la règle, pas une option.

## L'inspiration Cornelius

Le projet Cornelius (Les Frères Poulain) a inspiré des **idées** — les états de
présence, les cartes de confirmation, la délégation à un réseau d'agents. Cornelius
est sous licence PolyForm Noncommercial, incompatible avec le MIT d'Inari : **aucun
code n'en est repris**. Kitsune est écrit à neuf, dans le style de FoxBot.

## Le socle : `brain.db`

Le crate `kitsune` (`crates/kitsune`) tient toute la mémoire dans **un seul fichier SQLite,
`brain.db`** (WAL, `busy_timeout=5000`), sans aucune base client-serveur. Son schéma avance et
recule par des migrations numérotées (`crates/kitsune/migrations`, montée et descente testées une à
une ; un ancien `kitsune.db` est repris tel quel à l'ouverture) :

- **notes** — une ligne par pensée : `raw_transcript` (ce qui a été dit), `clean_summary` (ce
  qu'il faut en retenir), `category`, `tags` (JSON), le rappel qu'elle porte (`trigger_type`
  `none` | `time` | `geofence`, `due_date` en ISO 8601 UTC, `latitude`/`longitude`/`radius_meters`,
  100 m par défaut), son `status` (`active`, `dismissed`, `pending_transcription`,
  `pending_geocode`, `failed_geocode`, `ambiguous_geocode`…) et `created_at` en ISO 8601 UTC.
- **notes_fts** — l'index plein-texte FTS5 (BM25, accents ignorés) sur le résumé, la
  transcription et les tags, tenu à jour par des triggers.
- **notes_vec** — le vecteur de la note, `FLOAT[384]` (sqlite-vec, distance cosinus), compilé
  dans le binaire statique.
- **notes_geo** — un R*Tree des zones de rappel : « suis-je dans la zone d'une note ? » se
  répond sans parcourir la table, puis la distance exacte (haversine) tranche.
- **audio_refs** — l'audio gardé pour une note ; le fichier lui-même est chiffré au repos.

Supprimer une note purge aussi son texte indexé, son vecteur et sa zone (testé). Sur 10 000
notes, une recherche plein-texte prend environ 1 ms et une recherche par le sens environ 9 ms
(benchmark `ten_thousand_notes_are_searched_within_the_budget`, en release).

## Le pipeline de capture

Une pensée suit toujours le même chemin, qu'elle vienne de la voix, du clavier ou d'un agent par
MCP : **entendue → lue → placée → gardée → comprise**. Chaque serveur est local, réglé dans la GUI,
et facultatif. Sans lui, la note est quand même gardée et rien n'est perdu.

1. **Entendue** : Whisper (whisper.cpp, faster-whisper) répond en `verbose_json`. Si la moyenne
   des `avg_logprob` est sous **-1,0**, ou celle des `no_speech_prob` au-dessus de **0,6**, la
   capture est **refusée comme inaudible** (`AudioInaudible`, HTTP 422). Rien n'est inséré, aucun
   audio n'est gardé, et Kitsune dit simplement « je n'ai pas bien entendu, redis-le ». Les deux
   seuils sont des réglages. Si le serveur est absent, l'audio est gardé et la note attend ses mots
   (`pending_transcription`).
2. **Lue** : un petit modèle (≤ 3B, llama.cpp…) reçoit la transcription. Son JSON est **contraint par
   un schéma** (`response_format: json_schema` strict, que llama.cpp compile en grammaire GBNF) :
   résumé, catégorie (liste réglable), tags, `trigger_type`, moment, lieu. La réponse est lue par
   serde **telle quelle** : un champ en trop, une valeur hors schéma ou des balises markdown, et
   elle est refusée, jamais réparée par regex. Sans modèle, ou s'il échoue, la note est gardée
   telle qu'elle a été dite.
3. **Les dates sont calculées en Rust, pas par le modèle.** Le modèle dit quel jour (décalage ou
   date) et quel moment (matin, midi, après-midi, soir, nuit, ou une heure dite). Rust en fait un
   instant ISO 8601 UTC. « Demain matin » donne le lendemain à **09:00:00 UTC**, et les heures des
   moments de la journée sont des réglages. Une heure déjà passée aujourd'hui glisse au lendemain :
   un rappel ne sonne jamais dans le passé.
4. **Placée** : pour un rappel de lieu, le lieu est cherché sur le géocodeur **auto-hébergé**
   (Nominatim ou Photon, jamais une instance publique).
   - Aucun résultat : pas de rappel, statut `failed_geocode`, tag `erreur:geocodage`.
   - Plusieurs résultats : s'ils désignent le même endroit, c'est lui. Sinon, Kitsune prend le plus
     proche de la dernière position donnée par le téléphone (passivement, jamais demandée). Sans
     position connue, le statut est `ambiguous_geocode`.
   - Géocodeur injoignable : statut `pending_geocode`, et le lieu est réessayé plus tard.
5. **Comprise** : le résumé reçoit son vecteur de 384 dimensions (serveur d'embeddings local, par
   exemple multilingual-e5-small). Un modèle qui n'en donne pas 384 est refusé, pas tronqué.

`Kitsune::retry` (action `retry` de l'API) reprend tout ce qui attendait un serveur : l'audio sans
mots, les lieux pas encore trouvés, les notes sans vecteur.

**Zéro cloud, appliqué dans le code** : chaque adresse est résolue avant l'envoi, et la requête est
refusée si l'adresse n'est pas sur la machine ou un réseau privé (boucle locale, RFC 1918,
link-local, CGNAT/Tailscale, ULA IPv6). Même un réglage qui pointerait vers un service public ne
laisserait sortir ni audio, ni mots, ni vecteurs.

Les tests sur **tes** modèles (les 20 transcriptions en JSON strict, la largeur 384) tournent sur la
machine qui les sert :

```text
KITSUNE_SLM_URL=http://127.0.0.1:8081/v1 KITSUNE_SLM_MODEL=qwen2.5-3b-instruct \
KITSUNE_EMBED_URL=http://127.0.0.1:8082/v1 KITSUNE_EMBED_MODEL=multilingual-e5-small \
  cargo test -p kitsune --release --test live_models -- --ignored --nocapture
```

## Les rappels qui sonnent

L'ordonnanceur du cœur regarde toutes les `tick_seconds` (30 par défaut) quels rappels d'heure sont
dus. Chacun **sonne une seule fois**. Si le cœur était éteint, ce qui a été manqué sonne d'un coup
au réveil. Toutes les `retry_seconds` (300 par défaut), il relance `retry`.

Un rappel qui sonne :

- s'affiche sur la page Kitsune (flux SSE `GET /api/kitsune/events`, réservé au propriétaire) et
  **reste à l'écran** jusqu'à « Done » ou « In 1 h » : pour une personne TDAH, un rappel qui
  disparaît tout seul est un rappel perdu. La page vibre, et elle notifie quand elle est cachée, si
  la permission a été donnée ;
- est envoyé au **webhook local** réglé (n8n, Home Assistant…), en `POST {event:
  "kitsune.reminder", note}`, sur le réseau privé uniquement.

Les rappels de lieu ne sondent jamais le GPS. C'est le **géorepérage du système** du téléphone qui
signale une entrée (action `entered {lat, lon}`). Les rappels de la zone sonnent alors, puis se
taisent pendant `geofence_quiet_minutes` (30 par défaut), pour ne pas sonner à chaque passage
devant la porte. Tous ces réglages sont dans `rings`.

## Sur le téléphone (l'app Inari)

L'app Android ouvre Kitsune derrière l'empreinte, comme tout Inari. Elle ajoute ce qu'une page web
ne peut pas faire seule :

- **Le widget « Parler à Kitsune »** : un tap sur l'écran d'accueil, l'empreinte, et Kitsune écoute
  déjà. Un tap n'importe où garde la note. C'est zéro friction pour une pensée qui disparaît en dix
  secondes.
- **Le micro dans la page** : l'app accorde le micro à la seule origine Inari configurée.
- **Les rappels sonnent même hors réseau.** À chaque ouverture, la page confie ses rappels au
  téléphone (`window.InariPhone`). Les rappels d'heure passent par l'alarme d'Android, exacte si
  c'est permis. Les rappels de lieu passent par le **géorepérage du système** (`GeofencingClient`,
  entrée seulement), et Kitsune ne demande jamais de position. Les notifications sont locales, avec
  « Fait » et « Dans 1 h », et restent affichées jusqu'à ce que la personne agisse. Alarmes et
  zones sont reposées après un redémarrage ou une mise à jour.
- **L'app n'appelle jamais le cœur en arrière-plan**, parce que le mot de passe du gate reste
  derrière l'empreinte. Ce qui est fait depuis une notification est mis en file. La page l'envoie
  au cœur, avec sa propre session, à la prochaine ouverture.

Permissions demandées au moment où elles servent : le micro (à la première capture), les
notifications, et la position « toujours » (seulement quand un rappel de lieu existe).

La CI compile et teste l'APK à chaque push (job `android`). L'APK de debug est gardé 14 jours comme
artefact.

## Les assistants (moteur local)

Le moteur local d'Inari porte l'assistant de Kitsune et ses spécialistes, un par métier. Ils
suivent les principes de `docs/tdah-recherche-fr.md`.

| Agent | Niveau | Ce qu'il fait |
|---|---|---|
| `assistant` | standard | l'assistant du quotidien ; il confie chaque travail au bon spécialiste |
| `triage` | rapide | lit une note, dit ce qu'elle est et où elle va (liste, rappel, métier) |
| `organizer` | standard | organise une journée ou un projet : 3 priorités au plus, avec premier pas, durée et déclencheur « si… alors… » |
| `shopper` | rapide | courses par rayon, garde-manger, ce qui périme, idées repas |
| `tracker` | rapide | suivi perso (sommeil, humeur…) en notes `suivi:…`, bilan de la semaine |
| `librarian` | standard | propose un nom et une place à chaque document ; ne déplace rien lui-même |
| `summarizer` | rapide | résume un texte, un fichier ou des notes, avec leurs actions |
| `home` | standard | maison et connectivité via les serveurs MCP réglés (Home Assistant, n8n) |

**Les niveaux de modèle** (rapide, standard, expert) se règlent dans la GUI : Réglages → Agents
et abonnements → Moteur local. Chaque niveau désigne un modèle d'un serveur. Le petit travail
tourne sur un petit modèle, le travail difficile sur un gros. Un niveau non réglé utilise le
modèle par défaut. Un agent déclare son niveau dans son en-tête (`model: fast`), et une étape de
workflow peut le remplacer.

**L'outil `kitsune`** donne aux agents l'accès au même cerveau que les pages et le téléphone :
chercher, garder une note, étiqueter, poser ou retirer un rappel, et gérer les listes (tâches,
courses, garde-manger : ajouter, cocher, retirer, ce qui périme). Quand l'agent travaille pour
quelqu'un d'autre (un pass), chaque écriture doit être autorisée par le propriétaire.

**Plus loin que le fil** :

- `web_search` passe par **ton SearXNG** (réglage « Recherche web »), jamais par un moteur
  public. `web_fetch` lit une page publique en texte, et refuse les adresses de la machine ou du
  réseau privé. Le spécialiste `researcher` (niveau expert) cite ses sources.
- `delegate` ouvre un **fil Inari** sur un autre harnais (moteur local, Claude Code, Codex…) avec
  un travail complet. Le fil est étiqueté avec le fil qui l'a demandé, et tu le suis dans Inari.
  Par défaut, l'agent demande ton accord avant d'ouvrir le fil. Le spécialiste `developer`
  (niveau expert) écrit le plan et les critères d'acceptation.

**Fournisseurs cloud** (Gemini par son endpoint compatible OpenAI, par exemple
`https://generativelanguage.googleapis.com/v1beta/openai`, avec ta clé d'API) : on les ajoute
comme n'importe quel serveur de modèles, et on les attribue à un niveau (par exemple expert →
Gemini Pro). Le moteur les joint en https par `curl`, comme le reste du cœur : le binaire reste
statique, et ni la clé ni la requête n'apparaissent dans une ligne de commande. **Ce que le cloud
a le droit de toucher** se règle à part. Par défaut, un agent sur un modèle cloud peut garder une
note, lire les rappels et les listes, et les remplir ; il ne peut ni fouiller toutes les notes,
ni lire tes fichiers.

**Les workflows** : `kitsune-inbox` (trie les notes du jour), `daily-plan` (plan du jour, que tu
valides avant qu'il soit écrit), `weekly-review` (bilan de la semaine, privé), `groceries`
(garde-manger et liste de courses), `doc-sort` (rangement de documents, que tu valides avant
tout déplacement), `research-brief` (une question, ses sources, gardée en note), `dev-handoff`
(plan et critères ; une fois validés, un fil s'ouvre sur un harnais coder).

## Le chiffrement au repos (incrément 4)

Les fichiers audio — captures vocales brutes et voix synthétisée, le gros de ce qui est
sensible — sont chiffrés au repos en **AES-256-GCM** (RustCrypto, pur Rust : le binaire reste
statique, aucun C, aucune build OpenSSL). La **clé** vit dans un fichier **mode 600**, lisible
par le seul utilisateur du cœur et créée au premier usage, exactement comme Inari garde ses
autres secrets (`keys.json`, `session.key`) ; son chemin est un réglage, jamais codé en dur.
Chaque fichier est scellé avec un nonce tiré au hasard, et la base ne garde que son chemin
relatif.

La base elle-même (notes, embeddings) est protégée au repos par le **chiffrement de volume
(LUKS)** du cœur, la même posture que les capsules d'Inari. Ce choix évite d'embarquer SQLCipher
et sa build C OpenSSL, qui fragiliserait le binaire statique musl ; il est cohérent avec le reste
du cœur, qui n'a aucune dépendance crypto en C.

## La capture vocale (incrément 2)

Le téléphone capte l'audio ; le cœur le reçoit, le **scelle** (coffre du chiffrement), le fait
**transcrire** par un serveur de reconnaissance vocale local, puis range la note liée à son audio.
Si le serveur ne répond pas ou renvoie une erreur, l'audio est gardé et la note passe en
`pending_transcription` : **rien de ce qui a été dit n'est perdu**.

Le serveur de transcription (whisper.cpp, faster-whisper, LocalAI…) est joint en HTTP sur son
endpoint `POST /audio/transcriptions` compatible OpenAI. Son adresse, son modèle et la langue sont
des **réglages de la GUI, jamais en dur** ; il tourne sur la machine ou le réseau privé, donc rien
ne sort de la maison. Ce choix — parler à un serveur local en HTTP plutôt que compiler Whisper en
dur dans le binaire — garde le cœur **pur Rust et statique**, exactement comme le moteur local
parle à vLLM.

La façade `Kitsune` (le cerveau derrière une seule porte) ouvre le store, le coffre audio et lit
ses réglages dans `config.json` ; c'est ce qu'un cœur tient et ce que la GUI configure.

## La voix de synthèse (incrément 5)

`Kitsune::speak` dit un texte à voix haute : il le fait **synthétiser** par un serveur de voix
local (Qwen3-TTS, CSM, Kokoro…) joint en HTTP sur son endpoint `POST /audio/speech` compatible
OpenAI, **scelle** l'audio reçu (coffre du chiffrement) et le range, lié à une note. La page
s'en sert pour relire une note à voix haute.

Le serveur, la voix, le modèle et le format sont des **réglages de la GUI, jamais en dur** ; il
tourne sur la machine ou le réseau privé, donc la voix ne sort pas de la maison. **Le choix du
moteur et le test d'écoute A/B en français restent sur tes postes**, là où le modèle est servi :
le cœur n'est que le fil jusqu'à lui (même patron HTTP que l'ASR et le moteur local). La fidélité
prime côté serveur ; Inari ne décide pas du modèle à ta place.

## L'intégration au cœur (incrément 8)

Le cœur tient un cerveau Kitsune par machine, ouvert dans `<data_dir>/kitsune/`. La passerelle
l'expose par une API **réservée au propriétaire** :

- `GET /api/kitsune` — son état (réglages, serveurs ASR/TTS prêts, nombres de notes et de rappels
  actifs).
- `POST /api/kitsune {action, …}` — `set_settings`, `capture_text`, `ingest_voice` (audio en
  base64), `list_notes`, `search` (`mode` `hybrid` | `fts` | `semantic`, `limit`, `filters`),
  `speak`, `get_note`, `correct_note`, `delete_note`, `seen_at` (position passive du téléphone),
  `entered` (le géorepérage du téléphone signale une entrée), `retry`, et pour les rappels `set_reminder`
  (`{trigger_type: "time", due_date}` ou `{trigger_type: "geofence", latitude, longitude,
  radius_meters}`), `reminders` (`trigger_type` facultatif), `snooze` (`due_date`) et `dismiss`.
- `GET /api/kitsune/audio?path=` — l'audio scellé, ouvert.
- `GET /api/kitsune/events` — les rappels au moment où ils sonnent (SSE, `event: ring`).

Et **une porte pour le réseau agentique** : `POST /api/kitsune/mcp` reçoit une requête MCP
(JSON-RPC). Le propriétaire y accède toujours ; un autre appelant (n8n, Home Assistant) présente
le **jeton MCP du cerveau** en en-tête `Authorization: Bearer …`. Tant qu'aucun jeton n'est réglé,
la porte est fermée au réseau (le propriétaire seul).

Le serveur MCP (protocole 2025-06-18) offre **exactement quatre outils**, chacun avec son schéma
d'entrée et de sortie (`structuredContent`) :

- `search_notes(query, mode = hybrid | fts | semantic, limit = 5)` — mots (BM25) et sens
  (vecteurs) fusionnés par RRF ;
- `get_active_reminders(trigger_type?)` — les rappels encore à venir, les heures d'abord ;
- `create_note(raw_transcript)` — garder une pensée ;
- `dismiss_reminder(note_id)` — marquer un rappel comme fait.

Rien n'est en dur : adresses, modèles, voix,
jeton et outils offerts sont tous des réglages.

Les appels au cerveau (SQLite, et HTTP vers les serveurs de voix) tournent **hors du runtime
async** (`spawn_blocking`), derrière un `Mutex`, pour ne pas bloquer la passerelle.

