#!/usr/bin/env bash
# Contrôles à passer avant chaque commit : mise en forme, clippy sans avertissement, tests.
# Avec PLUMEO_CORPUS (et EPUBCHECK_JAR, facultatif), lance aussi le corpus réel : voir docs/corpus.md.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== Mise en forme (rustfmt.toml)"
cargo fmt --all -- --check

echo "== Clippy, avertissements refusés"
cargo clippy --workspace --all-targets --features plumeo-moteur/fixtures -- -D warnings

echo "== Tests"
cargo test --workspace --features plumeo-moteur/fixtures

if [ -n "${PLUMEO_CORPUS:-}" ]; then
  echo "== Corpus réel"
  cargo test --release -p plumeo-moteur --test corpus -- --ignored --nocapture
fi

echo "Tout est propre."
