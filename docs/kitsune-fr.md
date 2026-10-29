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

## Le socle (incrément 1)

Le crate `kitsune` (`crates/kitsune`) tient toute la mémoire dans un seul fichier
SQLite, dont le schéma avance et recule par des migrations numérotées
(`crates/kitsune/migrations`, montée et descente testées une à une) :

- **notes** — ce qui a été dit ou écrit ; d'où ça vient (voix, texte, MCP) ; s'il faut
  y revenir (`needs_review` quand la transcription a échoué) ; ses tags, ses entités
  (dates, lieux, personnes), son lieu.
- **notes_fts** — l'index plein-texte (BM25, accents ignorés), tenu à jour tout seul.
- **embeddings** — le vecteur d'une note, par modèle ; la taille n'est jamais supposée,
  elle vient du modèle réglé dans la GUI.
- **audio_refs** — l'audio gardé pour une note : la capture dont elle vient, ou la voix
  synthétisée depuis elle ; le fichier lui-même est chiffré au repos.
- **triggers** — ce qui ramène une note : une heure, un lieu entré ou quitté, un
  événement du système.

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
`needs_review` : **rien de ce qui a été dit n'est perdu**.

Le serveur de transcription (whisper.cpp, faster-whisper, LocalAI…) est joint en HTTP sur son
endpoint `POST /audio/transcriptions` compatible OpenAI. Son adresse, son modèle et la langue sont
des **réglages de la GUI, jamais en dur** ; il tourne sur la machine ou le réseau privé, donc rien
ne sort de la maison. Ce choix — parler à un serveur local en HTTP plutôt que compiler Whisper en
dur dans le binaire — garde le cœur **pur Rust et statique**, exactement comme le moteur local
parle à vLLM.

La façade `Kitsune` (le cerveau derrière une seule porte) ouvre le store, le coffre audio et lit
ses réglages dans `config.json` ; c'est ce qu'un cœur tient et ce que la GUI configure.

Ce qui suit (voix de synthèse, intégration au cœur et à la page du téléphone) arrive incrément par
incrément

Ce qui suit (voix de synthèse…) arrive incrément par incrément, chacun testé avant le
suivant. Le choix du moteur de voix et le test d'écoute à l'aveugle se font sur les
postes, là où les modèles peuvent être servis.
