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

Chaque phase est poussée sur la branche et livrée en binaire tout fait (Linux x86_64, statique). Les tests
tournent contre un faux serveur de modèle qui parle comme vLLM (flux SSE, appels d'outils découpés) ; l'essai
contre un vrai modèle servi en local reste à faire sur les postes (le réseau de l'atelier de développement ne
laisse pas télécharger de modèle).

1. **L'onglet des abonnements et de leurs harnais.** Fait : les harnais sont modifiables à chaud.
2. **Le cœur de l'agent.** Fait : client LLM local, boucle de l'agent, outils, agents, fournisseurs, compactage.
3. **Workflows, sous-agents, skills, commandes, MCP.** Fait :
   - l'onglet Plan d'un fil du moteur local est celui d'OpenFox (critères, tâches, constats, réflexion,
     questions, file des messages, contexte, compactage) ;
   - les sous-agents (explorer, verifier, code reviewer) travaillent dans un contexte à eux et rendent leur
     valeur ; les skills et les commandes `/…` sont lus au format d'OpenFox ;
   - les workflows suivent l'exécuteur d'OpenFox : étapes agent, sous-agent, shell, pour la personne,
     parallèles (l'une après l'autre), transitions, variables `{{…}}`, sous-groupes ;
   - les serveurs MCP (une commande sur le cœur, ou une adresse http du réseau privé) donnent leurs outils aux
     agents ; leurs secrets restent sur le cœur ;
   - la bibliothèque du moteur (agents, skills, commandes, workflows, serveurs MCP, outils) se règle depuis sa
     fiche, avec les mêmes fenêtres que celle d'OpenFox.
4. **Tableau des tâches, serveur de dev, processus, Git.** Fait :
   - le dossier d'un fil reste sur le disque entre deux tours (réglage du moteur) ;
   - tableau des tâches (`.openfox/tasks.json` dans le dossier : il voyage avec le fil), serveur de dev
     (`.openfox/dev.json`, lancé par le cœur, `$PORT` tiré de son adresse), processus en arrière-plan de
     l'agent, branches, instructions du projet ;
   - **forges Git, réglées dans l'interface, rien dans le code** : autant qu'on veut, chacune GitHub ou
     Forgejo/Gitea, son adresse web, le compte qui publie, son jeton (gardé sur le cœur, jamais réaffiché),
     Git par SSH en option (`ssh://git@atelier:2222`, avec les clés du cœur), son API si elle n'est pas
     l'habituelle. Depuis l'onglet Plan d'un fil : cloner un dépôt de la forge dans le dossier, puis pousser
     la branche et ouvrir une pull request. C'est la doctrine de FH Coder : l'agent produit, la personne
     publie ; le jeton ne passe jamais par les commandes du modèle ;
   - OpenFox, le programme à part, reste utilisable comme harnais « héritage » pour qui le fait déjà tourner.
     Le client (`src/openfox.rs`) n'est pas retiré : des postes s'en servent encore.

## Pour la forge de l'atelier

Dans Réglages → Agents et abonnements → le moteur local → Forges Git : nom « Forgejo atelier », type
« Forgejo ou Gitea », adresse web celle de la VM atelier (par exemple `http://atelier:3000`), le compte de
publication et son jeton, et « Git par SSH » si les clones passent par le port SSH de la forge. « Tester »
dit si le jeton est reconnu (« Connecté en tant que … »). GitHub s'ajoute de la même façon (adresse
`https://github.com`).
