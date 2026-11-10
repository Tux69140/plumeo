# Pluméo — convertisseur PDF → EPUB (conception)

**Date :** 2026-11-08 · **Dépôt :** `git@github.com:Tux69140/plumeo.git` (public, neuf)
**Statut :** conception validée en brainstorming, à planifier.

Ce document décrit ce que Pluméo doit faire et pourquoi, assez précisément pour qu'un
développeur Rust/TypeScript puisse en tirer un plan sans revenir vers le porteur du
projet. Les décisions **du porteur** sont marquées *(décision)* ; celles prises par
l'assistant pendant la conception sont marquées *(choix de conception)* et restent
contestables ; ce qui est laissé au constructeur est listé à la fin.

---

## 1. Intention

Le porteur possède plusieurs centaines de livres en PDF (spiritualité, ésotérisme :
Omraam Mikhaël Aïvanhov, Rudolf Steiner, Alice Bailey, et d'autres en réserve), de
provenances très hétérogènes : PDF produits par Calibre à partir d'ebooks, scans OCRisés
par Acrobat ou OmniPage, PDF Distiller, doubles pages scannées. Il veut en faire des
**EPUB propres** (et plus tard du Markdown pour un RAG), et surtout **pouvoir vérifier**
que rien ne manque, que les titres sont justes, et que les en-têtes et pieds de page
ont bien disparu.

Les outils existants échouent soit sur la qualité (titres perdus, paragraphes fusionnés,
en-têtes résiduels), soit sur la vérification (aucun ne montre le PDF et le résultat
côte à côte avec des réglages par famille de mise en page). Un essai sur 9 PDF réels
(section 12) l'a confirmé pour les deux scripts maison existants.

**Ce qui rend Pluméo particulier :** les réglages se font **par série** (famille de
mise en page), visuellement, sur les vraies pages ; puis chaque livre se **révise côte
à côte** avant de produire l'EPUB.

## 2. Objectifs, non-objectifs, anti-objectifs

### Objectifs de la v1
- Importer un PDF ou un dossier de PDF ; vérifier chaque PDF avant conversion.
- Ranger les PDF en **séries** ; régler une série visuellement ; contrôler qu'un PDF
  appartient bien à sa série.
- Convertir en un Markdown interne fidèle : titres, paragraphes, notes, tableaux,
  images ; en-têtes et pieds retirés.
- Réviser chaque livre côte à côte (PDF | Markdown) et corriger.
- Produire un **EPUB 3** dans un dossier de sortie configurable.
- Linux (Debian 13) d'abord.

### Prévu, hors v1 (l'architecture ne doit pas l'empêcher)
- **Export Markdown** *(décision)* — le Markdown interne en est la base.
- **Chunks pour RAG**, d'abord pour Open WebUI *(décision)*. Point ouvert : Open WebUI
  redécoupe probablement lui-même les documents ; à vérifier avant de concevoir le format.
- **Windows** *(décision : Linux d'abord, Windows ensuite)*. Aucun choix technique de la
  v1 ne doit l'exclure (pdfium, Tesseract et Tauri existent sous Windows).
- Produire un « PDF cherchable » après OCR ; détection de tableaux par modèle
  (piste : `oar-ocr`, Rust + PaddleOCR/ONNX).

### Anti-objectifs (échecs même si « ça marche »)
- **Perte silencieuse de texte.** Tout ce qui est retiré (en-têtes, pieds, zones exclues)
  doit être visible et vérifiable ; un retrait douteux est signalé, jamais caché.
- **Obliger l'utilisateur à installer des outils** (Tesseract, ocrmypdf, Ghostscript,
  Python, Java…) *(décision : l'OCR doit être intégré)*.
- **Dépendre d'une IA ou d'un service en ligne** pour convertir.
- **Modifier les PDF d'origine.**
- **EPUB à mise en page fixe** (images de pages) : on veut un texte recomposable.
- **Une appli difficile à maintenir** *(décision : maintenabilité à long terme prioritaire)*.

## 3. Contraintes et décisions techniques

| Sujet | Décision |
|---|---|
| Coquille de l'appli | **Tauri 2** *(décision)* — l'ancien Pluméo Electron est abandonné. |
| Moteur de conversion | **Rust** *(décision)*, dans le cœur Tauri (pas de programme compagnon Python). |
| Lecture PDF | **pdfium** via la crate `pdfium-render` (0.9.x, MIT OR Apache-2.0) *(choix de conception)*. PyMuPDF est écarté (AGPL) ; le dépôt est public. |
| OCR | **Tesseract embarqué** dans l'appli (Apache-2.0), modèle `fra` (tessdata_fast 1 Mo ou tessdata_best 3,7 Mo) *(choix de conception)*. Le moteur rend la page avec pdfium, l'envoie à Tesseract et récupère le texte **avec positions** (TSV/hOCR). Pas d'ocrmypdf ni de Ghostscript. |
| Interface | React + TypeScript dans la webview Tauri ; réutilisation des composants de l'atelier Lumosphère (section 11). |
| Plateforme v1 | Linux ; empaquetage AppImage (comme Lumosphère). |
| Dépôt | Nouveau dépôt `Tux69140/plumeo` *(décision)*, **public** : aucun PDF du corpus ne doit y être versionné (droits d'auteur). |

## 4. Parcours utilisateur

```
Import (PDF / dossier)
  └─ Vérification de chaque PDF ── problème ? → action proposée sur la ligne
       └─ Rangement en série (empreinte) ── contrôle d'appartenance
            └─ Réglage de la série (si nouvelle ou dérivée)
                 └─ Conversion → Markdown interne
                      └─ Révision côte à côte (PDF | Markdown)
                           └─ Validation → EPUB dans le dossier de sortie
```

### 4.1 Écran d'import — regroupé par série *(décision : option B)*
- Boutons « + PDF » et « + Dossier » ; **dossier de sortie séparé et configurable**
  *(décision)*, affiché en permanence.
- Les livres sont rangés **sous leur série** ; chaque série indique si son réglage est
  fait. Tant qu'une série n'est pas réglée, ses livres attendent ; une fois réglée, toute
  la série part en conversion.
- Une section « Sans série » reçoit ce qui n'a pas pu être rangé.
- Chaque ligne de livre porte son diagnostic et l'action correspondante (4.2), et son
  état : en attente / bloqué / à convertir / conversion… / à réviser / validé / EPUB produit.

### 4.2 Vérification à l'import
| Cas détecté | Action proposée |
|---|---|
| Aucune couche texte (scan brut) | **« Lancer l'OCR »**, puis la conversion reprend *(décision)*. Le livre est bloqué d'ici là. |
| Couche texte de mauvaise qualité | **« Refaire l'OCR »** (sur tout le livre ou sur les pages signalées). Mesure **par page** (voir 12.3 : une mesure globale ne discrimine pas). |
| Doubles pages scannées | **« Couper en 2 »** au pli (position réglable dans le réglage de série). |
| Schémas, tableaux, images | Signalés pour la révision. |

Les noms de fichiers sont comparés après **normalisation Unicode NFC** : le corpus
contient des noms identiques à l'écran mais encodés en NFC ou NFD.

### 4.3 Séries
Une **série** = une famille de **mise en page**, indépendante de l'auteur *(décision : le
sens convient)*. Exemple réel : 5 livres Calibre de trois auteurs partagent la même mise
en page.

- **Empreinte** (pour *proposer* une série) : outil producteur, format et proportions de
  page, taille du texte courant, simple/double page, marges du bloc de texte.
  L'outil producteur n'est qu'un indice *(décision : « ce n'est pas parce qu'il a été
  produit par Calibre qu'il a les mêmes caractéristiques »)*.
- **Contrôle d'appartenance** *(décision : validé)* — c'est lui qui décide, à l'entrée
  d'un PDF dans une série et à la demande sur toute la série. Les réglages de la série
  sont appliqués à ~15 pages réparties dans le livre, et on mesure :
  - format : proportions, simple/double page ;
  - bloc de texte : marges haut/bas/côtés (paires et impaires) dans une tolérance ;
  - zones : aucune ligne de texte courant coupée par une zone d'en-tête/pied ; ce qui est
    retiré se répète bien d'une page à l'autre ;
  - texte courant : même taille, même interligne ;
  - colonnes : même nombre, mêmes positions ;
  - titres : la règle de la série en trouve un nombre plausible.
  Résultat : une **fiche par PDF** (✓/⚠ par critère, avec la raison chiffrée, ex.
  « marge basse 9 % contre 11 % pour la série ») et une **planche** de pages du PDF avec
  les zones de la série superposées.
- En cas d'échec : rattacher quand même, **créer une série dérivée** (copie des réglages
  à ajuster) *(décision : « série dérivée c'est bien »)*, ou créer une nouvelle série.
- On peut déplacer un livre d'une série à l'autre, renommer et fusionner des séries.
- **Premier import d'un dossier sans série existante** : Pluméo **regroupe lui-même** les
  PDF par empreinte et propose des séries nommées provisoirement (ex. « calibre · Lettre ·
  15 pt »), que l'utilisateur confirme, renomme, scinde ou fusionne. Un PDF qui ne
  ressemble à rien va dans « Sans série ».

### 4.4 Écran de réglage d'une série *(décision : option A + navigation)*
- **Gauche :** liste des séries (+ nouvelle série).
- **Centre :** une grande page du PDF, zones superposées et **manipulables à la souris**
  (tirer les bords). **Défilement libre** dans les pages (molette, flèches) et **champ de
  saisie d'un numéro de page** *(décision)*. Bascule **paire / impaire** : chaque parité
  a ses zones.
- **Droite :** valeurs chiffrées des réglages et **texte extrait de la page affichée**,
  mis à jour en direct.
- **Réglages d'une série :**
  - zone **en-tête** (retirée), zone **pied** (retirée), zone **notes** (gardée et
    déplacée en notes) — par parité ;
  - **colonnes** : 1, 2 ou auto ; la mise en page peut **alterner** sur une même page
    une partie pleine largeur et une partie en plusieurs colonnes *(décision)* ;
  - **double page** : coupe au pli (position réglable) ;
  - **règle des titres en cascade** *(choix de conception)* : signets du PDF s'ils
    ressemblent à des chapitres → sinon gras + MAJUSCULES → sinon taille de police ;
    avec possibilité de **forcer** la règle pour un livre.
- **Exceptions par page** *(choix de conception)* : sur une page atypique (ouverture de
  chapitre, page de tableau), des zones propres à cette page, sans toucher la série.

### 4.5 Écran de révision *(décision : option B, Markdown brut par défaut)*
- **Gauche :** le PDF (zoom, numéro de page, recherche). **Droite :** le **Markdown
  brut** éditable, avec repères de page visibles. **Défilement synchronisé** ; le passage
  sous le curseur est surligné des deux côtés, **au niveau du paragraphe (bloc)**, pas
  seulement de la page — c'est ce que montrait la maquette validée. Le défilement
  proportionnel de Lumosphère (`useScrollSync.ts`, sans ancres) ne suffit donc pas : la
  synchro s'appuie sur les ancres de bloc (section 7).
- Bascule **Liseuse** pour voir le rendu EPUB. *(Choix de conception : en lecture seule
  dans la v1.)*
- Liste des **points à vérifier** (⚑) générée par Pluméo : titres suspects (ex. un
  titre courant pris pour un titre de chapitre), pages au texte douteux, tableaux
  reconstruits, schémas gardés en image, notes orphelines. Un clic amène aux deux endroits.
- On peut tracer sur le PDF une zone **« image »** ou **« tableau »** pour corriger la
  détection.
- **Validation** du livre → génération de l'EPUB. L'EPUB se régénère depuis le Markdown
  à chaque validation.
- **Un livre traité n'est plus touché** *(décision : « si j'ai déjà traité un livre,
  pourquoi revenir dessus ? »)*. Dès qu'un livre est ouvert en révision, les changements
  ultérieurs de réglages de sa série ne s'appliquent plus à lui : ils ne valent que pour
  les livres pas encore révisés. Reconvertir un livre traité reste possible, mais
  seulement à la demande, avec un avertissement : les corrections faites à la main sont
  perdues. Pas de fusion entre l'ancienne et la nouvelle conversion.

## 5. Moteur de conversion (Rust)

Étapes, page par page puis livre entier :
1. **Lecture** : glyphes/mots avec boîtes, polices, tailles, gras, via pdfium ; ou, pour
   un livre OCRisé par Pluméo, mots avec boîtes issus de Tesseract.
2. **Découpe de double page** si réglée.
3. **Zones** : retrait en-tête/pied, extraction des notes, exceptions de page.
4. **Colonnes et ordre de lecture**, y compris alternance pleine largeur / colonnes.
5. **Lignes → paragraphes** : recollage des césures, **soft hyphens (U+00AD)**,
   paragraphes coupés au saut de page (paragraphe précédent sans ponctuation finale),
   dernière ligne d'un paragraphe repérée par le blanc à droite (règle validée dans
   Lumosphère : > 15 % de la largeur du bloc).
6. **Titres** (cascade 4.4), fusion des titres multilignes, saut du sommaire.
7. **Tableaux** → reconstruction en **vrai tableau HTML** *(décision : « l'idéal serait un
   vrai tableau HTML »)*. Aucune reconstruction n'étant garantie sans erreur, chaque
   tableau reconstruit est un point à vérifier en révision, et peut être basculé en image.
8. **Schémas et illustrations** → **images** *(choix de conception)*. Exemple réel :
   Bailey, *Lettres sur la méditation occulte*, p. 6 — un schéma tracé avec des cadres qui
   **ne doit pas** devenir un tableau.
9. **Notes** : appel de note dans le texte ↔ note (voir 6). Deux origines à gérer
   dès la v1 : notes **en bas de page** (zone « notes » de la série) et notes
   **regroupées** sous un titre « Notes » en fin de chapitre ou de livre (constaté dans
   #6). Les appels sont appariés aux notes par numéro ; un appel sans note ou une note sans
   appel est un point à vérifier.
10. **Mise en forme dans les paragraphes** *(choix de conception)* : italique, gras et
   exposants (appels de note) **conservés** pour les PDF à couche texte (les polices
   italiques et grasses sont présentes dans le corpus). Pour les livres OCRisés par
   Pluméo, Tesseract ne donne pas l'italique de façon fiable : ils n'en auront pas.
11. **Langue** *(choix de conception)* : français en v1 ; la langue est un réglage de
   série (défaut `fra`) qui choisit le modèle Tesseract, le dictionnaire de mesure de
   qualité et le `dc:language` de l'EPUB. Ajouter une langue = ajouter un modèle et un
   dictionnaire, sans reconvertir les autres livres. Le dictionnaire système n'existant
   ni dans l'AppImage ni sous Windows, un dictionnaire à licence compatible est embarqué.
12. Écriture du **Markdown interne** (section 7).

## 6. EPUB produit
- **EPUB 3** recomposable, un fichier par chapitre (titre de niveau 1), table des matières
  (nav) tirée des titres.
- **Notes** *(décision)* : appel de note en `epub:type="noteref"` vers la note en
  `epub:type="footnote"`, regroupée **en fin de chapitre**. Les liseuses compatibles
  l'ouvrent en **fenêtre surgissante** ; chaque note porte en plus un **lien retour ↩**
  vers son appel, pour revenir à la position de lecture sur les liseuses qui sautent à la
  note.
- **Métadonnées** *(choix de conception)* : titre et auteur proposés depuis les
  métadonnées du PDF, à défaut depuis le nom du fichier, toujours modifiables (elles sont
  souvent fausses : « Microsoft Word - F - Les Anges et l'Abre de Vie », « Livres AAB »).
- **Couverture** *(choix de conception)* : rendu image de la page 1, remplaçable.
- Tableaux en HTML, images embarquées.
- L'EPUB doit passer **epubcheck** sans erreur.

## 7. Markdown interne (format pivot)
Le Markdown est le format de travail : la révision l'édite, l'EPUB en est généré, et
l'export Markdown (hors v1) le livrera. Il doit donc porter sans perte ce dont l'EPUB a
besoin : titres hiérarchisés, **repères de page** (pour les chunks RAG futurs : source +
page + chemin de titres), notes, tableaux, images, italique/gras/exposants.
Chaque **bloc** (paragraphe, titre, tableau, image, note) porte une **ancre** : page du
PDF (et demi-page en cas de double page coupée) et rectangle dans la page. Ces ancres
servent la synchro au paragraphe, les points à vérifier et les zones tracées en
révision ; elles doivent **survivre à l'édition** du Markdown (un bloc modifié garde son
ancre, un bloc ajouté hérite de celle de son voisin).
La syntaxe exacte est laissée au constructeur (section 14), sous ces conditions.

## 8. Stockage
Réglages des séries, état de chaque livre (étapes franchies, Markdown révisé, exceptions
de page) : persistés localement, hors du dossier de sortie, et survivant à un
redémarrage. Les PDF d'origine ne sont jamais copiés ni modifiés ; un livre OCRisé par
Pluméo garde son résultat d'OCR à part.

## 9. Performance (ordre de grandeur attendu)
- Le corpus visé compte des centaines de PDF ; un livre de 768 pages en fait partie.
- La conversion d'un livre à couche texte doit se compter en secondes, pas en minutes
  (référence : `pdf-to-ebook` annonce 6,7 s pour 419 pages).
- L'OCR est lent par nature : il tourne en arrière-plan, avec progression, et
  l'interface reste utilisable.

## 10. Environnement de développement constaté (Debian 13)
- GPU AMD Radeon RX 6600 — **sans usage** : aucun traitement n'exige de GPU.
- Présents : `pdftotext`/poppler-utils, Calibre (Flatpak, `ebook-convert` 9.17),
  Sigil et Foliate (Flatpak) pour contrôler les EPUB, `libtesseract5` (bibliothèque
  seule), Java 21, Rust (cargo).
- Absents : commande `tesseract` et données `fra`, ocrmypdf, pandoc. Pour le
  développement, `sudo apt install tesseract-ocr tesseract-ocr-fra` ; dans l'appli
  livrée, Tesseract est embarqué.

## 11. Existant réutilisable
| Source | Quoi | Comment |
|---|---|---|
| **Lumosphère** (`/home/stef/Documents/Lulu/lumosphere`, licence MIT, même auteur) | Révision côte à côte : `src/features/atelier/components/{RevisionLayout,PdfPane,PdfToolbar,VoletPdf}.tsx`, `useScrollSync.ts`, `usePdfZoom.ts`, `usePdfRecherche.ts`, `useHauteurVolets.ts`, `useLargeurVolets.ts`, `useReportPosition.ts` ; specs `docs/superpowers/specs/2026-08-2{8,9}-revision-pdf-*` | À détacher de la notion de « lot », de l'API PHP (`lotSourcePdfUrl`) et du mode focus. Stack : React 19, Tailwind 4, Radix, `react-pdf`/`pdfjs-dist`. |
| Lumosphère | Fabrication Tauri : `src-tauri/` (profil release allégé, AppImage, mises à jour signées), `scripts/fabriquer-applis*.sh` | Modèle pour la configuration et l'empaquetage. |
| Lumosphère | Règles de `workers/process_pdf.py` : corps = taille dominante en volume ; tolérances de taille 0,5 pt ; ≤ 12 mots pour un titre ; ligne finale > 15 % de blanc ; saut du sommaire avec filet de sécurité | À **porter en Rust** (règles validées sur de vrais livres). |
| Lumosphère | `workers/export_epub_paquet.py` (EPUB 3 sans outil externe) | Référence pour l'emballage (le reste de `export_epub.py` est propre à Lumosphère). |
| `pdf_to_md.py` (ancien dépôt `editeurmd`) | Soft hyphens, recollage au saut de page, motifs de numéros de page (« Page X de Y », « - 12 - », …) | Règles à reprendre. Ses défauts constatés : paragraphes fusionnés, aucun titre, doubles pages illisibles. |
| **`pdf-to-ebook`** (github.com/deniztasdogen/pdf-to-ebook, **Unlicense** = domaine public, Rust, pdfium) | `crates/extract/src/layout/{chrome,columns,headings,paragraphs,lines}.rs`, `ocr.rs`, `pdf.rs`, `crates/ebook/src/epub.rs` | Copiable librement. Projet jeune (3 commits), **non testé** sur notre corpus : à évaluer avant de s'appuyer dessus. |
| chunky (github.com/GiovanniPasq/chunky, MIT) | Atelier RAG : PDF/Markdown/chunks côte à côte, comparaison de découpages | Inspiration pour l'étape RAG future, non testé. |

## 12. Corpus de test et constats

### 12.1 Fichiers (hors dépôt — chemins locaux)
| # | Fichier (sous `/home/stef/Documents/RAG/`) | Profil | Attendus vérifiables |
|---|---|---|---|
| 1 | `omraam/pdf/Aïvanhov, Omraam Mikhaël - Approche de la Cité céleste.pdf` | Calibre, Lettre, 136 p. | 73 signets = chapitres (« 1. VISITE À PATMOS »…) ; titres 19,4 pt gras |
| 2 | `omraam/pdf/Anges et l’arbre de vie - Omraam Mikhaël Aïvanhov.pdf` | Calibre (ex-Word), 195 p., pas de signets | « CHAPITRE N: » en 16,9 pt gras MAJ ; métadonnée titre fausse |
| 3 | `omraam/pdf/L’alchimie_spirituelle_Tome_2_Omraam_Mikhaël_Aïvanhov.pdf` | Scan Acrobat Paper Capture, 207 p. | En-tête alterné : paire « 112 L'alchimie spirituelle », impaire « *titre du chapitre* 113 » ; signets inutilisables (un par image) ; note « * » p. 36 ; ouverture de chapitre p. 29 |
| 4 | `steiner/pdf/Chronique de l'Akasha - Rudolf Steiner.pdf` | Calibre, A5, 252 p. | 23 signets ; titres 9 pt gras MAJ (**plus petits** que le corps 12 pt) |
| 5 | `steiner/pdf/Le sens de l'amour (Rudolf Steiner).pdf` | Calibre, 106 p. | 11 signets sur 2 niveaux |
| 6 | `steiner/pdf/Steiner Rudolf - Connaissance Logique Pensée pratique.pdf` | Scan OmniPage, **doubles pages**, 96 p. | Pied « 92 93 » ; signets « Page N » inutilisables ; **notes regroupées** sous des titres « Notes » / « NOTES » avec appels numérotés |
| 7 | `bailey/pdf/[02] Lettres Sur La Méditation Occulte - Alice Bailey.pdf` | Distiller, A4, 262 p., pas de signets | Schéma encadré p. 6 → image, pas tableau |
| 8 | `bailey/pdf/Alice Bailey - Initiation humaine et solaire.pdf` | Calibre, 234 p. | Titres « CHAPITRE I » gras MAJ **même taille** que le corps (15 pt) ; p. 145 : **liste numérotée** en bas de page, à ne pas prendre pour des notes |
| 9 | `bailey/pdf/guerison esoterique_ Une connai - Alice Bailey.pdf` | Calibre, **768 p.** | Idem 8 ; p. 367 : liste numérotée, pas des notes |

Les noms contiennent des apostrophes typographiques et des accents en NFD : les
retrouver par chemin normalisé. Ces PDF servent de **tests de non-régression** du moteur
via une variable d'environnement pointant le corpus ; ils ne sont jamais versionnés.

### 12.2 Résultats de l'essai des scripts existants (2026-11-08)
| | `pdf_to_md.py` | `process_pdf.py` |
|---|---|---|
| Titres | aucun sur 9 PDF | bons sur #1 ; 94 faux h1 sur #2 ; 142 faux h3 sur #3 ; 448 faux h3 sur #6 ; aucun sur #4, #7, #8, #9 |
| Paragraphes (médiane) | 1 800 à 8 000 car., max 42 000 | 70 à 600 car. |
| Double page #6 | lignes des deux pages entremêlées | ordre correct |
| En-têtes #3 | partiellement restés | non traités |

### 12.3 Constats sur la qualité du texte
Taux de mots reconnus par le dictionnaire (`/usr/share/dict/french`) : 93 à 98 % sur
les 9 PDF, et 95,7 % sur un livre dont la page de copyright est un OCR illisible
(`omraam/Au_Commencement_était_le_Verbe_Tome_9_…pdf`). **Une mesure globale ne suffit
pas** : la détection du texte douteux doit être faite **par page**.

## 13. Critères de réussite
- Sur les 9 PDF du corpus : titres de chapitre attendus (12.1) retrouvés ; aucun
  en-tête/pied résiduel ; paragraphes de taille plausible ; double page lue dans l'ordre ;
  le schéma de #7 en image ; les notes en notes cliquables.
- Aucun texte retiré sans être visible dans l'appli (zones, points à vérifier).
- EPUB valides (epubcheck) et lisibles dans Foliate et Calibre.
- Un scan sans texte se convertit après OCR **sans aucune installation** par l'utilisateur.
- Un nouveau PDF d'une série réglée est rangé, contrôlé et converti sans réglage
  supplémentaire quand il est conforme.

## 14. Laissé au constructeur
- Syntaxe du Markdown interne (repères de page, notes, tableaux, références d'images),
  dans le respect de la section 7.
- Format de stockage local (SQLite, JSON…) et emplacement (répertoire de données de
  l'appli).
- Seuils et tolérances exacts du contrôle d'appartenance et de la qualité par page, à
  calibrer sur le corpus.
- Choix tessdata_fast ou tessdata_best ; mode d'appel de Tesseract (programme embarqué
  ou bibliothèque liée).
- Éditeur du Markdown brut (CodeMirror recommandé) et moteur d'affichage PDF
  (`pdfjs-dist`, comme Lumosphère).
- Découpage en crates/modules, gestion de la concurrence de conversion.
- Ce qu'on reprend réellement de `pdf-to-ebook` après évaluation.

## 15. Choix de conception à relire par le porteur
1. pdfium plutôt que PyMuPDF (licence).
2. Tesseract embarqué, sans ocrmypdf ni Ghostscript ; pas de « PDF cherchable » en v1.
3. Règle des titres en cascade (signets → gras+MAJ → taille), forçable par livre.
4. Exceptions de zones par page.
5. Zone « notes » distincte du pied.
6. Schémas et illustrations en image ; tableaux en HTML signalés en révision.
7. Vue Liseuse en lecture seule en v1.
8. Couverture = page 1 ; métadonnées depuis le PDF ou le nom de fichier.
9. Le corpus de test reste hors du dépôt public.
10. Regroupement automatique en séries proposées au premier import.
11. Italique, gras et exposants conservés (sauf OCR) ; langue réglée par série, français
    en v1.
12. Synchro et surlignage au paragraphe, grâce à des ancres de bloc dans le Markdown.
13. Un livre ouvert en révision est figé ; le reconvertir est volontaire et efface les
    corrections (avec avertissement).
