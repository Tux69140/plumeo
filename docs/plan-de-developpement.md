# Pluméo — plan de développement complet (v1)

**Date :** 2026-11-10 · **Cadrage :** [`docs/superpowers/specs/2026-11-08-plumeo-convertisseur-design.md`](superpowers/specs/2026-11-08-plumeo-convertisseur-design.md)

Ce document décrit **tout le chemin** jusqu'à une Pluméo v1 **utilisable selon le cadrage** :
- importer des PDF ou des dossiers ;
- les ranger en séries et régler chaque série visuellement ;
- convertir ;
- réviser côte à côte ;
- produire des EPUB propres, sous Linux, sans rien installer d'autre que Pluméo.

Le chemin compte **6 jalons**. Chacun livre un logiciel qui fonctionne seul et se teste. Le
détail d'un jalon (code des tests, signatures exactes) s'écrit au moment de le commencer,
dans `docs/superpowers/plans/`, parce qu'il dépend du code des jalons précédents. Le
jalon 1 est déjà détaillé et en cours.

---

## 0. Vue d'ensemble

| Jalon | Livrable | Ce qu'on peut faire à la fin | Dépend de | État |
|---|---|---|---|---|
| **1. Moteur et EPUB** | Bibliothèques Rust + `plumeo-cli` | Convertir un PDF à couche texte en EPUB, en ligne de commande, avec un fichier de réglages de série | — | **terminé** (contrôle visuel Foliate à faire par le porteur) |
| **2. Analyse à l'import** | Diagnostic, empreinte, séries, contrôle d'appartenance + `plumeo-cli analyser` | Analyser un dossier : scans, doubles pages, schémas, texte douteux par page, séries proposées, fiche d'appartenance | 1 | à faire |
| **3. Application** | Appli Tauri : stockage, import, réglage de série, conversion en arrière-plan | Tout faire depuis l'appli, sauf la révision : importer, ranger, régler visuellement, convertir, obtenir l'EPUB | 1, 2 | à faire |
| **4. Révision** | Écran PDF / Markdown synchronisé, points à vérifier, validation | Vérifier et corriger chaque livre côte à côte avant de produire l'EPUB | 3 | à faire |
| **5. OCR et paquet** | Tesseract embarqué + AppImage | Convertir un scan sans texte sans rien installer ; distribuer Pluméo en un fichier | 3 (4 conseillé) | à faire |
| **6. Mise en page avancée** | Colonnes, schémas en image, tableaux HTML | Convertir correctement les pages à colonnes, les schémas et les tableaux | 1, 4 | à faire |

**Pluméo v1 utilisable = jalons 1 à 6 terminés et critères de la section 7 vérifiés.**

Ordre conseillé : 1 → 2 → 3 → 4 → 5 → 6.
- Le jalon 5 peut passer avant le 4 si les scans deviennent prioritaires.
- Le jalon 6 a besoin de l'écran de révision (jalon 4) pour ses zones tracées à la main.

---

## Conventions communes à tous les jalons

**Dépôt et branches :**
- Dépôt public `git@github.com:Tux69140/plumeo.git`.
- Une branche par jalon (`plan-N-…`), fusionnée dans `main` quand le jalon est validé.
- **Aucun push sans accord du porteur.**
- Aucun PDF ni EPUB versionné (droits d'auteur) : le `.gitignore` exclut `*.pdf`, `*.epub` et `vendor/`.

**Tests :**
- On écrit le test avant le code (TDD) ; chaque tâche est relue avant de passer à la suivante.
- Les PDF de test sont **fabriqués par le code de test**.
- Le corpus réel (`/home/stef/Documents/RAG`, variable `PLUMEO_CORPUS`) sert aux tests de non-régression marqués `#[ignore]`.

**Suivi :**
- Chaque jalon met à jour `docs/corpus.md` (constats sur les 9 PDF).
- Chaque jalon met à jour la section « État » de ce document.

**Règles qui ne se négocient pas** (anti-objectifs du cadrage) :
- jamais de perte silencieuse de texte : tout retrait est tracé et visible ;
- aucun outil à installer pour l'utilisateur ;
- aucune IA ni aucun service en ligne ;
- les PDF d'origine ne sont jamais modifiés ;
- pas d'EPUB à mise en page fixe ;
- code maintenable.

---

## Jalon 1 — Moteur de conversion et EPUB (ligne de commande)

**Plan détaillé :** [`docs/superpowers/plans/2026-11-10-plan-1-moteur-epub.md`](superpowers/plans/2026-11-10-plan-1-moteur-epub.md)
**Cadrage couvert :** §5 (étapes 1–3, 5, 6, 9, 10, 12), §6, §7.

| # | Tâche | Livré | État |
|---|---|---|---|
| 1.1 | Espace de travail Cargo, pdfium, lecture des mots (position, taille, gras, italique), signets, métadonnées, rendu PNG | `plumeo-moteur::pdf` | ✅ |
| 1.2 | Mots → lignes, exposants (appels de note) | `lignes` | ✅ |
| 1.3 | Réglages de série (JSON), coupe des doubles pages, zones en-tête / pied / notes par parité, exceptions de page, numéros de page résiduels | `reglages`, `double_page`, `zones` | ✅ |
| 1.4 | Lignes → paragraphes : fin de paragraphe (blanc > 15 %), alinéas, césures, U+00AD, italique/gras/exposants, raccord entre pages | `blocs`, `paragraphes` | ✅ |
| 1.5 | Titres en cascade (signets → gras+MAJ → taille), sommaire d'origine, titres courants suspects | `titres`, `points` | ✅ |
| 1.6 | Notes de bas de page et notes regroupées sous « Notes », appariement, orphelines | `notes` | ✅ |
| 1.7 | Markdown interne ancré (`<!-- @p12 … -->`), échappement, conversion d'un livre entier, aperçu d'une page | `markdown`, `convertir` | ✅ |
| 1.8 | EPUB 3 : un fichier par chapitre, nav, notes `noteref`/`footnote` avec retour ↩, couverture, epubcheck | crate `plumeo-epub` | ✅ |
| 1.9 | Titre et auteur proposés depuis le nom de fichier, nom de sortie sûr, `plumeo-cli convertir` / `page` | crate `plumeo-cli` | ✅ |
| 1.10 | Réglages des séries du corpus, non-régression sur les 9 PDF, contrôle visuel dans Foliate | `tests/corpus/`, `docs/corpus.md` | ✅ (Foliate : porteur) |

**Livrable :**
```
plumeo-cli convertir <pdf> --serie <reglages.json> --sortie <dossier>
```
Cette commande produit l'EPUB, le Markdown interne, la liste des retraits et les points à vérifier.

**Critères d'acceptation :**
- les attendus du corpus (§12.1 du cadrage) sont tenus pour les 9 PDF ;
- epubcheck ne trouve aucune erreur ;
- le livre de 768 pages se convertit en moins de 60 s.

---

## Jalon 2 — Analyse à l'import

**Cadrage couvert :** §4.2 (vérification à l'import), §4.3 (séries, empreinte, contrôle d'appartenance), §12.3 (qualité par page).
**Livrable :**
- des modules du moteur, utilisables par l'appli ;
- la commande `plumeo-cli analyser <pdf|dossier> [--series <dossier>]`, qui produit un rapport JSON par PDF.

### 2.1 Qualité du texte par page
**Étapes :**
1. Embarquer un dictionnaire français à licence compatible. Piste : la liste de mots de
   Grammalecte / Dicollecte (MPL 2.0). Il est chargé depuis les ressources de l'appli,
   jamais depuis `/usr/share/dict`.
2. Mesurer pour chaque page :
   - le taux de mots reconnus ;
   - les mots hybrides (lettres et chiffres mêlés) ;
   - les caractères parasites ;
   - la longueur moyenne des mots.
3. Calibrer les seuils sur le corpus. Cas de référence : la page de copyright illisible
   du tome 9, dans un livre dont le taux global est de 95,7 %.

**Livré :**
- `QualitePage { page, taux_reconnus, hybrides, parasites, douteuse: bool }` ;
- `pages_douteuses(doc) -> Vec<u32>`.

**Acceptation :** la page illisible du tome 9 est signalée, et aucune page des 9 PDF propres n'est signalée à tort, ou au plus quelques-unes, listées dans `docs/corpus.md`.

### 2.2 Diagnostic d'un PDF
**Étapes :**
1. **Aucune couche texte :** des pages qui portent une image pleine page mais aucun caractère.
2. **Doubles pages :** proportion largeur/hauteur > 1,2 et deux blocs de texte séparés par un pli.
3. **Schémas et images :** objets image, ou densité de tracés vectoriels (cadres, flèches).
4. **Tableaux probables :** filets horizontaux et verticaux, ou colonnes de texte alignées.

**Livré :**
- `Diagnostic { cas: Vec<Cas>, pages_douteuses, pages_images, pages_tableaux }` ;
- avec `Cas = SansTexte | TexteDouteux | DoublePage | Schemas | Tableaux`.

**Acceptation :**
- #6 détecté en double page ;
- #3 sans alerte « sans texte » ;
- le schéma de la p. 6 de #7 signalé ;
- un PDF fabriqué sans texte signalé « SansTexte ».

### 2.3 Empreinte de mise en page
**Étapes :** on relève :
- le producteur et le créateur ;
- le format et les proportions ;
- la taille du corps (dominante en volume) ;
- simple ou double page ;
- les marges du bloc de texte, médianes, paires et impaires séparées.

**Livré :** `Empreinte` (sérialisable) et `empreinte(doc) -> Empreinte`.

**Acceptation :** les 5 livres Calibre en Lettre, à 15 pt, ont des empreintes voisines, et la marge basse de Bailey (9 %) se distingue de celle des autres (11 %).

### 2.4 Regroupement automatique en séries
**Étapes :**
1. Regrouper par empreinte, dans des tolérances : format, taille du corps ± 0,5 pt, marges ± 1,5 %.
2. Nommer provisoirement chaque groupe (« calibre · Lettre · 15 pt »).
3. Mettre les PDF isolés dans « Sans série ».

**Livré :** `proposer_series(empreintes) -> Vec<SerieProposee>`.

**Acceptation :** sur les 9 PDF, les regroupements sont plausibles et expliqués dans `docs/corpus.md`. Le producteur seul ne suffit pas à regrouper.

### 2.5 Contrôle d'appartenance
**Étapes :**
1. Appliquer les réglages d'une série à environ 15 pages réparties dans le livre.
2. Mesurer :
   - le format ;
   - les marges, dans la tolérance ;
   - aucune ligne de corps coupée par une zone ;
   - un contenu retiré qui se répète d'une page à l'autre ;
   - la taille et l'interligne du corps ;
   - les colonnes ;
   - un nombre de titres plausible.
3. Produire une fiche ✓/⚠ par critère, avec sa raison chiffrée.
4. Fournir les données de la planche : pages choisies et zones à superposer.

**Livré :**
- `FicheAppartenance { criteres: Vec<Critere { nom, ok, raison }> }` ;
- `controler(doc, reglages) -> FicheAppartenance`.

**Acceptation :** Bailey #8 contrôlé contre la série des Omraam Calibre donne ⚠ « marge basse 9 % contre 11 % pour la série ». Un livre de sa propre série donne ✓ partout.

### 2.6 Modèle des séries
**Étapes :**
- créer, dériver (copie des réglages), renommer, fusionner ;
- déplacer un livre d'une série à une autre ;
- tenir l'historique des réglages, pour savoir quels livres ont été convertis avec quelle version.

**Livré :** le module `series`, sans interface, testé unitairement.

### 2.7 Commande `analyser` et non-régression
**Livré :** `plumeo-cli analyser`, et le test corpus étendu (diagnostics, regroupement, fiches).

**Acceptation du jalon :** `plumeo-cli analyser /home/stef/Documents/RAG/…` sur les 3 dossiers du corpus produit en moins d'une minute :
- les séries proposées ;
- les diagnostics ;
- les fiches.

Tout cela sans faux scan.

---

## Jalon 3 — L'application : stockage, import, réglage de série

**Cadrage couvert :** §3 (Tauri, React), §4.1 (écran d'import, option B), §4.3 (séries dans l'interface), §4.4 (réglage de série, option A + navigation), §8 (stockage), §9 (arrière-plan).

**Livrable :** l'appli Pluméo (Tauri 2), lancée par `cargo tauri dev` puis construite en binaire Linux. Elle permet d'importer, ranger, régler, convertir et produire les EPUB. La révision vient au jalon 4 ; d'ici là, un livre converti passe directement en « EPUB produit ».

### 3.1 Coquille de l'appli
**Étapes :**
1. Créer le projet Tauri 2 et l'interface : React 19, TypeScript, Vite, Tailwind 4, Radix. Le choix suit Lumosphère.
2. Ajouter `src-tauri` comme membre du workspace Cargo.
3. Exposer les commandes Tauri qui appellent `plumeo-moteur`.
4. Placer pdfium à côté de l'exécutable.
5. Mettre en place la structure des écrans :
   - Import ;
   - Séries ;
   - Révision (vide pour l'instant) ;
   - Préférences.

**Acceptation :** l'appli s'ouvre, liste les écrans et appelle une commande du moteur (nombre de pages d'un PDF).

### 3.2 Stockage local
**Étapes :**
1. Base SQLite (`rusqlite`), dans le répertoire de données de l'appli (`~/.local/share/plumeo/`). Elle contient :
   - les tables séries (réglages versionnés), livres (chemin normalisé NFC, empreinte, diagnostic, série, état, version des réglages utilisés), exceptions de page et préférences (dossier de sortie) ;
   - les Markdown, sous forme de fichiers rangés par livre ;
   - les migrations.
2. Les PDF d'origine ne sont jamais copiés.
3. Un livre déjà importé, même encodé en NFD, est reconnu.

**Acceptation :** les données survivent à un redémarrage, et un réimport ne crée pas de doublon.

### 3.3 Travaux en arrière-plan
**Étapes :**
- une file de travaux (analyse, conversion, plus tard OCR), exécutés sur des fils dédiés ;
- la progression est envoyée par des événements Tauri ;
- un travail peut être annulé ;
- l'interface reste utilisable pendant les travaux.

**Acceptation :** l'interface reste réactive pendant la conversion du livre de 768 pages.

### 3.4 Écran d'import (option B : regroupé par série)
**Étapes :**
1. Boutons « + PDF » et « + Dossier », avec le dialogue natif. Le dossier est lu de façon récursive.
2. Dossier de sortie séparé, **configurable**, affiché en permanence.
3. Livres rangés **sous leur série**, chaque série indiquant « réglée » ou « à régler ».
4. Section « Sans série ».
5. Chaque ligne de livre porte :
   - le diagnostic (jalon 2) ;
   - l'action proposée : refaire l'OCR, couper en 2, choisir une série ;
   - l'état : en attente, bloqué, à convertir, conversion…, à réviser, validé, EPUB produit.

**Acceptation :** l'import des 3 dossiers du corpus reproduit la maquette B, avec les vrais diagnostics.

### 3.5 Premier import : séries proposées
**Étapes :** quand aucune série n'existe, l'appli montre les séries proposées (jalon 2.4). On peut les confirmer, les renommer, les scinder ou les fusionner, et déplacer un livre.

**Acceptation :** depuis un dossier vierge, on obtient en quelques clics des séries nommées.

### 3.6 Fiche d'appartenance et planche
**Étapes :**
1. Afficher pour chaque livre sa fiche ✓/⚠ avec les raisons chiffrées.
2. Afficher la planche : les pages du livre avec les zones de la série superposées.
3. En cas d'échec, proposer trois actions :
   - rattacher quand même ;
   - créer une série dérivée ;
   - créer une nouvelle série.

**Acceptation :** Bailey #8 montre ⚠ sur sa marge basse, et le bouton « série dérivée » crée une copie réglable.

### 3.7 Écran de réglage de série (option A + navigation)
**Étapes :**
1. **À gauche :** la liste des séries et « + nouvelle série ».
2. **Au centre :**
   - une grande page du PDF (`pdfjs-dist`), avec les zones superposées **manipulables à la souris** : en-tête, pied et notes, plus le pli d'une double page ;
   - défilement à la molette et aux flèches ;
   - **champ de numéro de page** ;
   - bascule **paire / impaire**.
3. **À droite :**
   - les valeurs chiffrées ;
   - le **texte extrait de la page affichée**, mis à jour en direct (commande `apercu_page`) ;
   - les retraits de la page.
4. **Réglages :**
   - source des titres (auto, signets, gras+MAJ, taille), avec forçage possible par livre ;
   - langue (`fra`) ;
   - colonnes : 1, 2 ou auto, ce dernier n'étant actif qu'au jalon 6.
5. **Exceptions de page :** des zones propres à une page, sans toucher la série.

**Acceptation :** sur L'alchimie spirituelle (#3), on règle les en-têtes alternés paire/impaire et on pose l'exception de la p. 29. Le texte extrait se met à jour en moins d'une demi-seconde.

### 3.8 Conversion d'une série et production des EPUB
**Étapes :**
1. Une série réglée part entièrement en conversion.
2. Les EPUB sont écrits dans le dossier de sortie.
3. Le titre et l'auteur proposés sont modifiables dans la ligne du livre.
4. Une série modifiée après coup ne reconvertit que les livres **pas encore révisés** (décision du cadrage, §4.5).

**Acceptation du jalon :** depuis l'appli seule, on importe les 9 PDF, on règle leurs séries et on obtient 9 EPUB valides dans le dossier de sortie choisi.

---

## Jalon 4 — Révision côte à côte

**Cadrage couvert :** §4.5 (option B, Markdown brut par défaut), §6 (métadonnées, couverture), §7 (ancres qui survivent à l'édition).
**Livrable :** l'écran de révision. Le parcours complet du cadrage fonctionne.

### 4.1 Lecture et survie des ancres
**Étapes :**
1. Lire le Markdown en blocs ancrés.
2. Après une édition :
   - un bloc modifié garde son ancre ;
   - un bloc ajouté hérite de celle de son voisin ;
   - un bloc supprimé disparaît avec la sienne.

**Livré :** `lire_blocs(md) -> Vec<BlocLu { ancre, markdown, debut, fin }>`.

**Acceptation :** les tests de survie des ancres (modifier, scinder, fusionner, insérer) passent.

### 4.2 Volet PDF
**Étapes :** reprendre de Lumosphère `PdfPane`, `PdfToolbar`, `usePdfZoom`, `usePdfRecherche`, `useHauteurVolets` et `useLargeurVolets`, détachés :
- des lots ;
- de l'API PHP (`lotSourcePdfUrl`) ;
- du mode focus.

Le volet offre le zoom, le numéro de page et la recherche.

### 4.3 Éditeur Markdown brut
**Étapes :**
- CodeMirror 6 ;
- repères de page visibles ;
- commentaires d'ancre atténués ou repliés ;
- coloration des titres et des notes ;
- balises HTML en ligne (`<sup>`, `<em>`, `<strong>`) colorées à part, pour qu'on les repère au survol ;
- enregistrement automatique dans le stockage.

### 4.4 Synchro au paragraphe
**Étapes :**
- défilement synchronisé à partir des ancres ;
- le bloc sous le curseur est surligné **des deux côtés**, avec son rectangle sur le PDF.

Le défilement proportionnel de Lumosphère ne suffit pas.

**Acceptation :** sur #3 p. 104, cliquer un paragraphe du Markdown surligne le même paragraphe sur le PDF, et inversement.

### 4.5 Points à vérifier
**Étapes :**
- la liste ⚑ : titres suspects, texte douteux, notes orphelines, sommaire, retraits douteux, et plus tard tableaux et schémas ;
- un point ⚑ « balise HTML non reconnue » pour toute balise hors de la liste blanche de `plumeo-epub` : elle serait affichée en texte dans l'EPUB. La vérification réutilise la fonction de `plumeo-epub`, recalculée à chaque modification du Markdown ;
- un clic amène aux deux endroits ;
- on peut marquer un point comme vu.

### 4.6 Liseuse
**Étapes :** une bascule « Liseuse » en lecture seule, qui rend le chapitre comme dans l'EPUB (même code que `plumeo-epub`).

### 4.7 Zone « image » tracée
**Étapes :** tracer un rectangle sur le PDF pour remplacer les blocs couverts par une image rendue de la zone. La zone « tableau » vient au jalon 6.

### 4.8 Métadonnées et couverture
**Étapes :**
- titre, auteur et langue modifiables ;
- couverture remplaçable : page 1 par défaut, ou une autre page, ou une image.

### 4.9 Validation, livre figé, reconversion volontaire
**Étapes :**
1. « Valider » génère l'EPUB depuis le Markdown révisé.
2. Un livre ouvert en révision n'est plus touché par les changements de sa série.
3. « Reconvertir » reste possible, mais avec un avertissement : les corrections sont perdues.
4. « Valider » refuse un livre qui contient encore une balise HTML non reconnue, et renvoie au point ⚑.
5. Avant d'accepter du Markdown édité, resserrer la liste blanche HTML de `plumeo-epub` (relecture finale du jalon 1) :
   - attributs de `img`, `br` et `hr` filtrés : ni `onerror`, ni `src` externe ou absent du paquet ;
   - un `>` entre guillemets ne ferme pas la balise (`<img alt="a>b">`) ;
   - entités HTML nommées (`&nbsp;`) converties au lieu de rendre l'XHTML invalide.

**Acceptation du jalon :** le parcours complet sur #1, #3 et #6 :

| Étape | Action |
|---|---|
| 1 | import |
| 2 | réglage |
| 3 | conversion |
| 4 | révision, avec une correction de titre et une de paragraphe |
| 5 | validation |

À l'issue, l'EPUB contient les corrections, et un redémarrage les conserve.

---

## Jalon 5 — OCR embarqué et paquet Linux

**Cadrage couvert :** §3 (Tesseract embarqué, AppImage), §4.2 (« Lancer l'OCR », « Refaire l'OCR »), §5.1 (mots OCR avec boîtes), §5.11 (langue), §13 (scan converti sans installation).
**Livrable :** une AppImage autonome. Un scan sans texte se convertit après OCR, sur un Debian sans Tesseract installé.

### 5.1 Choix de l'intégration de Tesseract
**Étapes :**
1. Comparer deux façons d'intégrer Tesseract : la bibliothèque liée (crate `tesseract`/`leptess`) et le programme embarqué dans l'AppImage.
2. Comparer `tessdata_fast` (1 Mo) et `tessdata_best` (3,7 Mo) sur #3 : qualité et temps.
3. Consigner la décision dans `docs/`.

### 5.2 OCR d'une page
**Étapes :**
1. Rendre la page avec pdfium (300 dpi).
2. Passer l'image à Tesseract, qui renvoie du TSV ou du hOCR.
3. Convertir le résultat en `Mot` avec boîtes. Ces mots n'ont pas d'italique.
4. Stocker le résultat à part, sans jamais toucher au PDF.
5. Le moteur lit ces mots à la place de la couche texte.

### 5.3 Actions d'OCR dans l'appli
**Étapes :**
- « Lancer l'OCR » sur un livre bloqué ;
- « Refaire l'OCR » sur tout le livre ou sur les pages douteuses (jalon 2.1) ;
- le travail tourne en arrière-plan, avec progression ;
- la conversion reprend ensuite automatiquement.

### 5.4 Langue par série
**Étapes :**
- la langue choisit le modèle Tesseract, le dictionnaire de qualité et `dc:language` ;
- ajouter une langue revient à ajouter un modèle et un dictionnaire.

### 5.5 Empaquetage AppImage
**Étapes :**
1. Construire l'AppImage sur le modèle de `lumosphere/scripts/fabriquer-applis*.sh` (profil release allégé). Elle contient pdfium, Tesseract, tessdata `fra` et le dictionnaire.
2. Tester sur une machine ou un conteneur Debian 13 **sans** tesseract, poppler ni Calibre.

**Acceptation du jalon :** un PDF scanné sans couche texte est importé, l'OCR est lancé et le livre est converti en EPUB valide, sans aucune installation en dehors de l'AppImage.

---

## Jalon 6 — Mise en page avancée : colonnes, schémas, tableaux

**Cadrage couvert :** §4.4 (colonnes, alternance), §5.4, §5.7 (tableaux en vrai HTML), §5.8 (schémas en image), §13 (schéma de #7 en image).
**Livrable :** un moteur qui lit les pages à colonnes et produit des images et des tableaux HTML, avec les outils de correction dans la révision.

### 6.1 Colonnes et ordre de lecture
**Étapes :**
- détecter les gouttières (histogramme des blancs verticaux) ;
- choisir entre 1, 2 colonnes ou auto ;
- gérer l'**alternance** sur une même page, entre une partie pleine largeur et une partie à colonnes ;
- lire dans l'ordre : de haut en bas, puis colonne après colonne.

**Acceptation :** une page fabriquée avec un titre pleine largeur, 2 colonnes, puis un paragraphe pleine largeur se lit dans le bon ordre.

### 6.2 Schémas et illustrations en image
**Étapes :**
1. Repérer les zones d'images et de tracés vectoriels.
2. Les rendre en PNG.
3. Les insérer dans le Markdown avec `![](images/…)`.
4. L'EPUB embarque les images.
5. Ajouter un point à vérifier « schéma gardé en image ».

**Acceptation :** le schéma de la p. 6 de #7 devient une image et non un tableau.

### 6.3 Tableaux en vrai HTML
**Étapes :**
1. Détecter les tableaux (filets, alignements).
2. Reconstruire les lignes et les cellules en `<table>`.
   Ajouter `table`, `thead`, `tbody`, `tr`, `th` et `td` à la liste blanche de `plumeo-epub` : sinon le tableau sort en texte, et le point ⚑ « balise HTML non reconnue » (4.5) se déclenche.
3. Signaler chaque tableau comme point à vérifier.
4. En révision, permettre de **basculer le tableau en image**.

**Acceptation :** des tableaux fabriqués (avec et sans filets) sont reconstruits sans erreur.

### 6.4 Zone « tableau » tracée en révision
**Étapes :** une zone tracée à la main force la reconstruction en tableau, ou en image, à cet endroit.

**Acceptation du jalon :** sur le corpus :
- le schéma de #7 sort en image ;
- les pages à colonnes éventuelles du corpus élargi sont lues dans l'ordre ;
- tout tableau reconstruit apparaît dans les points à vérifier.

---

## 7. Critères de réussite de la v1 (cadrage §13) et jalon qui les tient

| Critère | Jalon |
|---|---|
| Sur les 9 PDF : titres de chapitre attendus retrouvés | 1 |
| Aucun en-tête ou pied résiduel | 1 (réglage), 3 (réglage visuel) |
| Paragraphes de taille plausible | 1 |
| Double page lue dans l'ordre | 1 |
| Schéma de #7 en image | 6 |
| Notes cliquables, avec retour à la position de lecture | 1 (EPUB), 4 (vérification) |
| Aucun texte retiré sans être visible dans l'appli | 1 (retraits), 3–4 (affichage) |
| EPUB valides (epubcheck), lisibles dans Foliate et Calibre | 1 |
| Scan sans texte converti après OCR, sans aucune installation | 5 |
| Nouveau PDF d'une série réglée rangé, contrôlé et converti sans réglage supplémentaire s'il est conforme | 2 + 3 |
| Conversion d'un livre à couche texte en secondes ; OCR en arrière-plan, interface utilisable | 1, 3, 5 |

---

## 8. Après la v1 (prévu, hors périmètre)

1. **Export Markdown.** Le Markdown interne est déjà la base. L'export retire les ancres
   et garde des repères de page légers.
2. **Chunks pour RAG, d'abord pour Open WebUI.** Avant toute conception, vérifier si
   Open WebUI redécoupe lui-même les documents. Chaque chunk porte sa source, sa page et
   son chemin de titres (ancres). Inspiration : chunky.
3. **Windows.** pdfium, Tesseract et Tauri existent sous Windows. Il faudra un paquet MSI
   ou NSIS et faire passer les tests du corpus sous Windows.
4. **PDF cherchable après OCR.**
5. **Tableaux par modèle** : piste `oar-ocr`.

---

## 9. État

| Date | Jalon | Avancement |
|---|---|---|
| 2026-11-08 | — | Cadrage validé et commité |
| 2026-11-10 | 1 | Plan détaillé écrit. Tâches 1.1 à 1.6 codées, relues et commitées sur `plan-1-moteur` (non poussé) |
| 2026-11-16 | 1 | Tâches 1.7 à 1.10 faites ; relecture finale et correctifs (notes de fin de livre liées entre chapitres, résumé sans numéros en temps linéaire, OPF/nav validés). 135 tests, corpus 9/9 sans erreur epubcheck. Fusionné dans `main` et poussé. Reste : contrôle visuel Foliate des n° 1, 3 et 6 |
| 2026-11-16 | 1 | Retours du contrôle visuel du jalon 1 corrigés sur `plan-1-retours` (non fusionné, non poussé) : sauts de section `* * *` aux pages courtes, entrées numérotées séparées, signets à plat fusionnés, notes Calibre `[N]`, faux titres rejetés, titre du livre en tête du Markdown, sommaire imprimé cliquable, paragraphes espacés. Corpus 9/9 sans erreur epubcheck ; sorties régénérées dans `plumeo-controle/` pour un nouveau contrôle visuel du porteur |
| 2026-11-16 | 2 | Sommaires imprimés, sur `plan-2-sommaires` (non fusionné, non poussé) : une ligne de sommaire finissant par un numéro de page termine son paragraphe, entrées sur plusieurs lignes et numéro en tête ignoré, sommaire imprimé source des titres de niveau 1 des livres sans signets (Connaissance : 3 → 14 titres), sommaire en tête de l'EPUB (`chap-002.xhtml`). Corpus 9/9 sans erreur epubcheck, attentes figées pour les n° 3 et 6 |
