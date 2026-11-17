# Pluméo

Convertisseur de livres PDF en EPUB 3 recomposable, pensé pour des séries de livres
mis en page de la même façon. Le PDF passe par un Markdown interne ancré (chaque
bloc garde sa page et son rectangle d'origine), puis devient un EPUB : un fichier par
chapitre, table des matières, notes surgissantes avec lien retour.

État : jalon 1 (moteur et EPUB en ligne de commande). Feuille de route :
`docs/plan-de-developpement.md`.

## Organisation

- `crates/moteur` (`plumeo-moteur`) : lecture du PDF (pdfium), zones, paragraphes,
  titres, notes, écriture du Markdown interne.
- `crates/epub` (`plumeo-epub`) : du Markdown interne à l'EPUB 3.
- `crates/commun` (`plumeo-commun`) : conventions partagées par le moteur et l'EPUB
  (normalisation du texte, identifiants des titres, titre d'un sommaire imprimé).
- `crates/cli` (`plumeo-cli`) : la ligne de commande.

## Prérequis

- Rust 1.98 ou plus récent (édition 2024).
- pdfium, téléchargé dans `vendor/pdfium/` (binaire précompilé, somme de contrôle
  vérifiée) :

  ```bash
  scripts/recuperer-pdfium.sh
  ```

- Facultatif, pour valider les EPUB : Java et
  [epubcheck 5.4.0](https://github.com/w3c/epubcheck/releases), décompressé dans
  `vendor/epubcheck/`.

## Utilisation

```bash
cargo build --release
target/release/plumeo-cli convertir livre.pdf \
  --serie tests/corpus/series/calibre-a5.json --sortie sortie/
target/release/plumeo-cli page livre.pdf 12 --serie tests/corpus/series/calibre-a5.json
```

`convertir` écrit dans le dossier de sortie l'EPUB, le Markdown interne, les retraits
(`.retraits.json`) et les points à vérifier (`.points.json`). Options : `--titres`
(`auto`, `signets`, `gras-majuscules`, `taille`), `--titre`, `--auteur`. `page`
affiche le texte du corps d'une page et ce qui en a été retiré.

## Tests

```bash
cargo test --workspace --features plumeo-moteur/fixtures
```

Avant chaque commit, `scripts/verifier.sh` contrôle la mise en forme (`cargo fmt`,
réglée par `rustfmt.toml` : 120 colonnes), l'absence d'avertissement de clippy et les
tests ; avec `PLUMEO_CORPUS`, il lance aussi le corpus réel.

Les PDF de test sont fabriqués à la volée : aucun PDF n'est versionné. Le test de
non-régression sur un corpus de livres réels (non fourni) est décrit dans
`docs/corpus.md`.
