# Le moteur local d'Inari : OpenFox dedans, pas à côté

> Écrit en français pour Ben (listé dans `.english-exceptions`). Décidé le 29/10/2026.

## Le but

Un seul processus visible : `inari-core-rs`. Pour le travail en local, tout se passe dans Inari, grâce à une brique
« OpenFox core » portée en Rust : la boucle de l'agent, ses outils, ses agents, ses workflows, MCP. Plus d'OpenFox
qui tourne à côté, plus de pont `openfox-acp`, plus d'interface d'OpenFox ouverte en arrière-plan.

ACP ne sert plus qu'à brancher les agents extérieurs sur Inari : Claude Code, Codex, Gemini, Kimi… Inari devient
le point où l'on branche ces abonnements, et un onglet de l'interface les ajoute et les règle.

OpenFox est sous licence MIT : son code et ses prompts se reprennent, en gardant sa mention de copyright
(`THIRD-PARTY.md`, et en tête des fichiers repris).

## Ce qu'il y a à porter

Le serveur d'OpenFox 2.0.161 fait environ 55 000 lignes de TypeScript (sans les tests).

| Partie d'OpenFox | Lignes | Dans Inari |
|---|---|---|
| Sessions, événements, base, WebSocket, routes | ~12 000 | déjà là : fils, journal, bus, passerelle |
| Boucle de l'agent, client LLM (OpenAI-compatible, streaming, outils, réflexion, nouvelles tentatives) | ~10 000 | à porter |
| Outils : lire, écrire, éditer, shell, chercher, demander, tâches, critères | ~7 000 | à porter |
| Agents, skills, commandes, contexte et compactage, fournisseurs | ~2 500 | à porter (les prompts se reprennent tels quels) |
| Workflows, sous-agents | ~2 200 | à porter |
| Client MCP | ~2 200 | à porter |
| Tableau des tâches, serveur de dev, processus, Git, LSP | ~4 700 | à porter |

L'interface faite aux lots 0 à 3 (onglet Plan, cartes dans le fil, fiche OpenFox, bibliothèque, MCP, réglages)
reste : elle est rebranchée sur le moteur d'Inari au lieu de l'API d'OpenFox.

## L'onglet des abonnements et de leurs harnais

Dans l'interface, sans fichier à modifier :

- **Ajouter un abonnement frontier** sur un cœur : Claude (Claude Code), ChatGPT (Codex), Gemini (Gemini CLI),
  Kimi (Kimi CLI), ou un agent ACP quelconque (sa commande). Le harnais se crée sur le cœur choisi.
- **Pour chacun** : installé ou non (et l'installer ou le mettre à jour), connecté ou non (la connexion se fait
  dans le terminal de l'agent, depuis Inari), ses modèles, ses quotas, le retirer.
- **Le moteur local** (la brique OpenFox) apparaît dans le même onglet : ses fournisseurs (vLLM, Ollama,
  llama.cpp…) et ses modèles.
- Un harnais ajouté ou changé sert au message suivant, sans redémarrer le cœur.

## Les phases

Chaque phase est testée contre de vrais modèles servis en local (pas d'imitation), poussée sur la branche, et
livrée en binaire tout fait (Linux x86_64, statique). Rien ne casse en route : le moteur local arrive comme un
harnais de plus, « Local (Inari) », choisi dans l'interface ; OpenFox reste utilisable jusqu'à la fin de la phase 3.

1. **L'onglet des abonnements et de leurs harnais.** Les harnais deviennent modifiables à chaud (aujourd'hui
   ils sont lus une fois, au démarrage, dans la configuration).
2. **Le cœur de l'agent.** Client LLM local, boucle de l'agent, outils de base, agents planner et builder,
   fournisseurs, compactage du contexte. À la fin, un fil local travaille sans OpenFox.
3. **Workflows, sous-agents, skills, commandes, MCP.**
4. **Tableau des tâches, serveur de dev, processus, Git, diagnostics.** Puis le client OpenFox actuel
   (`src/openfox.rs`) et le pont `openfox-acp` sont retirés.

## En attendant

Les postes tournent sur ce qui marche : le binaire du 28/10 à 18 h 47 (`2867d7d`) avec la configuration
`openfox-acp` d'origine.
