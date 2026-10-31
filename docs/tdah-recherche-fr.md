# Kitsune et le TDAH : ce que dit la recherche, et ce qu'on en fait

Ce document fonde les choix de Kitsune sur des faits connus. Il s'appuie sur des recommandations
cliniques, des consensus d'experts, des essais randomisés et des méta-analyses, pas sur des
intuitions. Chaque principe de conception renvoie à sa source et dit **ce qu'elle prouve vraiment**.

> Kitsune n'est ni un dispositif médical ni un traitement. Il ne pose aucun diagnostic et ne
> remplace aucun suivi. Il se situe là où les recommandations placent les « aménagements de
> l'environnement » et le soutien à l'organisation, à côté d'un éventuel accompagnement
> professionnel.

Rédigé le 30 octobre 2026. Les sources sont en fin de document. Quand on n'a lu que le résumé
d'une source, c'est indiqué.

## 1. Ce qui est établi

| Constat | Force de la preuve | Source |
|---|---|---|
| Le TDAH touche environ 5,9 % des jeunes et 2,5 % des adultes. Il est le plus souvent durable et invalidant à l'âge adulte. | Consensus de 80 auteurs de 27 pays, fondé sur des méta-analyses et de très grandes cohortes | Faraone et al. 2021 (WFADHD, 208 conclusions) ; Kooij et al. 2019 |
| Le modèle de référence lie le trouble à un déficit d'**inhibition** et à des fonctions exécutives qui en dépendent. Les preuves les plus solides concernent l'inhibition, la **mémoire de travail**, la **régulation de la motivation** et le contrôle moteur. | Revue théorique très citée | Barkley 1997 (*Psychological Bulletin*) |
| La **perception du temps** est altérée : discrimination de durées (effet moyen, 25 études) et reproduction de durées (26 études, 2 364 participants). | Méta-analyse de 55 études | Marx et al. 2022 (*JAACAP*) |
| La **mémoire prospective** (se souvenir de faire, plus tard) : les tâches déclenchées par **l'heure** sont altérées dans toutes les études, celles déclenchées par **un événement** beaucoup moins. Chez l'adulte, la difficulté majeure est la **planification** de l'intention, plus que son rappel. | Études primaires, pas de méta-analyse trouvée | Fuermaier et al. 2013 ; synthèse 2018 (*ADHD Atten Def Hyp Disord*) |
| Les personnes TDAH réalisent moins leurs propres intentions de la vie réelle, et cette mémoire prospective explique en partie le lien entre TDAH et **procrastination**. | Étude primaire (29 contre 24 adultes) | 2018, même revue |
| La **dysrégulation émotionnelle** est fréquente : environ 70 % des adultes TDAH la rapportent. | Méta-analyse 2020 (auto-déclaration) ; revue Shaw et al. 2014 | BMC Psychiatry 2020 ; Shaw et al. 2014 |
| L'**aversion du délai** et l'attrait de la **récompense immédiate** sont un second chemin, motivationnel, distinct des fonctions exécutives. | Modèle à double voie, données **mitigées** selon les études | Sonuga-Barke 2003 ; revues de la littérature « choice-delay » |

## 2. Ce que recommandent les professionnels

- **NICE NG87 (Royaume-Uni)** : chez l'adulte, les **aménagements de l'environnement** viennent en
  premier. Le traitement médicamenteux est envisagé quand une gêne importante persiste *après* ces
  aménagements. Une intervention psychologique structurée fait partie de l'offre.
- **HAS 2024 (France, enfant et adolescent)** : les interventions **non médicamenteuses**, dont la
  **psychoéducation**, viennent d'abord. La recommandation adulte de la HAS était annoncée pour fin
  2025 ; nous n'avons pas trouvé sa publication, c'est à vérifier sur has-sante.fr.
- **Consensus européen (Kooij et al. 2019)** : des outils de diagnostic et des traitements validés
  existent chez l'adulte. Le trouble est souvent présent toute la vie.
- **TCC pour l'adulte (Safren et al. 2010, *JAMA*, 86 adultes)** : le premier module porte sur
  l'**organisation et la planification**, avec **un seul calendrier et un seul système de listes de
  tâches** et de la résolution de problèmes. Viennent ensuite la réduction de la distractibilité et
  la restructuration cognitive. Les gains par rapport au groupe témoin se maintiennent à 12 mois.
- **Thérapie « méta-cognitive » de groupe (Solanto et al. 2010, *Am J Psychiatry*, 88 adultes)** :
  12 semaines sur la **gestion du temps, l'organisation et la planification**. Les participants
  avaient plus de chances de s'améliorer que dans le groupe de soutien (rapport de cotes 5,4).
- **Intentions de mise en œuvre « si… alors… » (Gawrilow & Gollwitzer 2008)** : chez des enfants
  TDAH, formuler son but en « si X, alors je fais Y » a ramené l'inhibition au niveau des enfants
  sans TDAH (Go/No-Go). D'autres études montrent un effet sur la flexibilité et sur le report de la
  gratification. Attention : ce sont des données **chez l'enfant**.
- **Principes cliniques de Barkley** : agir **au point de performance** (là et au moment où l'action
  doit se faire), **externaliser** l'information, le temps et la motivation. Ces principes viennent
  de son œuvre clinique ; nous les connaissons ici par des **sources secondaires**.
- **Applications numériques** : les revues de 2025 sont prudentes. Les applications de TCC montrent
  un effet modéré sur les symptômes, mais la synthèse de 26 revues systématiques (*BMC Psychiatry*
  2025) juge l'efficacité **non concluante**, faute de preuves de qualité. Une application bien
  acceptée n'a pas amélioré l'observance. Leçon : **ne rien promettre**, et mesurer.

## 3. Principes de conception de Kitsune, reliés aux preuves

Chaque principe indique ce qu'il change dans Kitsune. La colonne « état » dit si c'est déjà
construit (✅), dans les maquettes (🎨) ou à faire (⏳).

| # | Principe | Pourquoi (preuve) | Dans Kitsune | État |
|---|---|---|---|---|
| P1 | **Rien ne doit rester dans la tête** : une pensée se dépose en un geste, à la voix, en moins de dix secondes. | Mémoire de travail (Barkley 1997) ; externaliser l'information (Barkley, secondaire) | Widget « Parler à Kitsune », voix, saisie, partage depuis une autre app | ✅ |
| P2 | **Préférer le déclencheur « événement » au déclencheur « heure »** quand c'est possible. | Mémoire prospective : déclencheur temps altéré, événement préservé (synthèse 2018) | Rappels de lieu (géorepérage) ; « quand j'arrive chez… » ; à étendre : « quand j'ouvre l'ordi », « après le café » (déclencheurs Home Assistant / n8n) | ✅ lieu, ⏳ autres |
| P3 | **Un rappel d'heure ne disparaît pas tout seul** : il reste jusqu'à « Fait » ou « Plus tard ». | Déclencheur temps fragile ; réalisation des intentions réduite (2018) | Bandeau persistant sur la page, notification Android permanente, « Dans 1 h » | ✅ |
| P4 | **Le système planifie avec la personne** : découper, proposer **le plus petit premier pas**, au maximum 3 priorités. | La difficulté majeure est la planification (Fuermaier 2013) ; TCC organisation et planification (Safren 2010, Solanto 2010) | Assistant Organisation, workflow « Plan du jour » | 🎨 ⏳ |
| P5 | **Formuler les intentions en « si… alors… »**. | Intentions de mise en œuvre (Gawrilow & Gollwitzer 2008, chez l'enfant) | L'assistant reformule « appeler le comptable » en « quand je finis le café (9 h), j'appelle le comptable » ; les rappels de lieu sont déjà des « si… alors… » | ⏳ |
| P6 | **Rendre le temps visible** : durées, comptes à rebours, minuteur. Pas seulement des heures abstraites. | Perception du temps altérée (Marx 2022) | « dans 20 min » à côté de « 09:00 », minuteur pour les blocs (« papiers, 30 min »), estimation corrigée par la durée réelle | ⏳ |
| P7 | **Un seul endroit** : un calendrier, un système de listes, pas dix apps. | Premier module de la TCC : un calendrier, un système de tâches (Safren 2010) | Kitsune centralise notes, rappels, tâches, courses et garde-manger ; n8n et Home Assistant alimentent ce même endroit | ✅ base, 🎨 écrans |
| P8 | **Récompense immédiate et visible** : chaque « Fait » est reconnu tout de suite, les progrès se voient. | Motivation et récompense immédiate (Barkley 1997 ; Sonuga-Barke 2003, données mitigées) | Accusé vocal court, compteur de la journée ; **pas** de séries (« streaks ») à perdre | ⏳ |
| P9 | **Jamais de culpabilisation** : ni compteur rouge de retards, ni ton moralisateur. Reporter ne coûte rien. | Dysrégulation émotionnelle chez environ 70 % des adultes (méta-analyse 2020 ; Shaw 2014) | Ton des prompts (assistants, voix) ; un retard se présente comme « on le replace quand ? » | ⏳ |
| P10 | **Agir au point de performance** : le rappel arrive là où l'action se fait, avec ce qu'il faut pour la faire. | Barkley (secondaire) | Le rappel de lieu sonne au magasin avec la liste triée par rayon ; le rappel « appeler » porte le numéro | ✅ lieu, ⏳ contenu |
| P11 | **Capturer la distraction sans la suivre** : une idée qui surgit se note et attend, la tâche en cours continue. | TCC : réduire la distractibilité (Safren 2010) | Capture express pendant une tâche, revue le soir dans le workflow « Inbox » | ⏳ |
| P12 | **Psychoéducation sobre, et renvoyer vers les professionnels.** | Recommandée en premier (HAS 2024, NICE) | Un écran « Comprendre » avec des sources sérieuses et des contacts (HyperSupers-TDAH France, centres experts) ; aucun conseil médical | ⏳ |
| P13 | **Humilité et mesure** : ne promettre aucune efficacité ; mesurer localement ce qui aide. | Preuves des applications non concluantes (BMC Psychiatry 2025) | Indicateurs **locaux et privés** : rappels faits, reportés ou ignorés, délai capture → action ; un bilan hebdomadaire que la personne lit, et que personne d'autre ne voit | ⏳ |

## 4. Conséquences pour les maquettes

Il faut ajuster les maquettes (https://claude.ai/artifact/Xvdb7wKCnQYEwjGWcDS6gE) avant de coder :

1. **Parler** : afficher « dans 20 min » à côté de « 09:00 » (P6). Remplacer « Fait » seul par
   « Fait » plus un accusé immédiat (P8).
2. **Plan du jour** : chaque priorité porte son premier pas *et* une durée estimée (P4, P6), et
   l'assistant la formule en « si… alors… » (P5).
3. **Listes** : un rappel « en retard » ne devient jamais rouge. Il propose « on le replace
   quand ? » (P9).
4. Ajouter un écran **« Comprendre »** (P12) et un **bilan de la semaine** privé (P13).

## 5. Ce qu'on ne fait pas

- Pas de diagnostic, pas de score de « gravité », pas de conseil sur les médicaments.
- Pas de gamification punitive : ni séries à perdre, ni classement.
- Pas de notifications en rafale. Chaque rappel compte ; trop de rappels finiraient ignorés.
  C'est un **choix de conception** de notre part, pas une donnée trouvée dans ces sources.
- Aucune donnée de suivi ne quitte la maison (voir `docs/kitsune-fr.md`, zéro cloud).

## Sources

- Faraone S.V. et al. (2021). *The World Federation of ADHD International Consensus Statement: 208
  evidence-based conclusions about the disorder.* Neurosci Biobehav Rev 128:789-818.
  https://www.adhd-federation.org/publications/international-consensus-statement.html
- Kooij J.J.S. et al. (2019). *Updated European Consensus Statement on diagnosis and treatment of
  adult ADHD.* Eur Psychiatry 56:14-34. https://ddd.uab.cat/record/223450 (résumé lu)
- NICE (2018, mis à jour 2019). *ADHD: diagnosis and management* (NG87).
  https://www.nice.org.uk/guidance/ng87
- HAS (2024). *TDAH chez l'enfant et l'adolescent : recommandations.*
  https://has-sante.fr/upload/docs/application/pdf/2024-09/tdah_enfant_recommandations_mel.pdf ;
  note de cadrage adulte : https://www.has-sante.fr/jcms/p_3302480/
- Barkley R.A. (1997). *Behavioral inhibition, sustained attention, and executive functions:
  constructing a unifying theory of ADHD.* Psychol Bull 121(1):65-94.
  https://pubmed.ncbi.nlm.nih.gov/9000892/
- Marx I. et al. (2022). *Meta-analysis: altered perceptual timing abilities in ADHD.* JAACAP.
  https://eprints.soton.ac.uk/456003/
- Fuermaier A.B.M. et al. (2013). *Complex prospective memory in adults with ADHD.* PLoS ONE.
  https://pmc.ncbi.nlm.nih.gov/articles/PMC3590133/
- Mémoire prospective, mémoire temporelle et procrastination chez l'adulte TDAH (2018). ADHD Atten
  Def Hyp Disord. https://link.springer.com/article/10.1007/s12402-018-0273-x (résumé lu)
- Safren S.A. et al. (2010). *Cognitive behavioral therapy vs relaxation with educational support
  for medication-treated adults with ADHD and persistent symptoms.* JAMA 304(8):875-880.
- Solanto M.V. et al. (2010). *Efficacy of meta-cognitive therapy for adult ADHD.* Am J Psychiatry
  167:958-968. https://pubmed.ncbi.nlm.nih.gov/20231319/
- Gawrilow C., Gollwitzer P.M. (2008). *Implementation intentions facilitate response inhibition in
  children with ADHD.* Cogn Ther Res 32:261-280.
  https://www.socmot.uni-konstanz.de/publications/implementation-intentions-facilitate-response-inhibition-adhd-children
- Shaw P. et al. (2014). *Emotion dysregulation in attention deficit hyperactivity disorder.* Am J
  Psychiatry 171(3):276-293. https://pmc.ncbi.nlm.nih.gov/articles/PMC4282137/
- Dysrégulation émotionnelle chez l'adulte TDAH, méta-analyse (2020). BMC Psychiatry.
  https://bmcpsychiatry.biomedcentral.com/articles/10.1186/s12888-020-2442-7 (résumé lu)
- Sonuga-Barke E.J.S. (2003). *The dual pathway model of AD/HD.* https://eprints.soton.ac.uk/18285/
  ; revue des paradigmes choice-delay : https://pmc.ncbi.nlm.nih.gov/articles/PMC2788754/
- Applications et TCC numérique : *European Psychiatry* 2025,
  https://pmc.ncbi.nlm.nih.gov/articles/PMC12436941/ ; *Frontiers in Public Health* 2025,
  https://pmc.ncbi.nlm.nih.gov/articles/PMC11904913/ ; revue de revues, *BMC Psychiatry* 2025,
  https://doaj.org/article/789ef3c13fd143468097858bb49f61e5
