# Revue et corrections d'octobre 2026 : document de passation

> Écrit en français pour Ben, qui relit sans l'agent. Il est listé dans `.english-exceptions`.
> Branche : `claude/laughing-maxwell-sc9qz4`. Aucune PR n'est ouverte : à toi de la créer après relecture.

## Comment relire

1. `git log --oneline main..claude/laughing-maxwell-sc9qz4` : un commit par lot, chacun avec un message qui explique le pourquoi.
2. Lancer la démo, qui sert la même page que le cœur Rust :
   ```sh
   uv sync --all-extras
   uv run inari-core demo --port 8787 --token demo
   ```
   puis ouvrir http://127.0.0.1:8787/#token=demo.
3. `./verify.sh` : formatage, clippy, tests Rust, ruff, pytest.
4. Les sections ci-dessous expliquent chaque lot : ce qui a changé, pourquoi, comment le vérifier à la main, et ce qui reste.

## État des lots

| Lot | Sujet | État |
|---|---|---|
| 1 | Fil incrémental, Markdown, erreurs visibles, hors ligne, contrastes, a11y, accept loop | fait, testé |
| 2 | Page découpée en fichiers, polices servies, alternatives visibles aux gestes cachés | fait, testé |
| 3 | Interface web FR/EN | fait, testé |
| 4 | Sécurité de la page : cookie de session, JS séparé, CSP stricte, limites du serveur HTTP | à faire |
| 5 | Android : partage par morceaux, bord à bord, thème, chaînes traduites | à faire |

---

## Lot 1 : fil de discussion, Markdown, erreurs, accessibilité

Commit : « Make the web page calmer, clearer and more accessible ».

| Problème | Cause | Correction | Où |
|---|---|---|---|
| Un appel d'outil ouvert se refermait toutes les ~1,5 s ; la sélection de texte disparaissait | `drawChat()` vidait `#msgs` et recréait tout à chaque rafraîchissement | Le fil est construit hors écran puis `reconcile()` ne remplace que les éléments dont la clé (`data-k`, l'id de l'événement) ou la signature (`data-s`, l'id du résultat d'outil) a changé. Un `<details>` ouvert reste ouvert, même quand il passe de *running* à *done*. | `app.js`, fonctions `drawChat`, `reconcile` |
| Les réponses des modèles s'affichaient mal (tableaux en texte brut, pas de listes ni de liens) | Deux mini-moteurs Markdown différents, très partiels | Un seul moteur, `md()` : titres, listes, citations, tableaux, règles, liens http(s) uniquement, code avec langue et bouton *Copy*. Tout est échappé **avant** la mise en forme, donc pas de XSS (testé avec `<img onerror>` et `javascript:`). | `app.js`, `inlineMd`, `mdBlocks`, `md` |
| Les erreurs du cœur étaient ignorées | `api()` faisait `r.json()` sans regarder le statut | `api()` affiche l'erreur (toast). Sur un 401, l'écran de connexion revient avec la raison. Il renvoie toujours un objet, donc les appelants existants ne cassent pas. | `app.js`, `api`, `showLogin` |
| Coupure réseau quasi invisible | Seulement une opacité à 0,6 | Bandeau « Connection to the core lost — reconnecting… » avec l'âge des données affichées. Le flux SSE est rouvert tout seul s'il est fermé. | `openStream`, `online` |
| Contrastes insuffisants (WCAG AA) | `--faint` à 2,8:1, bouton cuivre à 4,39:1 | Nouvelles valeurs, mesurées à ≥ 4,5:1 sur toutes les surfaces | `app.css`, `:root` |
| Petites cibles tactiles, onglets non accessibles | | 44 px sous `pointer:coarse`, `aria-selected` et flèches sur les onglets, `:focus-visible` | `app.css`, `drawBench` |
| Les demandes d'approbation étaient relues à chaque mise à jour, double clic possible | Reconstruites à chaque snapshot avec `role=alert` | Reconstruites seulement si la liste change ; les boutons se désactivent au clic | `renderApprovals` |
| Le brouillon était perdu quand la page se rechargeait pour une nouvelle version | | Sauvé en `sessionStorage`, restauré dans son fil | `saveDraft`, `restoreDraft` |
| Deux forks pouvaient porter le même nom | `Date.now() % 10000` | Suffixe aléatoire de 6 caractères | `forkNow` |
| Le serveur arrêtait d'accepter des connexions à la première erreur `accept` | `while let Ok(..)` | Boucle qui journalise l'erreur et continue | `gateway.rs`, `start` |

**Vérifier à la main** : ouvrir un fil de la démo, déplier un appel d'outil pendant que l'agent travaille (il reste ouvert). Envoyer `run: sleep 2; ls`. Arrêter la démo (le bandeau apparaît), puis la relancer (il disparaît tout seul).

## Lot 2 : page découpée, polices, gestes visibles

| Changement | Pourquoi | Où |
|---|---|---|
| La page est découpée en `index.html` (structure), `app.css`, `app.js` et `fonts/` | Un fichier de 120 Ko avec des lignes de 800 caractères était impossible à relire. C'est aussi le prérequis pour interdire les scripts inline (lot 4). | `src/inari_core/web/` |
| Route `/assets/<fichier>` publique, avec une liste blanche fixe | Les deux passerelles servent les mêmes fichiers. Rust les embarque dans le binaire (`include_bytes!`, liste `ASSETS`), Python les lit dans le paquet (liste `ASSETS`). Un fichier hors liste répond 404, `..` compris. | `gateway.rs` et `gateway.py` |
| La version de la page couvre maintenant aussi CSS, JS et polices | Une mise à jour du seul `app.js` déclenche quand même le rechargement des pages ouvertes. La version est écrite dans `<meta name="inari-page">` ; les URL des fichiers portent `?v=<version>` et sont mises en cache 24 h. | `page_version`, `index.html` |
| Polices servies : IBM Plex Sans/Mono, Bricolage Grotesque | Avant, le navigateur retombait sur Georgia et la police système (seul l'APK avait les polices). Sous-ensemble latin en woff2 : 121 Ko au total, contre 1,2 Mo en TTF. Bricolage est figée en graisse 600, la seule utilisée. Licences OFL jointes. | `web/fonts/`, `@font-face` en tête de `app.css` |
| Flèche « déplacer » sur chaque fil, **+** sur chaque cœur | Avant, ces actions n'existaient que sous forme de geste caché (glisser, appui long, touche T) | `row()`, `renderCores()` |
| Astuce affichée une seule fois sur les gestes | Mémorisée dans `localStorage` (`inari-tip-gestures`) | `renderCores()` |
| `?` liste les raccourcis, `/` ouvre la recherche, `Esc` ferme menus et feuilles | La spec `docs/ux.md` prévoyait `/` | fin de `app.js` |
| Sur un écran de 900 px de haut ou moins, la mascotte et les approbations rétrécissent | La liste des fils était écrasée à environ 100 px de haut | `app.css`, `@media (max-height:900px)` |

**Tests ajoutés** :
- `tests/test_gateway.py::test_the_page_assets_are_public_and_all_listed` : tout fichier référencé par la page existe, et rien d'autre n'est servi ;
- `crates/inari-core/tests/local_page.rs::the_page_and_its_assets_are_served_with_their_version` : même vérification côté Rust, et la marque de version est bien remplie ;
- `crates/inari-core/tests/node.rs` : adapté à la nouvelle place de la version (balise meta).

**Pour régénérer les polices** (si le TTF source change) :
```sh
U="U+0000-00FF,U+0131,U+0152-0153,U+02C6,U+02DA,U+02DC,U+2000-206F,U+20AC,U+2122,U+2190-2199,U+2212"
uvx --from 'fonttools[woff]' pyftsubset android/app/src/main/res/font/ibm_plex_sans.ttf --unicodes="$U" --flavor=woff2 --layout-features='*' --output-file=src/inari_core/web/fonts/ibm_plex_sans.woff2
# Bricolage : d'abord figer la graisse
uvx --from fonttools fonttools varLib.instancer android/app/src/main/res/font/bricolage_grotesque.ttf wght=600 opsz=24 wdth=100 -o /tmp/b.ttf
uvx --from 'fonttools[woff]' pyftsubset /tmp/b.ttf --unicodes="$U" --flavor=woff2 --output-file=src/inari_core/web/fonts/bricolage_grotesque.woff2
```

## Lot 3 : interface en français et en anglais

**Principe.** La clé d'une traduction est le texte anglais lui-même (style gettext) : `t("Move {name} to…", { name })`. Si une traduction manque, l'anglais s'affiche, jamais un trou. Les variables s'écrivent `{nom}` et sont remplies par `t()`.

| Fichier | Rôle |
|---|---|
| `web/i18n.js` | `LANGS` (langues proposées), `DICT.fr` (environ 300 entrées), `LANG` (choix : réglage mémorisé `inari-lang`, sinon langue du navigateur), `t()`, `setLang()`, `translatePage()` |
| `web/app.js` | toutes les chaînes visibles passent par `t()` (147 endroits modifiés) |
| `web/index.html` | reste écrit en anglais ; `translatePage()` traduit au démarrage ses textes, `title`, `aria-label` et `placeholder` avec la même table |
| `tests/test_i18n.py` | échoue si un `t("…")` de `app.js` ou un texte de `index.html` n'a pas de français, ou si une traduction perd une variable `{x}` |

- **Changer de langue** : Réglages (l'engrenage) → Langue. La page se recharge, et le brouillon est conservé.
- **Ajouter une langue** : ajouter une entrée dans `LANGS`, puis une table `DICT.xx` sur le modèle de `DICT.fr`. Le test vérifie seulement le français : pour une nouvelle langue, il faudra l'étendre.
- **Ajouter un texte** : écrire `t("Texte anglais")` dans `app.js` et ajouter la ligne dans `DICT.fr`. Le test le rappelle s'il manque.
- **Non traduit, volontairement** : ce que le serveur envoie tel quel (erreurs d'un agent, messages d'un moteur, noms de modèles) et le contenu des fils.
- **Vocabulaire choisi** : thread → fil, core → cœur, harness → moteur, artifacts → artefacts, fork → dupliquer, pass → pass.
