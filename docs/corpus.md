# Corpus de non-régression

Neuf PDF réels servent de tests de non-régression au moteur. **Ils ne sont pas
versionnés** (dépôt public) : le test les lit dans le dossier désigné par
`PLUMEO_CORPUS`. Aucun EPUB n'est versionné non plus.

```bash
PLUMEO_CORPUS=/home/stef/Documents/RAG \
EPUBCHECK_JAR=$PWD/vendor/epubcheck/epubcheck-5.4.0/epubcheck.jar \
  cargo test --release -p plumeo-moteur --test corpus -- --ignored --nocapture
```

Sans `PLUMEO_CORPUS`, le test échoue avec un message explicite. Sans `EPUBCHECK_JAR`,
la validation epubcheck est sautée. Le test affiche une ligne de mesures par livre,
puis ses échecs et ses dérogations.

## Fichiers

- `tests/corpus/manifeste.json` : un objet par livre (`fichier` relatif à
  `PLUMEO_CORPUS`, `serie`, `titres`, `attentes`, `derogations` facultatives).
  Le fichier est retrouvé en comparant les noms en NFC, l'apostrophe ’ ramenée à '.
- `tests/corpus/series/<serie>.json` : réglages de série (`ReglagesSerie`).
- `crates/moteur/tests/corpus.rs` : le test `corpus` (ignoré par défaut).

### Attentes

Communes à chaque livre :
- aucun bloc dont le texte entier est un numéro de page (motif de `zones::est_numero_page`) ;
- longueur médiane des paragraphes entre 80 et 1 500 caractères, maximum ≤ 8 000 ;
- ancres croissantes en (page, moitié) — les définitions de notes, déplacées en fin de
  chapitre par le Markdown interne, ne comptent pas ;
- le Markdown commence par un titre `#` égal au titre du livre (premier bloc, texte
  normalisé) ;
- au moins 1 retrait si la série a des zones non nulles ;
- epubcheck sans erreur (si `EPUBCHECK_JAR`) ; l'EPUB validé a alors autant de liens
  d'entrée de sommaire (`href="chap-…#t-…"` dans les chapitres, hors `nav.xhtml`) que le
  Markdown a d'entrées : une entrée dont le titre visé manque y deviendrait du texte sans
  avertissement.

Propres à un livre (un champ absent ou `null` n'est pas vérifié) :

| Champ | Sens |
|---|---|
| `source_titres` | source des titres réellement utilisée |
| `titres_n1` | nombre exact de titres de niveau 1 (commençant par `prefixe_n1` s'il est donné) |
| `n1_min` | nombre minimal de titres de niveau 1 (même décompte) |
| `n1_max` | nombre maximal de titres de niveau 1, tous confondus |
| `n1_page_min` | aucun titre de niveau 1 avant cette page (titres du corps, pas d'un résumé) |
| `n2` | nombre exact de titres de niveau 2 |
| `entrees_sommaire` | nombre exact d'entrées de sommaire imprimé rendues cliquables (`[texte](#t-…)`) |
| `titres_contiennent` | textes qu'au moins un titre doit contenir |
| `notes_pages` | `{ "page", "marqueur" }` : au moins une note ancrée sur la page (de ce marqueur) |
| `notes_min` | nombre minimal de notes appariées |
| `sans_notes_pages` | aucune note ancrée sur ces pages |
| `absent_regex` | aucun paragraphe ne vérifie ces motifs (les notes, titres, sauts et entrées de sommaire ne sont pas des paragraphes) |
| `moities` | ancres en `g` et en `d` (double page) |
| `duree_max_s` | durée maximale de conversion (en `--release`) |

Le test convertit chaque livre avec le titre que proposerait la CLI (métadonnées du PDF,
nom du fichier). Ce titre, placé en tête du Markdown, est un titre de service : le premier
titre de niveau 1, s'il porte ce titre (texte normalisé), est hors de `titres_n1`, `n1_min`,
`n1_max` et `n1_page_min`. Aucun attendu ne change ; seul l'affichage de Connaissance passe
de n1 1 à n1 0 (son titre « CONNAISSANCE LOGIQUE PENSÉE PRATIQUE », déjà détecté, est le
titre du livre).

Le test lit les blocs du Markdown interne par genre : titre, note (`[^…]:`), saut de
section (`* * *`), entrée de sommaire (`[texte](#t-…)`) ou paragraphe. Un saut n'est ni un
paragraphe ni un titre : il est hors des statistiques de paragraphes (médiane, maximum,
`absent_regex`) ; il porte son ancre comme tout bloc. Les sauts séparent les pages courtes
(page de titre, ISBN, avis, colophon, 4e de couverture) du texte qui suit.

Les entrées d'un sommaire imprimé rendues cliquables (lignes `[texte](#t-…)`) ne sont pas des
paragraphes : elles sont hors des statistiques de paragraphes. Le titre « Table des matières »
ou « Sommaire » de niveau 1 suivi d'une entrée est lui aussi un titre de service, hors des
mêmes décomptes. Chronique de l'Akasha passe ainsi de `titres_n1` 23 à 22 : son « SOMMAIRE »
(signet) mène désormais à 12 entrées cliquables. Approche de la Cité céleste attend 22 entrées
(17 chapitres et les 5 parties du chapitre 17). `entrees_sommaire` est fixé aussi pour
Chronique de l'Akasha (12), Le sens de l'amour (5) et Anges et l'arbre de vie (4, sommaire
d'origine p. 11), valeurs mesurées ; les autres livres n'ont pas d'entrée cliquable.

`derogations` (`raison`, `mediane_paragraphes_min`, `nombres_isoles_pages`) assouplit
une attente **commune** que le livre ne peut pas tenir, pour une raison mesurée et
consignée ci-dessous ; la raison est affichée à chaque passage du test.

Titre du livre déjà détecté : Connaissance (n° 6) commençait par « Rudolf Steiner » (nom de
l'auteur, en gras, p. 1g) avant son titre, déjà détecté comme premier titre `#`. Un premier
titre de niveau 1 égal au titre du livre, dans les 10 premières pages, remonte désormais en
tête de sa (demi-)page comme un titre retrouvé dans les paragraphes : le livre commence par
son titre et la dérogation `titre_livre_apres_couverture` est supprimée. La ligne de titre,
pleine largeur, empêche la p. 1g d'être une page courte : un saut `* * *` laissé à la place
du titre sépare « Rudolf Steiner » (p. 1g) de « Rudolf STEINER » (p. 1d), sans quoi le
raccord des pages les collait.

## Séries

Mesures faites avec `plumeo-cli page <pdf> <N> --serie …` et un relevé des lignes
(centre vertical, taille, gras) de toutes les pages.

| Série | Livres | Réglages | Mesure |
|---|---|---|---|
| `calibre-lettre` | 1, 2, 5, 8, 9 | zones nulles | Calibre ne produit ni titre courant ni numéro de page : premières et dernières lignes de toutes les pages relevées, aucune répétition. |
| `calibre-a5` | 4 | zones nulles | Idem (A5, 252 p.). |
| `distiller-a4` | 7 | zones nulles | Ni en-tête ni pied. Les repères `[2@101]` (pages de l'édition d'origine, 338 occurrences) sont dans le fil du texte : gardés. |
| `prosveta-scan` | 3 | en-tête 0,105 (paire et impaire) | Titre courant (« 112 L'alchimie spirituelle » / « *titre* 113 ») centré entre 0,068 et 0,093 ; première ligne de corps ≥ 0,114. Pas de pied. |
| `omnipage-double` | 6 | `double_page` 0,5 ; pied 0,11 | Pied « 92 93 » centré entre 0,90 et 0,92 ; dernière ligne de corps ≤ 0,87. |

Aucun livre n'a eu besoin d'une autre série.

### Exceptions de page (`prosveta-scan`)

- **p. 4 et 9 : en-tête 0.** p. 4 (adresses de l'éditeur, première ligne à 0,057) et
  p. 9 (préface sans titre courant, texte dès 0,074) perdaient du texte dans la zone
  d'en-tête.
- **Pages à notes de bas de page : zone de notes ajustée à la page** (12, 18, 19, 33,
  36, 38, 53, 57, 109, 112, 123, 125, 156, 160, 170, 174, 186). Sur ce scan le corps
  descend jusqu'à 0,89 sur les pages pleines : une zone de notes commune avalerait le
  corps. La zone part, sur chaque page, à mi-chemin entre la dernière ligne de corps
  (≥ 10 pt) et la première ligne de note (≈ 9 pt).

## Résultats (2026-11-16)

Conversion en `--release`, epubcheck 5.4.0 : **les 9 EPUB sont valides**, le test passe.
Mesures et durées du dernier passage (2026-11-16, vague finale des retours du contrôle
visuel), puis lot « sommaires imprimés » (plan 2).

| # | Attendu | Mesuré | Durée |
|---|---|---|---|
| 1 | signets, ~~73~~ ~~27~~ **17** n1 + 5 n2 dont « VISITE À PATMOS » (voir plus bas) ; 22 entrées de sommaire ; ≥ 46 notes appariées | signets, 17 n1 + 5 n2 (« 1 VISITE À PATMOS »…) ; 22 entrées de sommaire ; 46 notes appariées ; médiane 569 car. | 0,3 s |
| 2 | gras-majuscules, 10 n1 tous à partir de la p. 19, dont « LES 4 ÉLÉMENTS », « LA GENÈSE », « LUCIFER ET LE CHRIST » (attente réécrite, voir plus bas) ; 4 entrées de sommaire | gras-majuscules, les 10 ouvertures du corps (pp. 19, 26, 31, 32, 33, 36, 38, 40, 53, 56) ; 24 n2 ; 48 titres courants retirés ; résumé pp. 6-11 en sommaire, dont p. 11 aux 4 entrées liées ; médiane **32** (dérogation) | 0,2 s |
| 3 | pas de titre courant ; note `*` p. 36 ; 5 à 60 n1 ; **11 entrées de sommaire** ; aucune entrée collée à la suivante | 0 titre courant ; 7 notes dont `*` p. 36 ; 11 entrées de sommaire liées (sommaire de la p. 205, « TABLE DES MATIÈRES » en `#`) ; 14 n1 (les titres illisibles, `# Corps m\--`, `# np`, `# s`…, restent des paragraphes avec le point « texte peu plausible pour un titre (OCR, schéma, code) : non retenu ») ; médiane 282 | 1,2 s |
| 4 | signets, 22 titres (23 moins « SOMMAIRE », titre de service) ; 12 entrées de sommaire | signets, 22 n1, 0 n2, 12 entrées de sommaire ; médiane 603 | 0,8 s |
| 5 | signets, 4 n1 et 7 n2 ; 5 entrées de sommaire | signets, 4 n1, 7 n2, 5 entrées de sommaire ; médiane 456 | 0,3 s |
| 6 | `g` et `d` ; pas de « 92 93 » ; ≥ 1 note appariée ; titre du livre en tête ; **14 n1** dont les titres complets des conférences I et X ; **14 entrées de sommaire** ; aucun numéro romain seul en paragraphe | `g` et `d` ; 0 pied résiduel ; 5 notes appariées ; titre du livre en tête ; 14 n1 (3 parties, 10 conférences, « Ouvrages de Rudolf Steiner… ») + « TABLE DES MATIÈRES » ; 14 entrées liées ; médiane 221 | 0,4 s |
| 7 | se convertit ; EPUB valide | 28 n1 ; sommaire pp. 1-2 reconnu ; médiane 123 ; EPUB valide ; schéma p. 6 en texte (plan 6) | 0,8 s |
| 8 | 19 n1 « CHAPITRE » ; pas de note p. 145 | 19 n1 « CHAPITRE » ; aucune note ; médiane 183 | 0,5 s |
| 9 | ≥ 8 n1 « CHAPITRE » ; pas de note p. 367 ; < 60 s | 9 n1 dont 8 « CHAPITRE … » (+ « PREMIER CHAPITRE ») ; aucune note ; médiane 258 | 1,8 s |

### Dérogations (attentes communes non tenables)

| # | Attente | Mesure | Cause |
|---|---|---|---|
| 2 | médiane ≥ 80 | 32 car. (pp. 1-98 : 223 ; pp. 99-195 : 30) | Les pp. 99-195 sont un catalogue que le PDF lui-même met une ligne par paragraphe, deux colonnes d'origine entremêlées : rien à reconstituer côté moteur. |
| 3 | pas de nombre isolé | « 0 » p. 37, « 00 » p. 106, « 3 » et « 12 » p. 126 | Étiquettes de schémas (scan) : les schémas deviennent des images au plan 6. pp. 106 et 126 : elles étaient avalées par les chaînes de fusion de `raccorder_pages` (corrigées au retour du jalon 1) et ressortent seules. |
| 7 | pas de nombre isolé | « 1 3 » et « 2 » p. 192 | Étiquettes du schéma des Hiérarchies : plan 6. |

### Attentes ajoutées par le lot « sommaires imprimés » (n° 3 et 6)

Mesurées au passage du corpus, puis figées. Chacune échoue sur l'état précédent
(`02bf46c`, essai fait avec le nouveau manifeste) : 0 entrée de sommaire aux n° 3 et 6,
0 titre n1 au n° 6, numéros romains seuls aux p. 13d à 76g.

- **n° 3, `entrees_sommaire: 11`.** Le sommaire de la p. 205 (lignes à points de conduite
  pleine largeur) n'était pas reconnu : 3 paragraphes, dernière entrée recollée au
  colophon de la p. 206. Il donne maintenant ses 11 entrées liées à des titres existants.
  `absent_regex` `\d+ Si vous ne mourez pas` interdit deux entrées collées dans un
  paragraphe (« … 11 Si vous ne mourez pas … 31 », présent dans l'ancien Markdown). Le
  contrôle de liens compare en plus les 11 entrées aux 11 liens de l'EPUB.
- **n° 6, `titres_n1: 14`.** Livre sans signets : les titres de niveau 1 viennent du
  sommaire imprimé (p. 3d). Les 14 sont les 3 parties (CONNAISSANCE, LOGIQUE, PENSÉE
  PRATIQUE), les 10 conférences (« I. LES BASES D'UNE THÉORIE… (I) » à « X. LA CULTURE
  PRATIQUE DE LA PENSÉE ») et « Ouvrages de Rudolf Steiner disponibles en langue
  française ». Le titre du livre et « TABLE DES MATIÈRES » (titres de service) sont
  exclus du décompte. Avant le lot, la navigation n'avait que 3 titres.
- **n° 6, `titres_contiennent`** : le titre complet de la 1re conférence (« I. LES BASES
  D'UNE THÉORIE DE LA CONNAISSANCE RELATIVE À LA THÉOSOPHIE (I) ») et celui de la 10e
  (« X. LA CULTURE PRATIQUE DE LA PENSÉE » ; la 9e, au texte identique, garde « PENSEE »
  sans accent, défaut de l'OCR). Ils prouvent que « I », « LES BASES », « D'UNE THÉORIE… » ne
  restent plus en paragraphes séparés.
- **n° 6, `absent_regex` `^(II|III|IV|V|VI|VII|VIII|IX|X)$`** : aucun numéro romain seul
  en paragraphe (`absent_regex` porte sur les paragraphes du Markdown, titres, entrées et
  notes exclus). « I » est volontairement absent du motif : p. 66d porte un « *I* »
  en italique, légende de schéma (vrai contenu), et la 1re conférence est déjà couverte
  par `titres_contiennent`.
- **EPUB, sommaire en tête (tous les livres).** Quand un chapitre au titre H1
  « Table des matières » ou « Sommaire » (casse et accents ignorés) porte des liens
  internes `#t-…`, le test exige que ce soit `chap-002.xhtml`, juste après la page de
  titre. Les n° 1, 3, 4 et 6 sont concernés ; les autres n'ont pas de tel chapitre.

### Attente du n° 1 corrigée : 27 titres et non 73

Le PDF a bien 73 signets de niveau 1, mais **46 d'entre eux sont les ancres des
notes de fin** (« [1] » … « [46] », pp. 134-136). Les 27 autres sont les 17 chapitres
(« 1. VISITE À PATMOS » … « 17. LA CITÉ CÉLESTE ») et les 5 sections de la fin, chacune
en deux signets (« I » puis « La pierre cubique », etc.). Avec 73, l'EPUB avait 46
chapitres d'une ligne. Le moteur ignore désormais les signets « [n] » ; l'attente
passe de 73 à 27.

**Puis 17 n1 et 5 n2** (retour du contrôle visuel) : les 5 sections de la fin sont des
signets à plat (« I » puis « La pierre cubique »), que le moteur fusionne en « I. La
pierre cubique » et descend d'un niveau (série romaine après des chapitres en chiffres
arabes) : `titres_n1` passe de 27 à 17, `n2` de non vérifié à 5.

**Puis `notes_min` 46** (retour du contrôle visuel) : les notes de fin « [1] » … « [46] »
(pp. 134-136, sans titre « Notes ») sont appariées à leurs appels en exposant « [n] » ;
`absent_regex` `^\[\d+\]$` garantit qu'aucune note ne reste un paragraphe « [n] » isolé.

### Attente du n° 2 réécrite : les chapitres du corps

L'attente du brief (« 10 titres de niveau 1 commençant par CHAPITRE ») visait les
10 lignes « CHAPITRE n: » des pp. 6-11 ; ce sont celles d'un **résumé** sans numéros
de page, pas les ouvertures de chapitre. Décision du contrôleur : les titres
attendus sont ceux du corps. Le moteur reconnaît désormais ce résumé (pages de
lignes courtes dont les titres reviennent plus loin) : il reste du texte, avec un
point « sommaire », et ses paires « CHAPITRE n: » + titre servent de pseudo-signets.
Les 10 titres de niveau 1 tombent sur les ouvertures mesurées : « LES 4 ÉLÉMENTS »
(p. 19), « LES 4 SEASONS DANS LA NATURE » (p. 26), « LES SEPT COULEURS » (p. 31, « LES
7 COULEURS » au résumé), « LA GENÈSE » (p. 32), « LES ANGES DANS LA TRADITION
CABALISTIQUE » (p. 33), « LUCIFER ET LE CHRIST » (p. 36), « MOISES ET L'ARBRE DE VIE »
(p. 38), « L'ARBRE DE VIE » (p. 40), « L'ÉVOLUTION HUMAINE » (p. 53), « D'AUTRES PENSÉES
AU SUGET DES » (p. 56 ; « ANGES », p. 57, reste au niveau 2). Le titre du livre répété
au fil des pages, « LES ANGES ET L'ABRE DE VIE » (48 fois), est retiré comme titre
courant et tracé. Dans ce PDF (Word → Calibre), la taille d'un titre dépend de sa
place sur la page (21,2 / 16,9 / 15 pt) : les niveaux 2 et 3 restent approximatifs.

## Corrections du moteur tirées du corpus

Chacune a son test unitaire, sans PDF du corpus.

1. **Tiret de fin de ligne** (`pdf.rs`, `paragraphes.rs`) — pdfium rend le tiret qui
   termine une ligne par `\u{2}`, lu comme une espace : « Bétha nie », « sensibi
   lité » (n° 3, 6), « concentrez vous » (Calibre). Le tiret est rendu au mot ; le
   **vocabulaire du livre** (mots et composés attestés à l'intérieur des lignes)
   tranche : suite en majuscule → tiret gardé (« Saint-Esprit ») ; mot recollé
   attesté → césure ; composé attesté (« dit-il ») → tiret gardé ; sinon tiret gardé
   seulement devant une suite sans ambiguïté (vous, nous, même(s), ci, là, t, être,
   dessus, dessous, moi, toi, soi), césure dans tous les autres cas. Une première
   version à liste fixe gardait le tiret devant « les », « leurs »… (« subti-les »,
   « cou-leurs » : 16 mots faux au n° 3). Les suites de notes regroupées passent par
   la même jonction (« l'uni- versité », « philo- sophe » au n° 6). Limite : un
   composé jamais écrit dans le livre hors fin de ligne, de suite ambiguë, est recollé
   (« dramesmystères »).
2. **Regroupement en lignes** (`lignes.rs`) — (a) l'OCR donne à des « * » ou « . » un
   rectangle à cheval sur deux lignes, qui servait de référence et fusionnait deux ou
   trois lignes (n° 3 p. 29 et 36) : la ponctuation passe en dernier et rejoint la
   ligne qu'elle chevauche le plus ; (b) un mot de même police au rectangle gonflé
   (« pouvoir‑deux », n° 8, 6 cas) absorbait la ligne du dessus : le rattachement par
   simple chevauchement exige désormais une police plus petite (exposant).
3. **Numéro de chapitre** (`zones.rs`) — le « 1 » en 19,4 pt au-dessus du titre
   (n° 1, 17 chapitres) était retiré comme numéro de page résiduel, et le signet ne
   retrouvait plus son titre. Un nombre plus grand que le texte de la page (≥ 1,5 pt)
   n'est plus un numéro de page.
4. **Titres** (`titres.rs`) — (a) niveaux : tailles enchaînées à 0,5 pt près
   (tolérance du projet), un groupe ne s'étendant pas sur plus d'1 pt (coupé à son
   plus grand écart) : 16, 15 et 14 pt font trois niveaux, le bruit d'OCR 16,1-16,9 pt
   du n° 3 reste groupé ; le premier groupe d'au moins 3 titres fait le niveau
   1, les tailles plus grandes mais isolées (couverture, schéma, « NOTE DE
   L'ÉDITEUR » 22,5 pt au n° 8) le rejoignent ; (b) au moins 3 titres « CHAPITRE … »
   (ou « PREMIER CHAPITRE ») forment le niveau 1, les autres descendent d'un niveau
   (n° 2, 8, 9) ; (c) un signet retrouvé à l'identique n'est plus limité à 12 mots /
   3 lignes (n° 1 : « 13. LA BÊTE QUI MONTE DE LA MER ET LA BÊTE QUI MONTE DE LA
   TERRE », 16 mots, sur 4 lignes) ; (d) sommaire : des points de suspension dans le
   texte ne suffisent plus, il faut un numéro final après une espace ou des points de
   conduite (n° 2 p. 15 et n° 9 p. 440 étaient pris pour des sommaires) ; (e) les
   signets « [12] » des notes de fin ne sont ni des titres ni un plan (n° 1, 46 signets) ;
   (f) résumé sans numéros de page (sans signets) : sommaire, et pseudo-signets
   « CHAPITRE n » pour le niveau 1 (n° 2 ; aussi les tables des pp. 1-2 du n° 7 et
   p. 2 du n° 8) ; les nombres 0 à 20 s'écrivent en lettres pour comparer les titres ;
   (g) titre courant pris dans le texte : un titre candidat qui se trouve **en
   bordure** (premier ou dernier paragraphe de sa page, ou voisin d'un paragraphe
   lui-même répété sur 5 pages, comme l'ancien pied de page « S.v.p. n'hésitez pas… »
   des PDF recomposés) sur au moins 5 pages et 10 % des pages du livre ; « Notes »
   n'en est jamais un. Retrait `titre-courant` et point (n° 2 : 48 occurrences, en
   bordure sur 46 pages de 195 ; 38 sont au milieu de la page, juste après l'annonce
   répétée) ; (h) une date n'est jamais collée
   à un titre voisin (« (7 MARS 1999) LES ANGES… » au n° 2) ; (i) pas de raccord de
   paragraphe à travers un saut de page si l'une des deux pages est un sommaire (les
   lignes du résumé du n° 2 et des tables du n° 7 restent séparées).
5. **Notes** (`notes.rs`) — (a) appel « * » séparé du mot par une ponctuation et une
   espace (« éthérique. * », n° 3 p. 36) ; (b) appel en chiffres collés au mot,
   exposant lu par l'OCR (« Hartmann2 », n° 6) — seulement si le mot finit par une
   minuscule et qu'une note de ce numéro attend ; (c) notes regroupées : une note
   restée collée à la précédente (« … introduction. 6. Kant : … ») en est détachée si
   son numéro suit le précédent ; (d) notes Calibre (n° 1) : appels et marqueurs entre
   crochets « [n] », stockés sans crochets ; section de notes implicite, sans titre
   « Notes », ouverte au paragraphe de la note 1 si les notes 2 et 3 ouvrent des
   paragraphes dans les 8 blocs suivants et si les appels « 1 », « 2 » et « 3 » les
   précèdent ; un « [n+1] »
   collé en fin de note (« … 5e partie. [32] ») ouvre la note suivante ; (e) une
   section de notes (explicite ou implicite) se ferme avant une (demi-)page sans aucun
   marqueur de note (n° 6 : la suite de la note 53 p. 93d, le catalogue p. 94d-95d et
   l'épigraphe p. 96 redeviennent du corps avec leurs ancres, au lieu d'être collés
   aux notes orphelines « 53 » puis « 29 ») ; dans une section implicite, un paragraphe
   sans marqueur ne continue une note que si elle est vide ou ouverte en bas de page.

Défauts signalés aux revues précédentes et **non observés** sur le corpus : le motif
`DEBUT_NOTE` qui couperait une note sur « 12 mars 1914 » (les dates en début de
paragraphe du n° 7 sont hors zone et hors section de notes). Aucun changement.

## Contrôle visuel (étape 4)

Une première relecture (Foliate, Calibre) a donné les retours du jalon 1, corrigés ensuite
sur `plan-1-retours` : sauts `* * *` aux pages courtes, entrées numérotées séparées,
signets à plat fusionnés, notes `[N]` implicites, faux titres rejetés, titre du livre en
tête, sommaire imprimé cliquable, paragraphes espacés. Les sorties à relire sont
régénérées dans `plumeo-controle/` (ignoré par git). Le détail ci-dessous date du premier
passage.

Fait sans interface graphique, sur le contenu des EPUB n° 1, 3 et 6 (décompressés) :

- **Table des matières** (premier passage ; depuis, n° 1 : 17 chapitres et 5 parties en
  `##`, soit 22 entrées liées dans la page « Table des matières » imprimée) : n° 1, 27 entrées après la page de titre (« 1 VISITE À
  PATMOS » … « 17 LA CITÉ CÉLESTE », puis « I », « La pierre cubique » … « V »,
  « L'avènement de la Nouvelle Jérusalem ») ; n° 3, les 11 chapitres, plus 2 titres
  de couverture et 5 fragments de schémas ou d'OCR (« Corps m\-- », « s »… : plan 6) ;
  n° 6, 3 titres seulement (les titres de conférence, au corps du texte et non gras,
  ne sont pas détectés).
- **Notes** : n° 3, 7 notes `<aside epub:type="footnote">`, appel `<a epub:type="noteref">`
  et retour `<a href="#appel-N">↩</a>` ; n° 6, 5 notes. Le n° 1 n'avait aucune note
  appariée (notes en fin de livre appelées « [1] ») ; depuis le retour du contrôle
  visuel, ses 46 notes sont appariées, en fin du dernier chapitre.
- **En-têtes** : aucun « L'alchimie spirituelle » + numéro dans le n° 3 hors de la
  table des matières d'origine ; aucun pied « 92 93 » dans le n° 6.
- **Double page** (n° 6) : ancres `g` puis `d` page après page ; un paragraphe coupé
  au pli se poursuit (p. 40 g → d).

**Contrôle dans Foliate : à faire par le porteur.**

```bash
for n in "omraam/pdf/Aïvanhov, Omraam Mikhaël - Approche de la Cité céleste.pdf:calibre-lettre" \
         "omraam/pdf/L’alchimie_spirituelle_Tome_2_Omraam_Mikhaël_Aïvanhov.pdf:prosveta-scan" \
         "steiner/pdf/Steiner Rudolf - Connaissance Logique Pensée pratique.pdf:omnipage-double"; do
  cargo run --release -p plumeo-cli -- convertir "/home/stef/Documents/RAG/${n%%:*}" \
    --serie "tests/corpus/series/${n##*:}.json" --sortie plumeo-controle
done
flatpak run com.github.johnfactotum.Foliate plumeo-controle/<fichier>.epub
```

Points à vérifier : la table des matières ; une note qui s'ouvre puis le retour ↩
(n° 3 : appel après « le corps éthérique. », p. 36) ; l'absence d'en-tête (n° 3) ;
l'ordre de lecture de la double page (n° 6).

## Limites connues (hors attentes)

- n° 3 : les notes dont l'OCR a lu le « * » comme « • » restent du texte avec un point
  à vérifier ; les étiquettes de schémas donnent des titres parasites (plan 6).
- n° 4 p. 3 et n° 5 p. 2 : l'année seule en bas de la page de titre (« 1980 »,
  « 1977 ») est retirée comme numéro de page, avec un point à vérifier.
- n° 6 : les titres de conférence ne sont pas détectés ; la bibliographie p. 95d,
  redevenue du corps, garde un paragraphe par ligne du scan (« supé- » / « rieurs… »).
- n° 2 : les annonces de l'auteur (« S.v.p. n'hésitez pas… ») que le PDF répète au
  milieu des pages restent dans le texte.
