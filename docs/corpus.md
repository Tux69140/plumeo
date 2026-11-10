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
- au moins 1 retrait si la série a des zones non nulles ;
- epubcheck sans erreur (si `EPUBCHECK_JAR`).

Propres à un livre (un champ absent ou `null` n'est pas vérifié) :

| Champ | Sens |
|---|---|
| `source_titres` | source des titres réellement utilisée |
| `titres_n1` | nombre exact de titres de niveau 1 (commençant par `prefixe_n1` s'il est donné) |
| `n1_min` | nombre minimal de titres de niveau 1 (même décompte) |
| `n1_max` | nombre maximal de titres de niveau 1, tous confondus |
| `n2` | nombre exact de titres de niveau 2 |
| `titres_contiennent` | textes qu'au moins un titre doit contenir |
| `notes_pages` | `{ "page", "marqueur" }` : au moins une note ancrée sur la page (de ce marqueur) |
| `notes_min` | nombre minimal de notes appariées |
| `sans_notes_pages` | aucune note ancrée sur ces pages |
| `absent_regex` | aucun paragraphe ne vérifie ces motifs |
| `moities` | ancres en `g` et en `d` (double page) |
| `duree_max_s` | durée maximale de conversion (en `--release`) |

`derogations` (`raison`, `mediane_paragraphes_min`, `nombres_isoles_pages`) assouplit
une attente **commune** que le livre ne peut pas tenir, pour une raison mesurée et
consignée ci-dessous ; la raison est affichée à chaque passage du test.

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

## Résultats (2026-11-10)

Conversion en `--release`, epubcheck 5.4.0 : **les 9 EPUB sont valides**, le test passe.

| # | Attendu | Mesuré | Durée |
|---|---|---|---|
| 1 | signets, ~~73~~ **27** n1 dont « VISITE À PATMOS » (voir plus bas) | signets, 27 n1 (« 1 VISITE À PATMOS »…) ; médiane 419 car. | 0,3 s |
| 2 | gras-majuscules, 10 n1 « CHAPITRE », ≤ 20 n1 | gras-majuscules, 10 n1, tous « CHAPITRE » ; 39 n2 ; médiane **35** (dérogation) | 0,2 s |
| 3 | pas de titre courant ; note `*` p. 36 ; 5 à 60 n1 | 0 titre courant ; 7 notes dont `*` p. 36 ; 18 n1 : les 11 chapitres, 2 titres de couverture, 5 fragments de schémas ou d'OCR | 1,1 s |
| 4 | signets, 23 titres | signets, 23 n1, 0 n2 | 0,6 s |
| 5 | signets, 4 n1 et 7 n2 | signets, 4 n1, 7 n2 | 0,3 s |
| 6 | `g` et `d` ; pas de « 92 93 » ; ≥ 1 note appariée | `g` et `d` ; 0 pied résiduel ; 5 notes appariées | 0,2 s |
| 7 | se convertit ; EPUB valide | 30 n1 ; EPUB valide ; schéma p. 6 en texte (plan 6) | 0,5 s |
| 8 | 19 n1 « CHAPITRE » ; pas de note p. 145 | 19 n1 « CHAPITRE » ; aucune note | 0,4 s |
| 9 | ≥ 8 n1 « CHAPITRE » ; pas de note p. 367 ; < 60 s | 9 n1 dont 8 « CHAPITRE … » (+ « PREMIER CHAPITRE ») ; aucune note | 1,2 s |

### Dérogations (attentes communes non tenables)

| # | Attente | Mesure | Cause |
|---|---|---|---|
| 2 | médiane ≥ 80 | 35 car. (pp. 1-98 : 250 ; pp. 99-195 : 31) | Les pp. 99-195 sont un catalogue que le PDF lui-même met une ligne par paragraphe, deux colonnes d'origine entremêlées : rien à reconstituer côté moteur. |
| 3 | pas de nombre isolé | « 0 » p. 37 | Étiquette d'un schéma (scan) : les schémas deviennent des images au plan 6. |
| 7 | pas de nombre isolé | « 1 3 » et « 2 » p. 192 | Étiquettes du schéma des Hiérarchies : plan 6. |

### Attente du n° 1 corrigée : 27 titres et non 73

Le PDF a bien 73 signets de niveau 1, mais **46 d'entre eux sont les ancres des
notes de fin** (« [1] » … « [46] », pp. 134-136). Les 27 autres sont les 17 chapitres
(« 1. VISITE À PATMOS » … « 17. LA CITÉ CÉLESTE ») et les 5 sections de la fin, chacune
en deux signets (« I » puis « La pierre cubique », etc.). Avec 73, l'EPUB avait 46
chapitres d'une ligne. Le moteur ignore désormais les signets « [n] » ; l'attente
passe de 73 à 27.

### Attente du n° 2 : à revoir par le porteur

L'attente « 10 titres de niveau 1 commençant par CHAPITRE » est tenue, mais **ces
10 lignes forment le résumé des pp. 6-11** (« CHAPITRE 1: » … « CHAPITRE 10: »), pas
les ouvertures de chapitre du corps. Dans le corps, les chapitres s'ouvrent par
« LES 4 ÉLÉMENTS » (p. 19), « LA GENÈSE » (p. 32)… qui deviennent des titres de
niveau 2. Dans ce PDF (Word → Calibre), la taille des titres dépend de leur place
sur la page (première ligne 21,2 pt, deuxième 16,9 pt, ailleurs 15 pt) : elle ne dit
rien du niveau. L'EPUB du n° 2 a donc 10 chapitres très courts sur les pp. 6-11 et
tout le corps dans le dernier. Une attente plus juste porterait sur les titres du
corps ; elle demanderait de reconnaître le sommaire sans numéros de page.

## Corrections du moteur tirées du corpus

Chacune a son test unitaire, sans PDF du corpus.

1. **Tiret de fin de ligne** (`pdf.rs`, `paragraphes.rs`) — pdfium rend le tiret qui
   termine une ligne par `\u{2}`, lu comme une espace : « Bétha nie », « sensibi
   lité » (n° 3, 6), « concentrez vous » (Calibre). Le tiret est rendu au mot ; la
   jonction recolle la césure, sauf devant un mot de la liste des composés
   (« -vous », « -là », « -même », « -t-on », « -être »…) ou une majuscule
   (« Saint-Esprit ») où le tiret reste, sans espace. Limite : un composé dont la
   seconde partie est en minuscules et hors liste est recollé (« dramesmystères »).
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
4. **Titres** (`titres.rs`) — (a) niveaux : tailles groupées à 1 pt près (bruit
   d'OCR 16,2-16,9 pt au n° 3) ; le premier groupe d'au moins 3 titres fait le niveau
   1, les tailles plus grandes mais isolées (couverture, schéma, « NOTE DE
   L'ÉDITEUR » 22,5 pt au n° 8) le rejoignent ; (b) au moins 3 titres « CHAPITRE … »
   (ou « PREMIER CHAPITRE ») forment le niveau 1, les autres descendent d'un niveau
   (n° 2, 8, 9) ; (c) un signet retrouvé à l'identique n'est plus limité à 12 mots /
   3 lignes (n° 1 : « 13. LA BÊTE QUI MONTE DE LA MER ET LA BÊTE QUI MONTE DE LA
   TERRE », 16 mots, sur 4 lignes) ; (d) sommaire : des points de suspension dans le
   texte ne suffisent plus, il faut un numéro final après une espace ou des points de
   conduite (n° 2 p. 15 et n° 9 p. 440 étaient pris pour des sommaires) ; (e) les
   signets « [12] » des notes de fin ne sont ni des titres ni un plan (n° 1, 46 signets).
5. **Notes** (`notes.rs`) — (a) appel « * » séparé du mot par une ponctuation et une
   espace (« éthérique. * », n° 3 p. 36) ; (b) appel en chiffres collés au mot,
   exposant lu par l'OCR (« Hartmann2 », n° 6) — seulement si le mot finit par une
   minuscule et qu'une note de ce numéro attend ; (c) notes regroupées : une note
   restée collée à la précédente (« … introduction. 6. Kant : … ») en est détachée si
   son numéro suit le précédent.

Défauts signalés aux revues précédentes et **non observés** sur le corpus : le motif
`DEBUT_NOTE` qui couperait une note sur « 12 mars 1914 » (les dates en début de
paragraphe du n° 7 sont hors zone et hors section de notes). Aucun changement.

## Contrôle visuel (étape 4)

Fait sans interface graphique, sur le contenu des EPUB n° 1, 3 et 6 (décompressés) :

- **Table des matières** : n° 1, 27 entrées après la page de titre (« 1 VISITE À
  PATMOS » … « 17 LA CITÉ CÉLESTE », puis « I », « La pierre cubique » … « V »,
  « L'avènement de la Nouvelle Jérusalem ») ; n° 3, les 11 chapitres, plus 2 titres
  de couverture et 5 fragments de schémas ou d'OCR (« Corps m\-- », « s »… : plan 6) ;
  n° 6, 3 titres seulement (les titres de conférence, au corps du texte et non gras,
  ne sont pas détectés).
- **Notes** : n° 3, 7 notes `<aside epub:type="footnote">`, appel `<a role="doc-noteref">`
  et retour `<a href="#appel-N">↩</a>` ; n° 6, 5 notes. Le n° 1 n'a aucune note
  appariée (notes en fin de livre appelées « [1] », non gérées).
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
    --serie "tests/corpus/series/${n##*:}.json" --sortie /tmp/plumeo-corpus-out
done
flatpak run com.github.johnfactotum.Foliate /tmp/plumeo-corpus-out/<fichier>.epub
```

Points à vérifier : la table des matières ; une note qui s'ouvre puis le retour ↩
(n° 3 : appel après « le corps éthérique. », p. 36) ; l'absence d'en-tête (n° 3) ;
l'ordre de lecture de la double page (n° 6).

## Limites connues (hors attentes)

- n° 1 : notes de fin « [1] »… non appariées (format Calibre d'appels entre crochets).
- n° 3 : les notes dont l'OCR a lu le « * » comme « • » restent du texte avec un point
  à vérifier ; les étiquettes de schémas donnent des titres parasites (plan 6).
- n° 4 p. 3 et n° 5 p. 2 : l'année seule en bas de la page de titre (« 1980 »,
  « 1977 ») est retirée comme numéro de page, avec un point à vérifier.
- n° 6 : la bibliographie qui suit les notes est lue comme une suite de notes sans
  appel (points « note orpheline ») ; les titres de conférence ne sont pas détectés.
- n° 2 : les annonces de l'auteur (« S.v.p. n'hésitez pas… ») que le PDF répète au
  milieu des pages restent dans le texte.
