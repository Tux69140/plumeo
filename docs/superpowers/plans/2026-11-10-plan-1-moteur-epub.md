# Plan 1 — Moteur de conversion et EPUB (ligne de commande)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**But :** un moteur Rust qui convertit un PDF à couche texte en Markdown interne ancré
puis en EPUB 3, piloté par un fichier de réglages de série, utilisable en ligne de
commande et testé sur les 9 PDF du corpus.

**Architecture :** un espace de travail Cargo à trois crates :
- `plumeo-moteur` (bibliothèque) : lecture pdfium, zones, lignes, paragraphes, titres, notes et Markdown ;
- `plumeo-epub` (bibliothèque) : Markdown interne → EPUB 3 ;
- `plumeo-cli` (binaire) : branche les deux et sert d'outil de calibrage.

L'appli Tauri (plans suivants) réutilisera ces deux bibliothèques telles quelles.

**Pile :**
- Rust 1.98, édition 2024 ;
- `pdfium-render` 0.9.5 et pdfium `chromium/8125` (binaire bblanchon) ;
- `pulldown-cmark` 0.13, `zip` 9, `serde`/`serde_json` 1, `thiserror` 2, `regex` 1 ;
- `unicode-normalization` 0.1, `uuid` 1 (v5), `clap` 4 ;
- en test : `tempfile` 3, `roxmltree` 0.21.

**Spec :** `docs/superpowers/specs/2026-11-08-plumeo-convertisseur-design.md`

## Découpage en plans

Le document de conception couvre six sous-systèmes. Chaque plan livre un logiciel qui
fonctionne seul :

| Plan | Contenu | Sections du document |
|---|---|---|
| **1 (celui-ci)** | Moteur à couche texte + EPUB, en ligne de commande | 5 (étapes 1–3, 5, 6, 9, 10, 12), 6, 7 |
| 2 | Analyse à l'import : qualité par page, scan, double page, empreinte, regroupement en séries, contrôle d'appartenance | 4.2, 4.3 |
| 3 | Coquille Tauri, stockage, écran d'import, écran de réglage de série | 4.1, 4.4, 8 |
| 4 | Écran de révision : synchro par ancres, points à vérifier, validation, livre figé | 4.5 |
| 5 | OCR embarqué (Tesseract), empaquetage AppImage | 3, 4.2, 5.1, 5.11 |
| 6 | Colonnes et alternance, tableaux HTML, schémas en image, zones tracées en révision | 5.4, 5.7, 5.8 |

## Contraintes globales

- **Le dépôt est public : aucun PDF ni EPUB n'est versionné.** Le `.gitignore` exclut
  `*.pdf`, `*.epub` et `vendor/`. Les PDF de test sont fabriqués par le code de test ;
  le corpus est lu via la variable `PLUMEO_CORPUS` (= `/home/stef/Documents/RAG`).
- Les PDF d'origine ne sont jamais modifiés ni copiés.
- Aucune dépendance à un service en ligne ni à un outil externe à l'exécution. pdfium
  est une bibliothèque chargée dynamiquement. epubcheck (Java) ne sert qu'en test.
- Aucun texte retiré sans trace : tout retrait figure dans `retraits` avec son ancre ;
  tout retrait douteux produit aussi un `PointAVerifier`.
- Noms de fichiers comparés et produits en Unicode NFC.
- Langue par défaut `fra`.
- Règles reprises de Lumosphère :
  - corps = taille dominante en volume de caractères ;
  - tolérance de taille 0,5 pt ;
  - titre ≤ 12 mots ;
  - dernière ligne d'un paragraphe = blanc à droite > 15 % de la largeur du bloc.
- Les identifiants Rust et les messages sont en français, comme le reste du projet.
- Chaque commit se termine par `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
  **Aucun `git push`** sans accord explicite du porteur.

## Points de vigilance pour la revue

1. **PDF illisible** (corrompu, chiffré, ou pas un PDF) : on attend une erreur claire
   `ErreurMoteur::PdfIllisible`, sans panique. Test dans la tâche 1.
2. **Caractères parasites renvoyés par pdfium** (caractères de contrôle, U+FFFE,
   ligatures ﬁ/ﬂ) : un caractère de contrôle rend l'XHTML invalide et l'EPUB est
   refusé. On attend la suppression des caractères de contrôle et la décomposition des
   ligatures. Test dans la tâche 1.
3. **Texte qui ressemble à du Markdown ou à du HTML** (`# 3`, `1. Visite`, `*`, `<`,
   `&`, `[^`) : sans échappement, on obtient de faux titres, de fausses listes ou un
   XHTML cassé. On attend du texte littéral jusque dans l'EPUB. Test dans la tâche 7.
4. **Page sans aucun texte** au milieu d'un livre (page blanche, illustration) : on
   attend que la conversion continue, avec un point `PageSansTexte`. Test dans la tâche 7.
5. **Nom de sortie dangereux ou en NFD** (`/`, `:`, accents décomposés) : on attend un
   nom NFC sans séparateur de chemin. Test dans la tâche 9.

---

## Format du Markdown interne (décidé ici, section 14 du document)

```markdown
<!-- @p12 120,145,880,210 -->
## Titre du chapitre

<!-- @p12 120,220,880,400 -->
Texte avec *italique*, **gras**, un exposant 1<sup>er</sup> et un appel[^7-12].

<!-- @p12 120,850,880,900 -->
[^7-12]: Texte de la note.
```

**Ancres :**
- Chaque bloc est précédé, sur sa propre ligne, d'un commentaire d'ancre :
  `@p<page>` (page PDF, base 1), suivi de `g` ou `d` pour une moitié de double page,
  puis du rectangle `x0,y0,x1,y1` en millièmes de la (demi-)page, origine en haut à
  gauche.
- Un commentaire HTML sur sa propre ligne est un bloc HTML de type 2 en CommonMark : le
  bloc suivant reste intact.

**Notes :**
- Leur étiquette `<n>-<marqueur>` combine un numéro de séquence unique dans le livre et
  le marqueur d'origine (`12`, `*`), qui est celui qu'on affiche.
- Les définitions se placent à la fin du chapitre, juste avant le titre de niveau 1
  suivant ou à la fin du fichier.

**Titres :** `#`, `##` ou `###`. Le niveau 1 découpe les chapitres de l'EPUB.

---

### Tâche 1 : Espace de travail, pdfium et lecture d'un PDF

**Fichiers :**
- Créer : `Cargo.toml` (workspace, membres `crates/*`), `.gitignore`, `scripts/recuperer-pdfium.sh`
- Créer : `crates/moteur/Cargo.toml`, `crates/moteur/src/lib.rs`, `crates/moteur/src/modele.rs`, `crates/moteur/src/pdf.rs`, `crates/moteur/src/fixtures.rs`
- Test : `crates/moteur/tests/lecture.rs`

**Interfaces :**
- Produit (`modele.rs`) :
  ```rust
  #[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
  pub struct Rect { pub x0: f32, pub y0: f32, pub x1: f32, pub y1: f32 } // fractions 0..1, origine haut-gauche
  #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
  pub enum Moitie { Entiere, Gauche, Droite }
  #[derive(Clone, Debug, PartialEq)]
  pub struct Mot { pub texte: String, pub rect: Rect, pub taille: f32, pub gras: bool, pub italique: bool, pub espace_avant: bool }
  #[derive(Clone, Debug)]
  pub struct Page { pub numero: u32, pub moitie: Moitie, pub largeur_pt: f32, pub hauteur_pt: f32, pub mots: Vec<Mot> }
  #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
  pub struct Ancre { pub page: u32, pub moitie: Moitie, pub rect: [u16; 4] } // millièmes
  impl Ancre { pub fn depuis(page: u32, moitie: Moitie, r: Rect) -> Ancre }
  impl std::fmt::Display for Ancre   // "@p12 120,145,880,210" / "@p6g ..."
  impl std::str::FromStr for Ancre
  #[derive(Clone, Debug)] pub struct Signet { pub titre: String, pub niveau: u8, pub page: u32 }
  #[derive(Clone, Debug, Default)] pub struct MetaPdf { pub titre: String, pub auteur: String, pub producteur: String }
  #[derive(Debug, thiserror::Error)] pub enum ErreurMoteur { PdfIllisible(String), PdfiumIntrouvable(String), Page(u32, String), Reglages(String) }
  ```
- Produit (`pdf.rs`) :
  ```rust
  pub struct DocumentPdf { /* PdfDocument<'static> */ }
  impl DocumentPdf {
      pub fn ouvrir(chemin: &Path) -> Result<DocumentPdf, ErreurMoteur>;
      pub fn nb_pages(&self) -> u32;
      pub fn page(&self, numero: u32) -> Result<Page, ErreurMoteur>; // base 1, moitie = Entiere
      pub fn signets(&self) -> Vec<Signet>;      // aplatis, niveau 1 = racine
      pub fn meta(&self) -> MetaPdf;
      pub fn rendre_png(&self, numero: u32, largeur_px: u32) -> Result<Vec<u8>, ErreurMoteur>;
  }
  pub fn nettoyer_texte(s: &str) -> String;
  ```
- Produit (`fixtures.rs`, sous la feature `fixtures`, réutilisée par les autres crates en test) :
  ```rust
  pub enum StyleFixture { Normal, Gras, Italique }
  pub struct TexteFixture { pub x_pt: f32, pub y_pt: f32 /* depuis le haut */, pub texte: &'static str, pub taille: f32, pub style: StyleFixture }
  pub fn fabriquer_pdf(pages: &[&[TexteFixture]]) -> tempfile::TempPath; // A4, 595×842 pt, Helvetica
  ```

- [ ] **Étape 1 : poser l'espace de travail et récupérer pdfium**

`scripts/recuperer-pdfium.sh` télécharge
`https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F8125/pdfium-linux-x64.tgz`
et le décompresse dans `vendor/pdfium/`, ce qui donne `vendor/pdfium/lib/libpdfium.so`.
Le script ne fait rien si le fichier est déjà là.

Le `.gitignore` contient `target/`, `vendor/`, `*.pdf` et `*.epub`.

Dans `crates/moteur/Cargo.toml` : `pdfium-render = { version = "0.9.5", default-features = false, features = ["pdfium_8125", "thread_safe", "image_latest"] }` et une feature `fixtures = ["dep:tempfile"]`.

Lancer : `bash scripts/recuperer-pdfium.sh && ls vendor/pdfium/lib/libpdfium.so`
Attendu : le chemin s'affiche.

- [ ] **Étape 2 : écrire les tests qui échouent**

```rust
// crates/moteur/tests/lecture.rs  (cargo test -p plumeo-moteur --features fixtures --test lecture)
#[test] fn lit_les_mots_avec_position_et_taille() {
    let pdf = fabriquer_pdf(&[&[TexteFixture { x_pt: 72.0, y_pt: 100.0, texte: "Bonjour le monde", taille: 12.0, style: StyleFixture::Normal }]]);
    let doc = DocumentPdf::ouvrir(&pdf).unwrap();
    assert_eq!(doc.nb_pages(), 1);
    let p = doc.page(1).unwrap();
    let textes: Vec<_> = p.mots.iter().map(|m| m.texte.as_str()).collect();
    assert_eq!(textes, ["Bonjour", "le", "monde"]);
    assert!((p.mots[0].taille - 12.0).abs() <= 0.5);
    assert!((p.mots[0].rect.x0 - 72.0 / 595.0).abs() < 0.01);
    assert!(p.mots[0].rect.y0 < 100.0 / 842.0 && p.mots[0].rect.y1 > 90.0 / 842.0);
    assert!(!p.mots[0].espace_avant && p.mots[1].espace_avant);
}
#[test] fn reconnait_gras_et_italique() { /* un mot Gras, un mot Italique → gras / italique vrais, l'autre faux */ }
#[test] fn pdf_illisible_donne_une_erreur() {
    let f = tempfile::NamedTempFile::new().unwrap(); std::fs::write(f.path(), b"pas un pdf").unwrap();
    assert!(matches!(DocumentPdf::ouvrir(f.path()), Err(ErreurMoteur::PdfIllisible(_))));
}
#[test] fn nettoie_les_caracteres_parasites() {
    assert_eq!(nettoyer_texte("e\u{fb01}cace\u{2}\u{fffe}"), "efficace");
    assert_eq!(nettoyer_texte("Cite\u{301}"), "Cité"); // NFC
}
#[test] fn ancre_aller_retour() {
    let a: Ancre = "@p6g 120,145,880,210".parse().unwrap();
    assert_eq!((a.page, a.moitie, a.rect), (6, Moitie::Gauche, [120, 145, 880, 210]));
    assert_eq!(a.to_string(), "@p6g 120,145,880,210");
}
#[test] fn rend_une_page_en_png() { /* rendre_png(1, 300) commence par la signature PNG \x89PNG */ }
```

Lancer : `cargo test -p plumeo-moteur --features fixtures --test lecture`
Attendu : ÉCHEC (types absents).

- [ ] **Étape 3 : écrire `pdf.rs`, `modele.rs` et `fixtures.rs`**

**Chargement de pdfium :** une seule fois, dans un `OnceLock<Pdfium>`. On cherche
`libpdfium` dans cet ordre :
1. le dossier donné par `PLUMEO_PDFIUM` ;
2. le dossier de l'exécutable ;
3. `concat!(env!("CARGO_MANIFEST_DIR"), "/../../vendor/pdfium/lib")`.

Si aucun ne contient la bibliothèque : `ErreurMoteur::PdfiumIntrouvable`, avec les
chemins essayés.

**Des caractères aux mots :** on parcourt les caractères de la page
(`page.text()?.chars()`). Un nouveau mot commence dans trois cas :
- après une espace ;
- quand l'écart horizontal dépasse 0,25 × la taille du caractère ;
- quand le gras, l'italique ou la taille change (à plus de 0,5 pt près).

`espace_avant` vaut vrai si une espace ou un écart précède le mot.

**Gras et italique :**
- **gras** : poids de police ≥ 600, ou nom de police contenant `Bold`, `Black`,
  `Heavy` ou `Semibold` ;
- **italique** : nom contenant `Italic` ou `Oblique`, ou drapeau italique de la police.

**Coordonnées :** pdfium place l'origine en bas à gauche. On convertit avec
`y_haut = 1 − top/hauteur`.

**Nettoyage du texte (`nettoyer_texte`) :**
- supprimer les caractères de contrôle (sauf l'espace), U+FFFE et U+FFFF ;
- décomposer les ligatures U+FB00 à U+FB06 ;
- normaliser en NFC.

**Signets :** parcours en profondeur de `bookmarks()`. La page cible se lit dans
l'action ou la destination ; un signet sans page cible est ignoré.

**`rendre_png`** passe par `render_with_config(...).as_image()` et encode en PNG.

- [ ] **Étape 4 : vérifier**

Lancer : `cargo test -p plumeo-moteur --features fixtures --test lecture`
Attendu : 6 tests réussis.

- [ ] **Étape 5 : commiter**

```bash
git add Cargo.toml .gitignore scripts crates/moteur
git commit -m "Moteur : lecture des mots d'un PDF via pdfium"
```

---

### Tâche 2 : Mots → lignes, avec exposants

**Fichiers :**
- Créer : `crates/moteur/src/lignes.rs`
- Test : tests unitaires dans le même fichier (`#[cfg(test)]`)

**Interfaces :**
- Utilise : `Mot` et `Rect` (tâche 1).
- Produit :
  ```rust
  #[derive(Clone, Debug)] pub struct Ligne { pub mots: Vec<Mot>, pub exposants: Vec<usize> /* indices dans mots */, pub rect: Rect }
  impl Ligne { pub fn texte(&self) -> String; pub fn taille(&self) -> f32 /* médiane hors exposants */; pub fn tout_gras(&self) -> bool }
  pub fn regrouper_en_lignes(mots: &[Mot]) -> Vec<Ligne>; // triées de haut en bas
  ```

- [ ] **Étape 1 : écrire les tests qui échouent**

Les tests construisent des `Mot` à la main. Une petite fonction `mot(texte, x0, y0, x1, y1, taille)` sert à les fabriquer.
- `deux_lignes_separees` : 3 mots avec y ≈ 0,10 et 2 mots avec y ≈ 0,13 donnent 2 lignes ; le texte de la première vaut `"a b c"`.
- `ordre_horizontal` : les mots fournis dans le désordre se retrouvent triés par x0.
- `exposant_detecte` : à côté d'un corps en 12 pt dont le bas est à 0,120, un mot « 3 » en 7 pt dont le bas est à 0,114 donne `exposants == [index]`, et `texte()` vaut `"texte3"` (pas d'espace devant un exposant).
- `petit_texte_sur_la_ligne_de_base_n_est_pas_exposant` : un mot en 7 pt dont le bas est à 0,120 n'est pas un exposant.

Lancer : `cargo test -p plumeo-moteur lignes`
Attendu : ÉCHEC.

- [ ] **Étape 2 : écrire `regrouper_en_lignes`**

**Regroupement :**
- On trie les mots par centre vertical.
- Un mot rejoint la ligne courante si son centre vertical tombe entre `y0` et `y1` du plus grand mot de la ligne.
- Dans une ligne, les mots sont triés par `x0`.

**Exposant :** un mot est un exposant si ses deux conditions sont réunies :
- `taille ≤ 0,8 ×` la taille médiane de la ligne ;
- son bas est plus haut que le bas médian de la ligne, d'au moins 25 % de la hauteur de ligne.

**Texte de la ligne (`texte()`) :** les mots sont joints par une espace quand
`espace_avant` est vrai. Un exposant est collé au mot qui le précède.

- [ ] **Étape 3 : vérifier**

Lancer : `cargo test -p plumeo-moteur lignes`
Attendu : 4 tests réussis.

- [ ] **Étape 4 : commiter** (`git commit -m "Moteur : regroupement des mots en lignes"`)

---

### Tâche 3 : Réglages de série, double page et zones

**Fichiers :**
- Créer : `crates/moteur/src/reglages.rs`, `crates/moteur/src/double_page.rs`, `crates/moteur/src/zones.rs`
- Test : tests unitaires dans chaque fichier

**Interfaces :**
- Utilise : `Page`, `Ligne`, `Moitie`, `Ancre` (tâches 1 et 2).
- Produit :
  ```rust
  // reglages.rs — sérialisé en JSON (fichiers de série)
  #[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
  pub struct Zones { pub entete: f32, pub pied: f32, pub notes: f32 } // hauteurs en fraction de page ; notes = bande au-dessus du pied ; 0 = aucune
  #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
  #[serde(rename_all = "kebab-case")]
  pub enum SourceTitres { #[default] Auto, Signets, GrasMajuscules, Taille }
  #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
  pub struct ReglagesSerie {
      pub nom: String,
      #[serde(default = "fra")] pub langue: String,
      pub paire: Zones, pub impaire: Zones,
      #[serde(default)] pub double_page: Option<f32>, // position du pli, fraction de largeur
      #[serde(default)] pub titres: SourceTitres,
      #[serde(default)] pub exceptions: BTreeMap<String, Zones>, // clé "12", "6g", "6d"
  }
  impl ReglagesSerie { pub fn lire(chemin: &Path) -> Result<Self, ErreurMoteur>; pub fn zones_pour(&self, page: u32, moitie: Moitie) -> &Zones }
  // double_page.rs
  pub fn couper_double_page(page: Page, pli: f32) -> [Page; 2]; // [Gauche, Droite]
  // zones.rs
  #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)] #[serde(rename_all = "kebab-case")]
  pub enum Role { EnTete, Pied, NumeroPage, TitreNotes }
  #[derive(Clone, Debug, Serialize)] pub struct Retrait { pub ancre: Ancre, pub role: Role, pub texte: String }
  pub struct PageClassee { pub corps: Vec<Ligne>, pub notes: Vec<Ligne>, pub retraits: Vec<Retrait>, pub numero_residuel: bool }
  pub fn classer_lignes(page: &Page, lignes: Vec<Ligne>, zones: &Zones) -> PageClassee;
  ```

- [ ] **Étape 1 : écrire les tests qui échouent**
- `zones_par_parite` : `zones_pour(12, Entiere)` renvoie `paire` et `zones_pour(13, Entiere)` renvoie `impaire`. Pour une double page, la moitié `Gauche` est paire et la moitié `Droite` impaire.
- `exception_prioritaire` : avec `exceptions["29"]`, `zones_pour(29, Entiere)` renvoie l'exception ; `zones_pour(6, Gauche)` lit la clé `"6g"`.
- `json_minimal` : `{"nom":"x","paire":{"entete":0.07,"pied":0,"notes":0},"impaire":{"entete":0.07,"pied":0,"notes":0}}` se lit avec `langue == "fra"`, `titres == Auto` et pas de double page.
- `coupe_au_pli` : sur une page de 842 × 595 pt, un mot dont le centre est à x = 0,25 va à gauche avec x recalculé à ≈ 0,5, et un mot dont le centre est à x = 0,75 va à droite à ≈ 0,5. Chaque demi-page fait 421 pt de large et garde son `numero`.
- `classe_par_centre` : avec `entete = 0.07`, `pied = 0.05` et `notes = 0.10`, les lignes centrées à y = 0,03, 0,50, 0,90 et 0,97 sont classées respectivement en retrait `EnTete`, corps, note et retrait `Pied`.
- `numero_residuel_retire_et_signale` : une dernière ligne de corps « - 12 - » ou « 92 93 » passe en retrait `NumeroPage`, avec `numero_residuel == true`. Une ligne « 12 juin 1924 » reste dans le corps.

Lancer : `cargo test -p plumeo-moteur reglages double_page zones`
Attendu : ÉCHEC.

- [ ] **Étape 2 : écrire les trois modules**

**Classement d'une ligne**, selon son centre vertical `c` :
- `c < entete` : retrait `EnTete` ;
- `c > 1 − pied` : retrait `Pied` ;
- `notes > 0` et `c > 1 − pied − notes` : note ;
- sinon : corps.

**Numéro de page résiduel :**
- On teste seulement la première et la dernière ligne du corps.
- Le motif de numéro de page reprend `PAGE_NUM_RE` de `pdf_to_md.py`, complété par `\d+\s+\d+` (pied « 92 93 » du n°6) :
  ```
  ^\s*(?:Page\s+\d+\s+(?:de|sur|/)\s+\d+|\d+\s*/\s*\d+|\.{2,}\s*/\s*\d+|-\s*\d+\s*-|\d+(?:\s+\d+)?)\s*$   (insensible à la casse)
  ```

**Ancre d'un retrait :** `Ancre::depuis(page.numero, page.moitie, ligne.rect)`.

- [ ] **Étape 3 : vérifier**

Lancer : `cargo test -p plumeo-moteur reglages double_page zones`
Attendu : 6 tests réussis.

- [ ] **Étape 4 : commiter** (`git commit -m "Moteur : réglages de série, double page et zones"`)

---

### Tâche 4 : Lignes → paragraphes

**Fichiers :**
- Créer : `crates/moteur/src/blocs.rs` (types), `crates/moteur/src/paragraphes.rs`
- Test : tests unitaires dans `paragraphes.rs`

**Interfaces :**
- Utilise : `Ligne` (tâche 2), `Ancre` (tâche 1).
- Produit :
  ```rust
  // blocs.rs
  #[derive(Clone, Debug, PartialEq)]
  pub enum Segment { Texte { texte: String, gras: bool, italique: bool }, Exposant(String), AppelNote { etiquette: String, marqueur: String } }
  #[derive(Clone, Debug)]
  pub struct Paragraphe { pub segments: Vec<Segment>, pub ancre: Ancre, pub taille: f32, pub tout_gras: bool, pub nb_lignes: usize }
  impl Paragraphe { pub fn texte_brut(&self) -> String }
  #[derive(Clone, Debug)]
  pub enum Bloc {
      Paragraphe(Paragraphe),
      Titre { niveau: u8, texte: String, ancre: Ancre },
      Note { etiquette: String, marqueur: String, segments: Vec<Segment>, ancre: Ancre },
  }
  impl Bloc { pub fn ancre(&self) -> Ancre }
  // paragraphes.rs
  #[derive(Clone, Copy, Debug)] pub struct BlocTexte { pub x0: f32, pub x1: f32, pub interligne: f32 } // mesuré sur le livre
  pub fn mesurer_bloc(pages: &[Vec<Ligne>]) -> BlocTexte;
  pub fn paragraphes_page(lignes: &[Ligne], page: u32, moitie: Moitie, bloc: BlocTexte) -> Vec<Paragraphe>;
  pub fn raccorder_pages(blocs: Vec<Bloc>) -> Vec<Bloc>;
  ```

- [ ] **Étape 1 : écrire les tests qui échouent**
- `fin_de_paragraphe_par_blanc_a_droite` : une ligne pleine, puis une ligne qui s'arrête à 70 % de la largeur du bloc, puis une ligne pleine, donnent 2 paragraphes.
- `ecart_vertical` : un écart de 2 × l'interligne entre deux lignes pleines coupe le paragraphe.
- `alinea` : une ligne qui commence 5 % plus à droite que `bloc.x0` ouvre un paragraphe.
- `changement_de_taille` : une ligne en 19 pt suivie d'une ligne en 15 pt donne 2 paragraphes.
- `cesure_recollee` : « mer- » + « veilleux » donne « merveilleux ». « peut- » + « Être » reste « peut- Être », car la ligne suivante commence par une majuscule.
- `trait_d_union_conditionnel` : « mer\u{AD} » + « veilleux » donne « merveilleux », et un U+00AD au milieu d'une ligne disparaît.
- `segments_riches` : des mots normaux, puis italiques, puis normaux donnent 3 segments `Texte` ; un exposant donne `Segment::Exposant`.
- `raccord_inter_pages` : un paragraphe de la page 4 qui finit par « que » et un paragraphe de la page 5 qui commence par « l'on » ne font plus qu'un, avec l'ancre de la page 4. Si le paragraphe de la page 4 finit par « . », on ne raccorde pas ; si le bloc suivant est un `Titre`, non plus.

Lancer : `cargo test -p plumeo-moteur paragraphes`
Attendu : ÉCHEC.

- [ ] **Étape 2 : écrire `paragraphes.rs`**

**Mesure du bloc de texte (`mesurer_bloc`) :**
- `x0` et `x1` : médianes des minimums et maximums par page, sur les pages d'au moins 10 lignes de corps ;
- `interligne` : médiane des écarts `y0` entre lignes consécutives d'une même page.

**Coupures :** on coupe avant la ligne L dès qu'un de ces cas se présente :
1. le blanc à droite de la ligne précédente, `bloc.x1 − prec.rect.x1`, dépasse 15 % de `bloc.x1 − bloc.x0` ;
2. `L.y0 − prec.y0 > 1,5 × interligne` ;
3. `L.x0 − bloc.x0 > 3 %` de la largeur du bloc, alors que la ligne précédente n'est pas elle-même en retrait ;
4. `|taille(L) − taille(prec)| > 0,5` pt, ou `tout_gras` différent.

**Jonction des lignes :**
- le trait d'union conditionnel U+00AD est toujours supprimé ;
- `-` ou `‐` en fin de ligne, précédé d'une lettre, puis une ligne qui commence par une minuscule : on retire le tiret et on colle les deux lignes ;
- sinon, on joint par une espace.

**Segments :** les segments voisins de même style sont fusionnés.

**Ponctuation finale (`raccorder_pages`) :** `. ! ? : » … ) " ”`.

- [ ] **Étape 3 : vérifier**

Lancer : `cargo test -p plumeo-moteur paragraphes`
Attendu : 8 tests réussis.

- [ ] **Étape 4 : commiter** (`git commit -m "Moteur : paragraphes, césures et raccord entre pages"`)

---

### Tâche 5 : Titres en cascade et sommaire

**Fichiers :**
- Créer : `crates/moteur/src/titres.rs`, `crates/moteur/src/points.rs`
- Test : tests unitaires dans `titres.rs`

**Interfaces :**
- Utilise : `Bloc`, `Paragraphe` (tâche 4), `Signet` (tâche 1), `SourceTitres` (tâche 3).
- Produit :
  ```rust
  // points.rs
  #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)] #[serde(rename_all = "kebab-case")]
  pub enum GenrePoint { TitreSuspect, NumeroPageResiduel, NoteOrpheline, Sommaire, PageSansTexte }
  #[derive(Clone, Debug, Serialize)] pub struct PointAVerifier { pub ancre: Ancre, pub genre: GenrePoint, pub message: String }
  // titres.rs
  pub fn taille_corps(blocs: &[Bloc]) -> f32; // taille dominante pondérée par le nombre de caractères
  pub fn signets_utilisables(signets: &[Signet], nb_pages: u32) -> bool;
  pub fn detecter_titres(blocs: Vec<Bloc>, signets: &[Signet], nb_pages: u32, source: SourceTitres) -> (Vec<Bloc>, Vec<PointAVerifier>, SourceTitres /* effective */);
  pub fn normaliser(s: &str) -> String; // minuscules, sans diacritiques, ponctuation → espace, espaces réduits
  ```

- [ ] **Étape 1 : écrire les tests qui échouent**
- `signets_rejetes_si_un_par_page` : 207 signets pour 207 pages donnent `false` ; 96 signets « Page N » pour 96 pages, `false` ; 73 signets pour 136 pages, `true`.
- `titre_depuis_signet` : un signet « 1. VISITE À PATMOS » (niveau 1, page 3) et un paragraphe page 3 « 1. VISITE À PATMOS » donnent `Titre { niveau: 1, texte: "1. VISITE À PATMOS" }`.
- `titre_multiligne_depuis_signet` : avec un signet « LE SENS DE L’AMOUR DANS LE MONDE », les deux paragraphes « LE SENS DE L’AMOUR » et « DANS LE MONDE » donnent un seul titre.
- `signet_introuvable` : le titre est inséré en tête de la page, avec le texte du signet, et un point `TitreSuspect` est produit.
- `gras_majuscules` : 10 paragraphes « CHAPITRE n: » en gras majuscules (≤ 12 mots) deviennent des titres de niveau 1 ; un paragraphe gras mais en minuscules reste un paragraphe ; la source effective est `GrasMajuscules`.
- `par_taille_niveaux` : avec un corps à 12 pt, les paragraphes courts à 18 pt passent en niveau 1 et ceux à 14 pt en niveau 2 ; un paragraphe de 20 mots à 18 pt reste un paragraphe.
- `titres_consecutifs_fusionnes` : « CHAPITRE 1: » puis « LES ANGES », de même niveau sur la même page, donnent « CHAPITRE 1: LES ANGES ».
- `sommaire_non_titre` : une page dont 6 paragraphes finissent par un numéro (« Visite à Patmos ...... 12 ») ne produit aucun titre, et donne un point `Sommaire`.
- `titre_courant_suspect` : un même titre normalisé détecté sur au moins 5 pages donne un point `TitreSuspect` pour chacun.

Lancer : `cargo test -p plumeo-moteur titres`
Attendu : ÉCHEC.

- [ ] **Étape 2 : écrire `titres.rs`**

**Signets utilisables :** il faut réunir trois conditions :
- au moins 3 signets, pointant vers au moins 3 pages distinctes ;
- moins de signets que 0,9 × le nombre de pages ;
- moins de 50 % des titres de signets correspondent à `^(page|p\.?)\s*\d+$|^\d+$|\.(jpe?g|png|tiff?)$`.

**Mode `Auto` :** on prend les signets s'ils sont utilisables, sinon le gras et les majuscules s'il y a au moins 3 candidats dans le livre, sinon la taille.

**Candidats :**
- *gras + majuscules* : `tout_gras`, au moins 80 % des lettres en majuscules, ≤ 12 mots et ≤ 3 lignes ;
- *taille* : `taille ≥ corps + 1,5` pt, ≤ 12 mots et ≤ 3 lignes.

**Niveaux** (gras + majuscules et taille) : les tailles des candidats sont arrondies à 0,5 pt et classées de la plus grande à la plus petite. La plus grande donne le niveau 1, la suivante le niveau 2, les autres le niveau 3.

**Signets :**
- niveau = niveau du signet (plafonné à 3) ;
- on cherche sur la page cible, puis sur la suivante, le premier paragraphe dont le texte normalisé commence par le titre normalisé, ou en est le début ;
- dans ce second cas, on avale les paragraphes suivants jusqu'à couvrir le titre ;
- le texte retenu est celui du PDF.

**Sommaire :** une page où au moins 4 paragraphes, et au moins 50 % d'entre eux, finissent par `\s\d{1,4}$` ou contiennent `\.{3,}`.

- [ ] **Étape 3 : vérifier**

Lancer : `cargo test -p plumeo-moteur titres`
Attendu : 9 tests réussis.

- [ ] **Étape 4 : commiter** (`git commit -m "Moteur : titres en cascade et détection du sommaire"`)

---

### Tâche 6 : Notes en bas de page et notes regroupées

**Fichiers :**
- Créer : `crates/moteur/src/notes.rs`
- Test : tests unitaires dans le même fichier

**Interfaces :**
- Utilise : `Ligne`, `Bloc`, `Segment`, `PointAVerifier`, `normaliser` (tâches 2, 4 et 5).
- Produit :
  ```rust
  pub struct Etiqueteuse { suivant: u32 } impl Etiqueteuse { pub fn new() -> Self; pub fn etiquette(&mut self, marqueur: &str) -> String /* "7-12" */ }
  pub fn notes_bas_de_page(paragraphes: Vec<Paragraphe>, lignes_notes: &[Ligne], page: u32, moitie: Moitie, et: &mut Etiqueteuse)
      -> (Vec<Bloc>, Vec<PointAVerifier>);
  pub fn notes_regroupees(blocs: Vec<Bloc>, et: &mut Etiqueteuse) -> (Vec<Bloc>, Vec<Retrait>, Vec<PointAVerifier>);
  ```

- [ ] **Étape 1 : écrire les tests qui échouent**
- `note_etoile_appariee` : un paragraphe contenant `Exposant("*")` sur une page dont la zone de notes contient « * Lire la conférence… » donne un `Segment::AppelNote { marqueur: "*" }` et un `Bloc::Note` de même étiquette.
- `appel_etoile_colle_au_mot` : « double* » (étoile en texte normal collée au mot) compte aussi comme appel.
- `note_sur_deux_lignes` : « 1 Voir… » puis « suite de la note » donnent une seule note.
- `appel_sans_note` : `Exposant("2")` sans note reste un `Exposant` et produit un point `NoteOrpheline`.
- `note_sans_appel` : la note est conservée en `Bloc::Paragraphe` « 3 texte », avec un point `NoteOrpheline`.
- `liste_numerotee_hors_zone_reste_texte` : des paragraphes de corps « 1. … » et « 2. … » sans exposant ne sont pas des notes.
- `section_notes_regroupees` : appels `Exposant("1")` et `Exposant("2")` dans le chapitre, puis un `Titre` « NOTES », puis les paragraphes « 1 Texte un » et « 2 Texte deux » : on obtient 2 `Bloc::Note` appariées, et le titre « NOTES » passe en retrait `TitreNotes`.
- `numerotation_qui_repart` : deux chapitres, chacun avec un appel « 1 » et sa section de notes, donnent deux étiquettes différentes, chacune appariée dans son chapitre.

Lancer : `cargo test -p plumeo-moteur notes`
Attendu : ÉCHEC.

- [ ] **Étape 2 : écrire `notes.rs`**

**Début de note** (lignes de la zone de notes) : la ligne commence par
`^(\d{1,3}|\*{1,3})[\s.)]`, ou son premier mot est un exposant. Les lignes suivantes
sans marqueur continuent la note.

**Appels :**
- un `Exposant` qui vérifie `^(\d{1,3}|\*{1,3})$` ;
- ou un `*` collé à la fin d'un mot en texte normal.

**Appariement :** un appel est apparié à la note de même marqueur ; pour un marqueur
répété, le premier appel non apparié l'emporte.

**Portée :**
- notes en bas de page : la page ;
- notes regroupées : de la fin de la section « Notes » précédente (ou du début du livre) jusqu'au titre « Notes ».

**Section « Notes » :**
- elle commence à un `Titre` ou à un paragraphe dont le texte normalisé vaut `notes` ;
- elle finit au titre suivant ;
- un paragraphe sans marqueur prolonge la note précédente.

- [ ] **Étape 3 : vérifier**

Lancer : `cargo test -p plumeo-moteur notes`
Attendu : 8 tests réussis.

- [ ] **Étape 4 : commiter** (`git commit -m "Moteur : notes de bas de page et notes regroupées"`)

---

### Tâche 7 : Écriture du Markdown et conversion d'un livre

**Fichiers :**
- Créer : `crates/moteur/src/markdown.rs`, `crates/moteur/src/convertir.rs`
- Modifier : `crates/moteur/src/lib.rs` (exports publics)
- Test : `crates/moteur/tests/conversion.rs` (feature `fixtures`), et tests unitaires dans `markdown.rs`

**Interfaces :**
- Utilise : tout ce qui précède.
- Produit :
  ```rust
  // markdown.rs
  pub fn echapper_md(texte: &str, debut_de_bloc: bool) -> String;
  pub fn ecrire_markdown(blocs: &[Bloc]) -> String;
  // convertir.rs
  #[derive(Debug, Serialize)]
  pub struct Conversion { #[serde(skip)] pub markdown: String, pub retraits: Vec<Retrait>, pub points: Vec<PointAVerifier>,
                          pub source_titres: SourceTitres, pub nb_pages: u32, pub nb_notes: usize }
  pub fn convertir_livre(doc: &DocumentPdf, reglages: &ReglagesSerie, forcer_titres: Option<SourceTitres>,
                         progression: &mut dyn FnMut(u32, u32)) -> Result<Conversion, ErreurMoteur>;
  pub struct ApercuPage { pub moities: Vec<(Moitie, String /* texte du corps */, Vec<Retrait>)> }
  pub fn apercu_page(doc: &DocumentPdf, numero: u32, reglages: &ReglagesSerie) -> Result<ApercuPage, ErreurMoteur>;
  ```

- [ ] **Étape 1 : écrire les tests qui échouent**

Tests unitaires de `markdown.rs` :
- `echappement` :
  - `echapper_md("# 3 & <b> *x* [^1]", true)` → `"\\# 3 \\& \\<b\\> \\*x\\* \\[^1\\]"` ;
  - `echapper_md("1. Visite", true)` → `"1\\. Visite"` ;
  - le même texte avec `debut_de_bloc = false` → `"1. Visite"`.
- `style_et_espaces` : les segments `Texte("Il dit ", normal)`, `Texte("vraiment ", italique)` et `Texte("oui", normal)` donnent `Il dit *vraiment* oui`, l'espace étant sortie du marqueur.
- `ancres_et_titres` : `[Titre{2,"Titre"}, Paragraphe]` donne `<!-- @p12 … -->\n## Titre\n\n<!-- @p12 … -->\nTexte\n`.
- `notes_en_fin_de_chapitre` : avec `# A`, un paragraphe contenant un appel `[^1-1]`, `Note 1-1` puis `# B`, la définition `[^1-1]: …` (précédée de son ancre) se place avant `# B`.

Tests d'intégration (`conversion.rs`, PDF fabriqués avec `fabriquer_pdf`) :
- `livre_de_trois_pages` :
  - page 1 : titre 20 pt gras « CHAPITRE UN » ;
  - pages 1 à 3 : corps en 12 pt ;
  - en-tête « Mon livre » à y = 30 pt sur chaque page ;
  - réglages : `entete = 0.06`.

  On attend dans le Markdown `# CHAPITRE UN`, aucune occurrence de « Mon livre », et 3 retraits `EnTete`.
- `page_vide_au_milieu` : avec une page 2 sans texte, la conversion réussit, produit un point `PageSansTexte` ancré page 2, et les paragraphes des pages 1 et 3 sont présents.
- `progression_appelee` : le rappel est appelé avec `(1,3)`, `(2,3)` puis `(3,3)`.
- `apercu_d_une_page` : `apercu_page(doc, 1, …)` renvoie le texte du corps sans « Mon livre », et 1 retrait.

Lancer : `cargo test -p plumeo-moteur --features fixtures markdown --test conversion`
Attendu : ÉCHEC.

- [ ] **Étape 2 : écrire `markdown.rs`**

**Échappement (`echapper_md`) :**
- on fait précéder d'une barre oblique inverse `\ * _ \` [ ] < > &` et `` ` `` ;
- en début de bloc, aussi `# > - + =` et le motif `^\d+[.)]` (on n'échappe alors que le `.` ou la `)`).

**Rendu des segments :**
- `Texte` gras → `**…**`, italique → `*…*`, gras et italique → `***…***` ;
- les espaces de bord sortent des marqueurs ;
- `Exposant` → `<sup>…</sup>` ;
- `AppelNote` → `[^étiquette]`.

**Paragraphes :** chacun s'écrit sur une seule ligne.

- [ ] **Étape 3 : écrire `convertir.rs`**

**Ordre des opérations :**
1. Pour chaque page :
   1. `page()` ;
   2. découpe de la double page si `double_page` ;
   3. `regrouper_en_lignes` ;
   4. `classer_lignes` (zones de `zones_pour`) ;
   5. si `numero_residuel` : point `NumeroPageResiduel`.
2. `mesurer_bloc` sur tout le livre.
3. Pour chaque (demi-)page :
   1. `paragraphes_page` ;
   2. `notes_bas_de_page` ;
   3. si la page n'a ni corps ni note : point `PageSansTexte`.
4. Pour le livre entier, dans cet ordre :
   1. `detecter_titres` ;
   2. `notes_regroupees` ;
   3. `raccorder_pages` ;
   4. `ecrire_markdown`.

**Progression :** appelée après chaque page lue.

**`apercu_page`** applique les étapes 1 et 3 à une seule page, avec un bloc mesuré sur
cette page seule.

- [ ] **Étape 4 : vérifier**

Lancer : `cargo test -p plumeo-moteur --features fixtures`
Attendu : tous les tests du moteur réussissent.

- [ ] **Étape 5 : commiter** (`git commit -m "Moteur : Markdown interne ancré et conversion d'un livre"`)

---

### Tâche 8 : Génération de l'EPUB 3

**Fichiers :**
- Créer : `crates/epub/Cargo.toml`, `crates/epub/src/lib.rs`, `crates/epub/src/chapitres.rs`, `crates/epub/src/xhtml.rs`, `crates/epub/src/paquet.rs`, `crates/epub/src/style.css`
- Test : `crates/epub/tests/epub.rs`

**Interfaces :**
- Utilise : uniquement le texte Markdown au format décrit en tête de plan. La crate ne dépend pas de `plumeo-moteur`.
- Produit :
  ```rust
  pub struct Metadonnees { pub titre: String, pub auteur: String, pub langue: String, pub identifiant: String /* "urn:uuid:…" */, pub modifie: String /* "2026-11-10T12:00:00Z" */ }
  #[derive(Debug, thiserror::Error)] pub enum ErreurEpub { Ecriture(#[from] std::io::Error), Zip(String) }
  pub fn generer_epub(markdown: &str, meta: &Metadonnees, couverture_png: Option<&[u8]>, sortie: &Path) -> Result<(), ErreurEpub>;
  ```

- [ ] **Étape 1 : écrire les tests qui échouent**
- `structure_du_paquet` :
  - la première entrée est `mimetype`, stockée sans compression, de contenu `application/epub+zip` ;
  - on trouve `META-INF/container.xml`, `OEBPS/content.opf` et `OEBPS/nav.xhtml` ;
  - l'OPF contient `dc:title`, `dc:creator`, `dc:language` et `dc:identifier`, ainsi que `dcterms:modified` avec la valeur fournie.
- `un_fichier_par_chapitre` : avec `# A`, `## A1` et `# B`, on obtient `chap-001.xhtml` et `chap-002.xhtml`, et la nav contient A (avec A1 en dessous) puis B.
- `contenu_avant_le_premier_titre` : un Markdown sans `#` donne un seul chapitre, intitulé du titre du livre dans la nav.
- `notes_noteref_et_retour` : `Texte[^3-1].` suivi de `[^3-1]: La note.` donne :
  - `<a epub:type="noteref" id="appel-3" href="#note-3"><sup>1</sup></a>` ;
  - en fin de chapitre, `<aside epub:type="footnote" id="note-3">` qui contient `La note.` et `<a href="#appel-3">↩</a>`.
- `marqueur_etoile` : `[^4-*]` affiche `*`.
- `xhtml_bien_forme` : chaque `.xhtml` et l'OPF se lisent avec `roxmltree` sans erreur, y compris un Markdown qui contient `\& \<b\>`, `<sup>er</sup>` et des guillemets.
- `ancres_absentes` : aucun fichier ne contient `@p`.
- `couverture` : si une couverture PNG est fournie, on trouve `OEBPS/images/couverture.png`, un élément de manifeste qui porte `properties="cover-image"`, et `couverture.xhtml` en tête du spine.
- `epubcheck` (`#[ignore]`) : si `EPUBCHECK_JAR` est défini, `java -jar $EPUBCHECK_JAR livre.epub` renvoie 0, sur un livre qui combine titres, notes et couverture.

Lancer : `cargo test -p plumeo-epub`
Attendu : ÉCHEC.

- [ ] **Étape 2 : écrire la crate**

**Lecture du Markdown :** `pulldown-cmark` avec `ENABLE_FOOTNOTES`. Les commentaires
d'ancre sont des événements `Html` qui commencent par `<!-- @` : on les ignore.

**Découpage :** un chapitre par `Heading(H1)`, plus un chapitre initial s'il y a du
contenu avant le premier H1.

**Notes :**
- `FootnoteReference(label)` donne l'appel. Les identifiants XML viennent de la partie de l'étiquette avant `-`, et le texte affiché de la partie après.
- Les définitions sont rendues dans un `<aside>`, en fin de chapitre.

**XHTML :**
- produit avec `pulldown_cmark::html::push_html` sur les événements filtrés, puis enveloppé dans un document XHTML 1.1/EPUB 3 qui déclare `xmlns:epub` ;
- les balises vides HTML (`<br>`, `<hr>`, `<img>`) sont normalisées en balises auto-fermantes.

**Nav :** H1, avec les H2 imbriqués.

**Paquet :**
- `mimetype` en `CompressionMethod::Stored` ;
- le reste en `Deflated` ;
- feuille de style sobre : corps serif, `text-indent: 1.2em` sur les paragraphes, notes en plus petit.

- [ ] **Étape 3 : vérifier**

Lancer : `cargo test -p plumeo-epub`, puis, si un epubcheck est disponible :
`EPUBCHECK_JAR=… cargo test -p plumeo-epub -- --ignored`.
Attendu : tous les tests réussis.

epubcheck se télécharge depuis `https://github.com/w3c/epubcheck/releases/latest` vers
`vendor/epubcheck/`, qui est ignoré par git.

- [ ] **Étape 4 : commiter** (`git commit -m "EPUB 3 : chapitres, nav, notes avec retour, couverture"`)

---

### Tâche 9 : Métadonnées proposées, nom de sortie et ligne de commande

**Fichiers :**
- Créer : `crates/moteur/src/metadonnees.rs`, `crates/cli/Cargo.toml`, `crates/cli/src/main.rs`
- Test : tests unitaires dans `metadonnees.rs`, et `crates/cli/tests/cli.rs` (dev-dépendance `plumeo-moteur` avec la feature `fixtures`)

**Interfaces :**
- Utilise : `MetaPdf`, `normaliser`, `convertir_livre`, `apercu_page`, `generer_epub`.
- Produit :
  ```rust
  pub fn proposer(meta: &MetaPdf, nom_fichier: &str /* sans extension */) -> (String /* titre */, String /* auteur */);
  pub fn nom_sortie(titre: &str, auteur: &str) -> String;
  ```
- Ligne de commande :
  ```
  plumeo-cli convertir <PDF> --serie <reglages.json> --sortie <DOSSIER>
             [--titres auto|signets|gras-majuscules|taille] [--titre T] [--auteur A]
  plumeo-cli page <PDF> <N> --serie <reglages.json>
  ```

**Fichiers produits par `convertir`**, dans `<DOSSIER>` (créé s'il n'existe pas) :
- `<nom>.epub` ;
- `<nom>.md` ;
- `<nom>.retraits.json` ;
- `<nom>.points.json`.

**Résumé affiché :** pages, source des titres, nombre de titres de niveau 1, notes,
retraits, points à vérifier, durée.

**`page`** affiche, pour chaque moitié, le texte du corps puis les retraits avec leur rôle.

- [ ] **Étape 1 : écrire les tests qui échouent**

`proposer_sur_le_corpus` : les noms réels et les métadonnées mesurées le 2026-11-10 :

| Nom de fichier | Méta titre / auteur | Titre attendu | Auteur attendu |
|---|---|---|---|
| `Aïvanhov, Omraam Mikhaël - Approche de la Cité céleste` | `Aïvanhov, Omraam Mikhaël` / `Approche de la Cité céleste` | `Approche de la Cité céleste` | `Omraam Mikhaël Aïvanhov` |
| `Anges et l’arbre de vie - Omraam Mikhaël Aïvanhov` | `Microsoft Word - F - Les Anges et l'Abre de Vie` / `Windows` | `Anges et l’arbre de vie` | `Omraam Mikhaël Aïvanhov` |
| `L’alchimie_spirituelle_Tome_2_Omraam_Mikhaël_Aïvanhov` | vides | `L’alchimie spirituelle Tome 2 Omraam Mikhaël Aïvanhov` | `Inconnu` |
| `Chronique de l'Akasha - Rudolf Steiner` | `Chronique de l'Akasha` / `Rudolf Steiner` | `Chronique de l'Akasha` | `Rudolf Steiner` |
| `Le sens de l'amour (Rudolf Steiner)` | `Le sens de l'amour` / `Rudolf Steiner` | `Le sens de l'amour` | `Rudolf Steiner` |
| `Steiner Rudolf - Connaissance Logique Pensée pratique` | vides | `Connaissance Logique Pensée pratique` | `Steiner Rudolf` |
| `[02] Lettres Sur La Méditation Occulte - Alice Bailey` | `Livres AAB` / `AAB` | `Lettres Sur La Méditation Occulte` | `Alice Bailey` |
| `Alice Bailey - Initiation humaine et solaire` | `Initiation humaine et solaire: Une connaissance approfondie est révélée (French Edition)` / `Alice Bailey` | `Initiation humaine et solaire: Une connaissance approfondie est révélée` | `Alice Bailey` |
| `guerison esoterique_ Une connai - Alice Bailey` | `guérison ésotérique: Une connaissance approfondie est révélée (French Edition)` / `Alice Bailey` | `guérison ésotérique: Une connaissance approfondie est révélée` | `Alice Bailey` |

Autres tests :
- `nom_sortie_sur` :
  - `nom_sortie("a/b: c", "X")` → `"a_b_ c - X"` ;
  - un titre en NFD donne un résultat NFC ;
  - la longueur est d'au plus 150 caractères.
- `cli_convertit_un_pdf_fabrique` (`crates/cli/tests/cli.rs`) : sur un PDF fabriqué avec `fabriquer_pdf`, comme dans `livre_de_trois_pages` de la tâche 7, et un fichier de série écrit dans un dossier temporaire, `CARGO_BIN_EXE_plumeo-cli convertir …` renvoie 0, produit les 4 fichiers, et l'EPUB s'ouvre comme une archive zip.

Lancer : `cargo test -p plumeo-moteur metadonnees && cargo test -p plumeo-cli`
Attendu : ÉCHEC.

- [ ] **Étape 2 : écrire `proposer`**

**Préparation du nom de fichier :**
- normaliser en NFC ;
- retirer en tête les préfixes répétés `^[\[(]?\d+[\])]?[\s_-]+` ;
- remplacer `_` par une espace et réduire les espaces.

**Séparation titre / auteur :**
- `(Auteur)` en fin de nom ;
- sinon, la première occurrence de ` - ` ;
- sinon, tout le nom est le titre et l'auteur est inconnu.

**Côté auteur :** est candidat un côté de 2 à 4 mots qui commencent tous par une
majuscule, ou qui contient une virgule. Si les deux côtés sont candidats, ou aucun, l'auteur est à droite.

**Forme « Nom, Prénom » :** elle devient « Prénom Nom ».

**Titre des métadonnées :** il remplace celui du nom de fichier quand trois conditions sont réunies :
- son texte normalisé commence par le titre normalisé du nom de fichier ;
- il est plus long ;
- on lui a retiré une parenthèse finale contenant `dition`.

**Auteur des métadonnées :** il ne sert que si le nom de fichier n'en donne pas, et seulement s'il compte au moins 2 mots. Sinon, l'auteur est `Inconnu`.

**Nom de sortie :**
- `{titre} - {auteur}` en NFC ;
- `/ \ : * ? " < > |` remplacés par `_` ;
- caractères de contrôle supprimés ;
- tronqué à 150 caractères.

- [ ] **Étape 3 : écrire `main.rs` avec clap**

**Couverture :** `rendre_png(1, 1200)`.

**Identifiant :** `urn:uuid:` suivi d'un UUID v5 (espace de noms URL) calculé sur le
nom de fichier normalisé.

**`modifie` :** l'heure UTC courante au format `%Y-%m-%dT%H:%M:%SZ`. Le calcul se fait
à partir de `SystemTime`, sans dépendance de date.

**Langue :** `Metadonnees.langue` vient de `reglages.langue`, convertie du code ISO 639-3 de Tesseract au code BCP 47 attendu par l'EPUB (`fra` → `fr`, `deu` → `de`, `eng` → `en` ; sinon la valeur telle quelle). Un test unitaire couvre `fra` → `fr`.

**Code de retour :** 1, avec le message de l'erreur, si une erreur survient.

- [ ] **Étape 4 : vérifier**

Lancer : `cargo test -p plumeo-moteur metadonnees && cargo test -p plumeo-cli`
Attendu : tous les tests réussis.

- [ ] **Étape 5 : commiter** (`git commit -m "CLI : convertir et page ; métadonnées proposées"`)

---

### Tâche 10 : Réglages du corpus et tests de non-régression

**Fichiers :**
- Créer : `tests/corpus/series/*.json`, `tests/corpus/manifeste.json`, `crates/moteur/tests/corpus.rs`, `docs/corpus.md`

**Interfaces :**
- Utilise : `ReglagesSerie::lire`, `convertir_livre` (et `generer_epub` en dev-dépendance pour epubcheck).
- Produit : le manifeste, au format suivant (un objet par livre) :
  ```json
  { "fichier": "omraam/pdf/Aïvanhov, Omraam Mikhaël - Approche de la Cité céleste.pdf",
    "serie": "calibre-lettre", "titres": "auto",
    "attentes": { "source_titres": "signets", "titres_n1": 73, "titres_contiennent": ["VISITE À PATMOS"],
                  "prefixe_n1": null, "n1_min": null, "n2": null, "notes_pages": [], "sans_notes_pages": [],
                  "absent_regex": [], "moities": false } }
  ```
  `fichier` est relatif à `PLUMEO_CORPUS`. On le retrouve en comparant les chemins
  normalisés en NFC, avec l'apostrophe ’ ramenée à ' : les noms réels mélangent NFC et NFD.

- [ ] **Étape 1 : écrire le test de non-régression**

`#[test] #[ignore] fn corpus()` : si `PLUMEO_CORPUS` n'est pas défini, le test échoue
avec un message explicite. Pour chaque entrée du manifeste, il convertit le livre et
vérifie deux groupes d'attentes.

**Attentes communes à chaque livre :**
- aucun bloc dont le texte entier correspond au motif de numéro de page de la tâche 3 ;
- longueur médiane des paragraphes entre 80 et 1 500 caractères, maximum ≤ 8 000 ;
- ancres croissantes en (page, moitié) ;
- au moins 1 retrait si la série a des zones non nulles ;
- epubcheck sans erreur si `EPUBCHECK_JAR` est défini.

**Attentes propres à chaque livre**, d'après la section 12.1 du document et les mesures
du 2026-11-10 :

| # | Attentes |
|---|---|
| 1 | source `signets`, 73 titres de niveau 1, dont « VISITE À PATMOS » |
| 2 | source `gras-majuscules`, exactement 10 titres de niveau 1 commençant par « CHAPITRE », au plus 20 titres de niveau 1 au total |
| 3 | aucun paragraphe ne correspond à `^\d+\s+L['’]alchimie spirituelle` ni à `L['’]alchimie spirituelle\s+\d+$` ; une note de marqueur `*` ancrée page 36 ; entre 5 et 60 titres de niveau 1 |
| 4 | source `signets`, 23 titres |
| 5 | source `signets`, 4 titres de niveau 1 et 7 de niveau 2 |
| 6 | ancres en `g` et en `d` (double page) ; aucun paragraphe `^\d+\s+\d+$` ; au moins 1 note appariée |
| 7 | se convertit ; EPUB valide (le schéma de la p. 6 relève du plan 6) |
| 8 | exactement 19 titres de niveau 1 commençant par « CHAPITRE » ; aucune note ancrée page 145 |
| 9 | au moins 8 titres de niveau 1 commençant par « CHAPITRE » ; aucune note ancrée page 367 ; conversion en moins de 60 s en `--release` |

Lancer : `PLUMEO_CORPUS=/home/stef/Documents/RAG cargo test --release -p plumeo-moteur --test corpus -- --ignored --nocapture`
Attendu : ÉCHEC tant que les fichiers de série n'existent pas.

- [ ] **Étape 2 : calibrer les fichiers de série**

Le regroupement de départ suit l'empreinte mesurée pendant la conception :
- `calibre-lettre` : n° 1, 2, 5, 8 et 9 ;
- `calibre-a5` : n° 4 ;
- `prosveta-scan` : n° 3, avec en-têtes alternés ;
- `omnipage-double` : n° 6, avec `double_page` ;
- `distiller-a4` : n° 7.

Si un livre ne tient pas dans sa série, on en crée une autre et on note la raison dans
`docs/corpus.md` : c'est le cas que les séries dérivées traiteront.

Pour chaque série, on mesure avec
`cargo run --release -p plumeo-cli -- page "<pdf>" <N> --serie tests/corpus/series/<s>.json`
sur une page paire et une page impaire, une ouverture de chapitre (n°3 : p. 29) et une
page de notes. On ajuste ensuite `entete`, `pied`, `notes` et `double_page` jusqu'à ce
que :
- aucun titre courant ni numéro de page ne reste dans le corps ;
- aucune ligne de corps ne parte en retrait.

Les exceptions de page sont permises et doivent être justifiées dans `docs/corpus.md`.

- [ ] **Étape 3 : faire passer le test sur les 9 livres**

Lancer la commande de l'étape 1 : les 9 livres doivent passer.

Si une attente échoue à cause d'une règle du moteur et non d'un réglage :
- corriger la règle dans le module concerné ;
- ajouter un test unitaire qui reproduit le cas, sans PDF du corpus ;
- relancer `cargo test --workspace --features plumeo-moteur/fixtures`.

Toute attente impossible à tenir est remontée au porteur, avec la mesure à l'appui, au
lieu d'être assouplie en silence.

- [ ] **Étape 4 : contrôle visuel**

Lancer `plumeo-cli convertir` sur les livres n°1, 3 et 6, ouvrir les EPUB dans Foliate
(`flatpak run com.github.johnfactotum.Foliate <fichier>`) et vérifier quatre points :
- la table des matières ;
- une note qui s'ouvre puis le retour ↩ ;
- l'absence d'en-tête ;
- l'ordre de lecture de la double page.

Consigner le résultat dans `docs/corpus.md`.

- [ ] **Étape 5 : commiter**

```bash
git add tests/corpus crates/moteur/tests/corpus.rs docs/corpus.md
git status --short | grep -E '\.(pdf|epub)$' && echo "STOP : fichier interdit" || git commit -m "Corpus : réglages de série et non-régression sur 9 PDF"
```
