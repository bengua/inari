# OpenFox dans Inari : ce qu'Inari porte déjà, ce qu'il faut ajouter

> Écrit en français pour Ben (listé dans `.english-exceptions`). Fait le 28/10/2026 en comparant l'interface
> d'OpenFox 2.0.161 (`co-l/openfox`, dossier `web/src/components`, environ 250 composants) avec celle d'Inari
> (`crates/inari-core/web/`).

## Le principe

En mode local, l'interface d'Inari porte celle d'OpenFox : on n'ouvre plus l'interface d'OpenFox. Inari en porte
déjà l'essentiel (environ 80 %). Le reste s'ajoute **dans l'interface d'Inari**, avec ses composants à elle, et tous
les réglages se font depuis l'interface, jamais en dur.

Tout ce qui suit dépend d'une chose : qu'Inari parle **directement** à OpenFox (son API HTTP et son WebSocket), au
lieu de passer par le pont `openfox-acp`. Le protocole ACP ne transporte que le texte, les outils et les
permissions : les tâches, les critères, la réflexion, les sous-agents, le contexte, les workflows n'y passent pas.
C'est donc la première étape (lot 0).

Légende : ✅ déjà dans Inari · 🟡 en partie · ❌ à ajouter

## Le fil (la conversation)

| Élément d'OpenFox | Composant(s) OpenFox | Inari aujourd'hui | Lot |
|---|---|---|---|
| Liste des sessions, création | `SessionDropdown`, `CreateSessionModal` | ✅ fils par cœur, « Nouveau fil » | — |
| Messages, rendu Markdown, code | `MessageList`, `ChatMessage`, `AssistantMessage` | ✅ | — |
| Réponse au fil de l'écriture | `StreamingCursor` | ✅ (depuis `9bdba09`) | — |
| Appels d'outils et résultats | `ToolCallDisplay`, `RunCommandView` | ✅ outils repliables | — |
| Confirmations (chemins, commandes) | `CrossSessionConfirmationBanner`, permissions | ✅ approbations | 0 |
| Questions de l'agent (choix, oui/non) | `chat.ask_user` | ✅ carte dans le fil (lot 1) | — |
| Message pendant le travail (file) | `QueuedMessages` | ✅ file d'OpenFox (lot 0), visible et modifiable dans l'onglet Plan (lot 1) | — |
| Arrêter | bouton Stop | ✅ | — |
| Choix du modèle | `ModelPicker`, `ProviderSelector` | ✅ puce modèle | 0 |
| Agent (builder, planner, agents perso) | `AgentSelector` | ✅ puce du haut et onglet Plan (lot 1) | — |
| Niveau de danger / permissions | `DangerLevelSelector` | ✅ onglet Plan, « Confirmations » (lot 1) | — |
| Effort de réflexion | `EffortChangeGate` | ✅ onglet Plan (lot 1) | — |
| Blocs de réflexion | `ThinkingBlock`, `ThinkingSummary` | ✅ dans le fil, en direct (lot 1) | — |
| Liste de tâches de l'agent | `TodoListDisplay` | ✅ onglet Plan (lot 1) | — |
| Critères (contrat) et leur éditeur | `CriteriaEditor` | ✅ onglet Plan, modifiables (lot 1) | — |
| Sous-agents | `SubAgentContainer` | ✅ dans le fil (lot 1) | — |
| Contexte : remplissage, compactage | `ContextPopover`, `DynamicContextPreviewModal` | ✅ anneau, onglet Plan, « Compacter » (lot 1) ; aperçu du contexte dynamique : lot 2 | — |
| Statistiques du tour et de la session | `StatsModal`, `TurnStatsModal`, `Sparkline` | ✅ onglet Plan : dernier tour et totaux de la session (lot 2) ; courbes : non | — |
| Recherche dans les messages | `MessageSearchModal` | ✅ le fil ouvert d'abord, ouverture au message (lot 2) | — |
| Historique des invites | `PromptHistory` | ✅ ↑ et ↓ dans la zone de saisie (lot 2) | — |
| Mention de fichiers avec `@` | `AtMentionAutocomplete` | ✅ liste des fichiers du fil (lot 2) | — |
| Pièces jointes, images, vision | `VisionFallbackItem` | ✅ pièces jointes ; repli vision géré par OpenFox | — |
| Reprendre après une erreur du modèle | `chat.retry`, `chat.llm_retry_now` | ✅ bandeau dans le fil, « Réessayer » suivi (lot 1) | — |
| Session en lecture seule | `ReadonlySessionView` | ✅ pass « supervision » | — |

## Le travail autour du fil

| Élément d'OpenFox | Composant(s) OpenFox | Inari aujourd'hui | Lot |
|---|---|---|---|
| Fichiers du projet, visionneuse | `DirectoryBrowser` | ✅ onglets Artefacts / Visionneuse | — |
| Différences (diff) | `DiffView`, `DiffViewer` | ✅ diff des éditions dans le résultat de l'outil (lot 2) | — |
| Terminal | `TerminalDrawer`, `TerminalPane` | ✅ onglet Terminal | — |
| Branche Git, état, espace de travail | `WorkspaceBranchSection`, `BranchModal`, `WorkspaceModal` | 🟡 branche et fichiers modifiés dans l'onglet Plan (lot 2). Changer de branche : non, voir plus bas | — |
| Serveur de dev, journaux | `DevServerFooter`, `DevServerConfigModal`, `LogViewer` | ❌ | 3 |
| Processus d'arrière-plan | `BackgroundProcesses` | ❌ | 3 |
| Workflows (barre, lancement, paramètres) | `WorkflowBar`, `WorkflowParamModal`, `WorkflowStartedCard` | ❌ | 3 |
| Tableau des tâches du projet | `TasksModal`, `TaskColumn`, `TaskCard`, `GatesEditor` | ❌ | 3 |
| Vue partagée (plusieurs sessions) | `SplitView`, `SessionPane` | 🟡 un fil + l'établi | 4 |
| Projets (créer, ouvrir, supprimer, réglages) | `CreateProjectModal`, `ProjectSettingsModal` | 🟡 un espace de travail par fil | 3 |
| Serveurs MCP | `McpSelector`, `McpServerCard` | ❌ | 3 |

## Les réglages (depuis l'interface, jamais en dur)

| Élément d'OpenFox | Composant(s) OpenFox | Inari aujourd'hui | Lot |
|---|---|---|---|
| Adresse d'OpenFox, connexion (mot de passe) | `PasswordModal` | ❌ (fichier de `openfox-acp`) | 0 |
| Mode de départ, durée max d'un tour | — | ❌ (en dur dans le pont) | 0 |
| Fournisseur et modèle actifs | `ProviderSelector`, `model-list` | ❌ | 0 |
| Gérer les fournisseurs (ajouter, tester) | `ManageProvidersModal` | ✅ fiche OpenFox : tester, ajouter (clé envoyée à OpenFox seulement), relire les modèles, retirer (lot 2) | — |
| Agents, skills, commandes, instructions | `AgentsModal`, `SkillsModal`, `CommandsModal`, `InstructionsModal` | 🟡 commandes `/` listées | 3 |
| Workflows (éditeur, diagramme) | `WorkflowsModal`, `FlowDiagram` | ❌ | 3 |
| Outils et permissions | `ToolsTab` | ❌ | 3 |
| Motifs de nouvelle tentative | `RetryPatternsEditor` | ❌ | 3 |
| Affichage, thème | `DisplayTab`, `ThemeEditor` | 🟡 thème clair/sombre, langue | 4 |
| Raccourcis clavier | `KeybindingsTab` | 🟡 liste des raccourcis | 4 |
| Notifications | `NotificationCenter`, `NotificationSettings` | 🟡 fil d'activité, notifications Android | 4 |
| Plugins | `PluginsTab`, `PluginPanelHost` | ❌ | 4 |
| Premier lancement | `OnboardingWizard` | 🟡 conseil d'accueil | 4 |
| Mises à jour d'OpenFox, journal des versions | `AutoUpdateModal`, `ChangelogModal` | ✅ mises à jour des harnais | — |

## Ordre proposé

- **Lot 0 : OpenFox natif dans le cœur.**
  - Une connexion WebSocket gardée par fil, une reprise après coupure, la fin d'un tour jamais manquée.
  - La file `asap` d'OpenFox pour les messages pendant le travail.
  - Le protocole d'OpenFox décrit par des types Rust.
  - Les réglages du cœur depuis l'interface : adresse, connexion, mode, durée, fournisseur et modèle actifs.
  - Sans ce lot, rien d'autre ne peut passer.
- **Lot 1 : ce qu'on voit pendant un tour local.** Fait (28-29/10/2026).
  - Liste de tâches, critères, réflexion, sous-agents, agent et effort, questions à choix, compactage du contexte,
    reprise après erreur du modèle.
- **Lot 2 : le confort du fil.** Fait (29/10/2026).
  - Diff, branche Git, statistiques par tour, historique des invites, `@` fichiers, recherche dans les messages,
    gestion des fournisseurs.
- **Lot 3 : le travail du projet.**
  - Workflows, tableau des tâches, serveur de dev et processus, MCP, agents, skills et instructions, outils.
- **Lot 4 : la finition.**
  - Vue partagée, thème, raccourcis, notifications, plugins, premier lancement.

Chaque lot est testé (un faux serveur OpenFox dans les tests Rust, des essais dans un vrai navigateur), documenté
et poussé sur la branche avant le suivant.

## Ce qui n'est pas repris, et pourquoi

- **Changer de branche Git, créer un espace de travail.** Inari ne sort le dossier d'un fil sur le disque que
  pendant un tour, puis le range dans le fil (c'est ce qui permet de déplacer un fil d'un cœur à l'autre). Une
  branche changée entre deux tours serait écrasée au tour suivant. L'onglet Plan montre la branche et les fichiers
  modifiés tels qu'OpenFox les annonce ; pour changer de branche, le demander à l'agent dans le fil.
- **Les courbes de statistiques** (`Sparkline`) : les totaux et le dernier tour suffisent ici.

