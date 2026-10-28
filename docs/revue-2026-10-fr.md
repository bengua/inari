# Revue et corrections d'octobre 2026 : document de passation

> Écrit en français pour Ben, qui relit sans l'agent. Il est listé dans `.english-exceptions`.
>
> **Depuis le lot 7, Inari est Rust only** : le paquet Python, ses tests et `uv` ont quitté le dépôt. Les lots 1 à 6 parlent encore de la passerelle Python (`gateway.py`, `tests/test_*.py`) : ces fichiers n'existent plus, mais leur équivalent Rust est resté. Pour la démo, la page est maintenant dans `crates/inari-core/web/`.
> Branche : `claude/laughing-maxwell-sc9qz4`. Aucune PR n'est ouverte : à toi de la créer après relecture.

## En bref

Six commits sur `claude/laughing-maxwell-sc9qz4`, chacun vérifié (`./verify.sh` : rustfmt, clippy, tests Rust, ruff, 139 tests Python) et essayé dans un vrai navigateur (Chromium), sur la démo Python et sur un vrai nœud Rust avec FoxBot. Seul le lot 5 (Android) n'a pas pu être compilé ici.

Aucune PR n'est ouverte. Pour la créer après relecture : `main` ← `claude/laughing-maxwell-sc9qz4`.

## Comment relire

1. `git log --oneline main..claude/laughing-maxwell-sc9qz4` : un commit par lot, chacun avec un message qui explique le pourquoi.
2. Lancer la démo (Rust) :
   ```sh
   cargo build --release
   target/release/inari-core-rs demo            # ajouter --mascot <dossier de FoxBot> pour voir FoxBot
   ```
   puis ouvrir http://127.0.0.1:8787/#token=demo.
3. `./verify.sh` : formatage, clippy, tests, build. Rust uniquement.
4. Les sections ci-dessous expliquent chaque lot : ce qui a changé, pourquoi, comment le vérifier à la main, et ce qui reste.

## État des lots

| Lot | Sujet | État |
|---|---|---|
| 1 | Fil incrémental, Markdown, erreurs visibles, hors ligne, contrastes, a11y, accept loop | fait, testé |
| 2 | Page découpée en fichiers, polices servies, alternatives visibles aux gestes cachés | fait, testé |
| 3 | Interface web FR/EN | fait, testé |
| 4 | Sécurité de la page : cookie de session, CSP stricte, limites du serveur HTTP, frein aux essais | fait, testé |
| 5 | Android : bord à bord, partage sans saturer la mémoire, messages d'erreur utiles, chaînes | fait, **non compilé** |
| 6 | Timeline incrémentale (la page ne retélécharge plus tout le fil) | fait, testé |
| 7 | **Rust only** : démo, agent de démo, faux agent des tests et signatures en Rust ; capsules Python figées en fixtures ; Python retiré du dépôt et de la CI | fait, testé |

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
| `crates/inari-core/tests/i18n.rs` (Rust) | échoue si un `t("…")` de `app.js` ou un texte de `index.html` n'a pas de français, ou si une traduction perd une variable `{x}` |

- **Changer de langue** : Réglages (l'engrenage) → Langue. La page se recharge, et le brouillon est conservé.
- **Ajouter une langue** : ajouter une entrée dans `LANGS`, puis une table `DICT.xx` sur le modèle de `DICT.fr`. Le test vérifie seulement le français : pour une nouvelle langue, il faudra l'étendre.
- **Ajouter un texte** : écrire `t("Texte anglais")` dans `app.js` et ajouter la ligne dans `DICT.fr`. Le test le rappelle s'il manque.
- **Non traduit, volontairement** : ce que le serveur envoie tel quel (erreurs d'un agent, messages d'un moteur, noms de modèles) et le contenu des fils.
- **Vocabulaire choisi** : thread → fil, core → cœur, harness → moteur, artifacts → artefacts, fork → dupliquer, pass → pass.

## Lot 4 : sécurité de la page

**Avant** : le jeton du propriétaire circulait dans les URL (`/events?token=…`, adresses des logos). Il restait dans le `sessionStorage`, et le pass dans le `localStorage`. La CSP autorisait les scripts inline : une seule injection HTML suffisait pour voler le jeton.

**Maintenant** :

| Sujet | Comment | Où |
|---|---|---|
| Connexion | `POST /api/session {"secret": …}` reçoit le jeton du propriétaire ou un pass (le cœur sait lequel) et répond par un cookie `inari_session` en `HttpOnly` et `SameSite=Strict`, plus `Secure` si la gate annonce HTTPS via `X-Forwarded-Proto`. Le script de la page ne voit jamais ce cookie. | `crates/inari-core/src/session.rs` (nouveau), `gateway.rs` (`sign_in`, `session_access`, `access`) |
| Format du cookie | `v1.<o\|p>.<hash>.<expiration>.<hmac>` : signé, rien n'est stocké côté serveur. La clé est dans `<data_dir>/session.key` (créée au premier lancement, en 0600), donc les sessions survivent à un redémarrage. Durée : 30 jours. | `session.rs` |
| Révocation | La session d'un pass est revérifiée à chaque requête : un pass révoqué coupe l'accès immédiatement. Une session propriétaire contient le hash du jeton : changer le jeton déconnecte toutes les sessions propriétaire. | `session_access` |
| Ordre d'accès | pass présenté en en-tête → jeton présenté → cookie → page ouverte sur la machine elle-même (inchangé). La gate qui ajoute l'en-tête `Authorization` continue de marcher telle quelle. | `access` |
| Anciennes pages et liens | Les liens `#token=…` et `#pass=…`, ainsi que les secrets qu'une ancienne page avait laissés dans le stockage, sont échangés une fois puis effacés. **Un enfant qui avait un pass sur son téléphone n'a rien à ressaisir.** | `connect`, `legacySecret` dans `app.js` |
| Déconnexion | Réglages → Se déconnecter (ou « Quitter ce pass ») appelle `POST /api/logout`. | `signOut` |
| Essais en rafale | 5 mauvais secrets par adresse, puis attente croissante jusqu'à 5 min (`429` avec `retry_after`). Derrière une gate, toutes les requêtes ont la même adresse : le frein est alors global. | `Sessions::wait` / `attempt` |
| CSP | `script-src 'self'` (plus aucun script inline possible), `frame-ancestors 'none'`, `base-uri 'none'`, `object-src 'none'`, `Referrer-Policy: no-referrer`. Les attributs `style=""` restent permis. | constante `CSP` (Rust et Python) |
| Serveur HTTP | 512 connexions au plus (au-delà, fermeture immédiate) ; une requête doit arriver entière en 60 s (fin du slowloris). | `MAX_CONNECTIONS`, `REQUEST_DEADLINE` |
| Python (référence et démo) | Même CSP, sessions propriétaire seulement (la référence n'a pas de pass), avec une clé aléatoire par processus. | `src/inari_core/gateway.py` |

**Compatibilité** : l'en-tête `Authorization: Bearer` marche toujours pour les scripts et outils. `?token=` est encore accepté pour les vieux clients, mais à éviter. À supprimer quand plus rien ne l'utilise.

**Tests ajoutés** :
- `crates/inari-core/src/session.rs` : 4 tests unitaires (signature, falsification, expiration, frein, lecture du cookie) ;
- `crates/inari-core/tests/sessions.rs` : de bout en bout, de la CSP présente aux sessions propriétaire et pass, en passant par le cookie falsifié, la révocation, la déconnexion et le 429 ;
- `tests/test_gateway.py::test_a_page_signs_in_once_and_holds_a_cookie_not_the_token`.

**Vérifié à la main** dans Chromium, sur la démo et sur un vrai nœud Rust avec FoxBot : connexion par lien, par formulaire et par secret hérité ; cookie illisible depuis le JS ; plus aucun secret dans les URL ; déconnexion ; mauvais secret. FoxBot s'affiche dans son iframe malgré la CSP stricte.

### Deux observations sur FoxBot (rien de modifié)

1. **Animations de persona manquantes** : avec la persona `rebel`, `fox/sprites_persona/rebel/{cligne,rit,tourne}/` n'existe pas dans `inari-apk/fox-sprites-persona/`, ce qui donne des 404 (la mascotte se rabat sur une autre pose).
2. **Lien symbolique `sprites_persona`** : `sprite.js` dit que ce dossier est un lien symbolique vers `fox-sprites-persona`. Or `serve_static` (Rust) canonicalise le chemin et **refuse tout fichier dont le chemin réel sort de `mascot_dir`** : un lien symbolique qui pointe ailleurs donne 404. Si ton déploiement repose sur ce lien, les personas ne s'affichent pas avec le cœur Rust. Deux solutions : copier ou monter les sprites dans `mascot_dir`, ou assouplir `serve_static` (refuser les `..` dans le chemin demandé au lieu de comparer les chemins canoniques). C'est à toi de choisir, car ça touche à la protection contre la traversée de répertoires.

## Lot 5 : client Android (`android/` de ce dépôt)

> ⚠️ **Non compilé.** Le SDK Android n'est pas téléchargeable depuis cet environnement : `dl.google.com` est bloqué par le proxy, et l'exécution d'un ktlint téléchargé a été refusée par sécurité. J'ai relu le diff ligne à ligne, mais **lance `android/build.sh` avant de publier**. Les tests JVM (`InariHostTest`) contiennent un nouveau cas qui tournera avec.

| Problème | Correction | Où |
|---|---|---|
| Sur Android 15 (cible 35), l'affichage bord à bord est forcé : la barre de navigation gestuelle pouvait recouvrir la zone de saisie | `navigationBarsPadding()` sur la vue. La page ne reçoit plus que la hauteur que le clavier ajoute au-dessus de la barre (`ime - navigationBars`), sinon le décalage était compté deux fois. | `InariScreen.kt` |
| Partager une grosse vidéo pouvait faire planter l'app (`readBytes()` lisait tout avant de vérifier la taille) | La taille (`OpenableColumns.SIZE`) est lue d'abord ; au-delà de 20 Mo, rien n'est lu. Si la taille est inconnue, on lit au plus 20 Mo + 1 octet (`readNBytesCompat`, car `readNBytes` exige l'API 33). | `sharePayload` |
| L'écran d'erreur affichait `https://…: net::ERR_NAME_NOT_RESOLVED` | Une phrase actionnable selon la cause (réseau privé, cœur arrêté, certificat, autre), en EN et FR, avec le détail technique en petit dessous | `InariHost.hint`, `strings.xml` |
| Invite biométrique en anglais en dur | `R.string.app_name` / `inari_unlock_subtitle` | `Lock.kt` |
| Chaînes héritées de SwarmFox (garde-manger, liste de courses) | Supprimées (elles n'étaient plus utilisées) | `strings.xml` (EN et FR) |

### Ce que je n'ai pas fait à l'aveugle (à faire avec un build sous la main)

1. **R8 / minification** (`isMinifyEnabled = true`, `isShrinkResources = true` dans `android/app/build.gradle.kts`) : APK plus léger et code moins lisible. Il faut tester, car la réflexion et le WebView peuvent demander des règles `proguard-rules.pro`.
2. **Thème Material 3 et écran de démarrage** : remplacer `@android:style/Theme.Material.NoActionBar` (manifeste) par un thème `Theme.Material3.DayNight.NoActionBar`, et ajouter `androidx.core:core-splashscreen` aux couleurs graphite et cuivre. Il faut une nouvelle dépendance et une vérification visuelle.
3. **Envoi natif par morceaux** : aujourd'hui, un fichier partagé passe encore à la page en base64 via `evaluateJavascript` (jusqu'à 20 Mo, soit environ 27 Mo de texte). L'idéal est que l'app envoie elle-même le fichier à `/api/upload` par parties, avec le cookie de session de la WebView (`CookieManager`), puis ne passe à la page que le nom du fichier.
4. **Deux saveurs** (`play` sans `REQUEST_INSTALL_PACKAGES`, `direct` avec mise à jour automatique) si tu vises le Play Store.
5. **Version** : `versionCode`/`versionName` n'ont pas été incrémentés, ça dépend de ton processus de publication (`android/publish.sh`).

**Bon à savoir sur les sessions (lot 4) côté app** : la WebView garde les cookies (`CookieManager`), donc un pass reste connecté entre deux lancements, comme avant avec le `localStorage`. Un téléphone qui avait un pass enregistré par l'ancienne page n'a rien à ressaisir : la nouvelle page l'échange contre un cookie au premier lancement.

## Lot 6 : timeline incrémentale

**Avant** : pendant qu'un agent travaillait, la page retéléchargeait **tout** le journal du fil toutes les ~1,5 s, soit des centaines de Ko sur mobile pour un long fil.

**Maintenant** : la page demande `GET /api/agent/<id>/timeline?after=<dernier id>`. Le serveur répond `{events, from, count}` : seulement la suite, plus l'endroit où elle commence et la longueur totale. La page ajoute la suite si `from` vaut ce qu'elle a déjà et si le total correspond. Sinon, le journal a changé sous elle (fusion, duplication) et elle redemande tout. Sans `after`, ou avec un id inconnu, le serveur renvoie tout le journal, comme avant. Un ancien client continue donc de marcher.

| Où | Quoi |
|---|---|
| `gateway.rs` | `timeline_since()` + test `a_page_gets_only_what_came_since_its_last_event` |
| `gateway.py` | `timeline_since()` + même test |
| `app.js` | `refreshDetail()`. Il ignore aussi une réponse arrivée après qu'on a changé de fil (avant, elle pouvait s'afficher dans le mauvais fil). |

Vérifié dans Chromium : les requêtes portent bien `after=`, et le fil construit au fil de l'eau est identique au fil rechargé de zéro.

---

## Ce qui reste (du rapport de revue, pas traité ici)

Par ordre d'importance pour une version commerciale :

1. **Propriété des fils par métadonnée, et non par préfixe de nom** (rapport §3.2). Aujourd'hui, un fil appartient au pass `abc123` si son nom commence par `abc123-`, avec un id de pass de 6 caractères hex. C'est un risque réel pour la promesse « contrôle parental ». Le chantier touche le format des capsules et les deux implémentations : à décider avec toi.
2. **Installation et premier lancement** (§1) : installeur, assistant de démarrage, appairage du téléphone par QR code, TLS intégré. C'est le plus gros écart avec un produit grand public.
3. **Android** : R8, thème Material 3 et écran de démarrage, envoi natif par morceaux, saveur Play Store (voir le lot 5).
4. **Code** : état typé (`serde` au lieu de `serde_json::Value`), découpage de `gateway.rs` (encore ~1 700 lignes), `tracing` pour les journaux, fin de la double implémentation Python/Rust (§5).
5. **Tests d'interface** : les scénarios Playwright que j'ai utilisés (dans mon espace temporaire, perdus avec la session) gagneraient à entrer dans le dépôt, dans un `tests/ui/` lancé par `verify.sh` sur `inari-core demo`.
6. **Juridique et marque** (§6) : licence MIT (n'importe qui peut revendre), titulaire différent selon les dépôts (Bengua / FreedzHost SASU), logos et personas de tiers, RGPD.
7. **FoxBot** : les deux observations de la fin du lot 4 (animations de persona manquantes, lien symbolique `sprites_persona` refusé par le cœur Rust).

## Lot 7 : Rust only

Consigne de Ben (28/10) : « rust only », sans installer Rust et Python en double. C'est fait, en quatre commits :

| Étape | Ce qui a changé | Où |
|---|---|---|
| Démo | `inari-core-rs demo [--port] [--host] [--token] [--mascot DIR] [--workdir DIR]` : trois nœuds dans un processus, l'interface, des tours en continu et une question d'approbation toutes les 25 s | `crates/inari-core/src/demo.rs` |
| Agent de démo | `inari-core-rs demo-agent` : `write <fichier>: <texte>`, `run: <commande>`. Une commande en lecture seule (`ls`, `cat`, `echo`, `pwd`, `date`, `sleep`, `head`, `wc`, sans tube ni redirection) tourne directement ; tout le reste demande ton accord ; `sudo` ne tourne jamais | `src/demo_agent.rs` |
| Agent des tests | `inari-core-rs fake-agent` : portage exact de `tests/fake_acp.py` (mêmes mots, même journal). `cargo test` ne lance plus `python3` | `src/demo_agent.rs`, tests `crates/inari-core/tests/*.rs` |
| Signatures | `keygen`, `sign --key`, `verify --trusted` passent en Rust. Même format, et **même signature à l'octet près** pour une même clé. Une seule dépendance ajoutée : `ed25519-dalek` | `src/signature.rs` |
| Compatibilité | Une capsule, son archive et une copie signée, écrites par le Python avant son retrait, sont figées en fixtures. Le Rust doit les lire hash pour hash, signaler les dégâts dans les mêmes mots, et vérifier et reproduire leur signature | `tests/fixtures/python/`, `tests/python_capsules.rs` |
| Retrait | `src/inari_core`, `tests/`, `pyproject.toml`, `uv` dans `verify.sh` et la CI, entrées Python du `.gitignore`. La page web a déménagé dans `crates/inari-core/web/`. README, CONTRIBUTING et docs sont mis à jour | |

**Fonctions du Python qui n'avaient pas d'équivalent Rust et disparaissent** (aucune n'était utilisée par les cœurs, qui tournent en Rust depuis l'étape 5 de `docs/rust-port.md`) :
- la boucle de modèle intégrée (fournisseurs OpenAI-compatibles et Anthropic en direct, avec le « handover » entre formats) : les cœurs passent par les moteurs ACP (Claude Code, Gemini CLI, OpenFox) ;
- le bac à sable conteneur des commandes ;
- la génération des schémas JSON depuis les modèles Pydantic : `schema/` reste comme documentation du format ;
- la passerelle autonome (`inari-core gateway --connect`) : le nœud Rust intègre la sienne.

## Installer cette version (pour ton agent)

**Rust uniquement, rien d'autre à installer.** Ne pas installer le paquet Python, ni `uv`, ni de service Python : il n'y en a plus. Si un ancien service Python tourne encore sur une machine, l'arrêter et le désactiver.

```sh
git fetch origin claude/laughing-maxwell-sc9qz4
git checkout claude/laughing-maxwell-sc9qz4
./verify.sh                                   # doit finir par PASSED
cargo build --release                         # ou la build musl statique habituelle
```

Sur chaque cœur :
1. Remplacer le binaire `inari-core-rs` par le nouveau (garder l'ancien à côté, par exemple `inari-core-rs.prev`, pour revenir en arrière).
2. `systemctl --user restart inari-core.service` (le service que `join --service` installe), ou ton équivalent.
3. Rien à migrer : la configuration et le dossier de données ne changent pas. Au premier démarrage, la passerelle crée `<data_dir>/session.key` (en 0600).

**Mettre à jour une installation de cette branche** (après un correctif poussé dessus) :

```sh
git pull origin claude/laughing-maxwell-sc9qz4
./verify.sh && cargo build --release
```

Puis, comme ci-dessus : remplacer `inari-core-rs` (garder le précédent en `.prev`) et redémarrer le service. La page se recharge seule dans les navigateurs et l'app ouverts, car sa version change.

Correctifs poussés depuis la première installation :
- (commit suivant) : les onglets du volet des artefacts sont de nouveau cliquables (la poignée de redimensionnement les recouvrait).
- `23a6460` : bandeaux du haut affinés (36 px), sans rien masquer.
- `f7ac2ab` : choix de l'avatar (flèches autour de la mascotte, choix gardé par personne).
- (commit suivant) : la mascotte retrouve sa place. Volet replié, fil ouvert : en haut au centre de la discussion, à la taille du volet (au lieu de 96 px collée en haut), et le fil commence sous elle. Volet ouvert : dans le volet, comme avant. La version « posée sur le champ de saisie » de `f7ac2ab` est retirée : elle masquait les nouveaux messages.
- **Dépôt `inari-apk`, même branche, `46f98a9`** : FoxBot respecte le choix d'avatar et ne mélange plus jamais les images de FoxBot principal dans une persona. **Il faut redéployer le dossier `fox/`** (`android/app/src/main/assets/fox/`, avec `sprites_persona/` à côté de `sprites/`) là où pointe `mascot_dir`, sinon les flèches changent bien le choix mais l'ancien FoxBot laisse encore le modèle décider. La liste des séquences manquantes et le prompt pour générer : `docs/personas-sequences.md` dans `inari-apk`.
- (commit `3b4707b`) : pendant un glisser, les actions de la mascotte apparaissent en tuiles à côté des autres — « Le résumer », « L'endormir », et « Supprimer… » en rouge, qui demande confirmation.
- `2bf94c0` : à la souris (webapp, navigateur), on peut de nouveau lâcher un fil sur FoxBot ; son iframe ne laissait pas passer la souris.
- `7d7bb92` : sur téléphone, on peut de nouveau lâcher un fil sur FoxBot (il venait se cacher hors de l'écran quand la liste défilait) ; l'astuce sur les gestes n'apparaît plus au doigt.

**À vérifier après installation** :
- La page s'ouvre. La première fois, il faudra peut-être ressaisir le jeton : le cookie de session remplace le jeton en URL. Derrière ta gate qui ajoute l'en-tête, rien ne change.
- Un pass déjà enregistré sur un téléphone reste connecté (il est converti en cookie).
- Réglages → Langue bascule FR/EN ; FoxBot s'affiche.
- Envoyer un message, ouvrir un appel d'outil pendant que l'agent travaille : il reste ouvert.

**APK** : ce dépôt ne change l'APK que par le lot 5, **non compilé ici**. `android/build.sh` d'abord. L'APK n'est pas nécessaire pour tester le reste : il charge la page servie par le cœur.

**Revenir en arrière** : remettre `inari-core-rs.prev` et redémarrer le service. Les sessions ouvertes entre-temps seront simplement redemandées.

## Si quelque chose casse après merge

Chaque lot est un commit indépendant : `git revert <sha>` annule un lot sans toucher aux autres. Seuls les lots 2 et 3 dépendent l'un de l'autre (le 3 ajoute `i18n.js` à la liste créée par le 2). Revenir sur le lot 4 rend à la page son ancien mode de connexion par jeton.
