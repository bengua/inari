# AI-Memory dans Inari : la mémoire partagée des agents

[AI-Memory](https://github.com/akitaonrails/ai-memory) (MIT) est une mémoire long terme pour les
agents de code. Ce qu'un agent a appris dans un projet, le suivant le retrouve, même s'il vient
d'un autre fournisseur : tu quittes Claude Code en plein travail et tu reprends avec Codex, sans
tout réexpliquer. C'est un binaire Rust à part, avec sa propre base. **Inari ne l'embarque pas** :
il le démarre, le vérifie et y branche ses agents.

**AI-Memory et Kitsune** sont deux mémoires distinctes. Kitsune est ton second cerveau personnel
(ce que tu dis, tes rappels, tes listes). AI-Memory est la mémoire de code des agents. Les deux
parlent MCP, et on ne les fusionne pas.

## Installer

1. Installe AI-Memory sur le cœur, selon son propre README. La commande `ai-memory` doit être dans
   le PATH, ou tu indiqueras son chemin.
2. Dans Inari : **Réglages (l'engrenage) → Mémoire partagée → AI-Memory**.
   - **Sa commande** : `ai-memory`, ou son chemin.
   - **Démarré avec** : `serve --transport http`, la commande d'AI-Memory pour son serveur HTTP.
   - **Où il répond** : son adresse en local (par défaut `http://127.0.0.1:49374`).
   - **Son dossier de données** : vide pour le placer dans celui du cœur (`<data_dir>/ai-memory`).
   - **Fournisseur d'embeddings** : vide pour garder celui d'AI-Memory
     (`AI_MEMORY_EMBEDDING_PROVIDER`).
   - **Le démarrer avec le cœur** : si coché, le cœur le relance aussi s'il s'arrête (cinq essais au
     plus).
3. **Démarrer**, puis **Vérifier** : la fiche doit répondre « It answers at … ».

Le cœur crée lui-même le jeton du service (`AI_MEMORY_AUTH_TOKEN`) et le garde dans
`<data_dir>/ai-memory.token`, en mode 600. Il ne s'affiche jamais. Le journal du service est dans
`<data_dir>/ai-memory.log`, et ses dernières lignes apparaissent dans la fiche.

## Brancher les agents

- **Les agents frontier** (Claude Code, Codex, gemini-cli, opencode, qwen-code, cline, copilot-cli,
  kiro-cli) : dans la fiche, **Y brancher un agent**, choisis-le, puis **Brancher**. Le cœur lance
  `ai-memory install-mcp --client <agent> --apply` puis
  `ai-memory install-hooks --agent <agent> --apply`, et affiche leur réponse.
- **Le moteur local** : Réglages → Agents et abonnements → le moteur local → bibliothèque →
  Serveurs MCP → préréglage **AI-Memory** (`ai-memory mcp-bridge`). Ses outils `memory_*`
  deviennent alors ceux des agents.
- **Briefing au début d'un fil** : dans les réglages du moteur local, coche « Un nouveau fil part du
  briefing de la mémoire partagée ». Un fil neuf commence alors avec le résultat de
  `memory_briefing`, limité à 4 000 caractères. Ce n'est pas le cas des tours suivants, ni quand la
  case est décochée. Si AI-Memory ne répond pas dans les 15 secondes, le fil démarre sans briefing.

## Ce qui est garanti

- Local-first : AI-Memory écoute en local, et rien ne part vers le cloud sans ton accord.
- Rien en dur : commande, arguments, adresse, dossier et fournisseur sont des réglages.
- Aucun code C dans le cœur : Inari parle à AI-Memory en HTTP et MCP, et lance sa commande. Le
  binaire du cœur reste statique.

## Tests

- `tests/kitsune_api.rs` le vérifie contre un faux `ai-memory` (un script) :
  - le service démarre avec son jeton, puis « Vérifier » passe ;
  - le branchement de Codex lance bien les deux commandes, et un agent inconnu est refusé ;
  - le jeton est en mode 600 et n'apparaît jamais dans la fiche ;
  - le service s'arrête, et un invité ne voit rien.
- `tests/engine.rs` vérifie que le briefing est bien au début d'un fil neuf, et seulement là.
