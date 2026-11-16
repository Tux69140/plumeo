#!/usr/bin/env bash
# Télécharge pdfium (binaires précompilés) dans vendor/pdfium/.
set -euo pipefail
SHA256_PDFIUM=1c251efce460d67437a4c76d4be9033ae2f17bf5603f5bff1af10209b94ca723
racine="$(cd "$(dirname "$0")/.." && pwd)"
cible="$racine/vendor/pdfium"
if [ -f "$cible/lib/libpdfium.so" ]; then
  echo "pdfium déjà présent : $cible/lib/libpdfium.so"
  exit 0
fi
mkdir -p "$cible"
curl -fL "https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F8125/pdfium-linux-x64.tgz" -o "$cible/pdfium.tgz"
tar -xzf "$cible/pdfium.tgz" -C "$cible"
rm "$cible/pdfium.tgz"
# Somme de contrôle de la bibliothèque (chromium/8125, linux-x64) avec laquelle
# Pluméo est testé.
if ! (cd "$cible/lib" && echo "$SHA256_PDFIUM  libpdfium.so" | sha256sum -c --quiet); then
  echo "somme de contrôle de libpdfium.so incorrecte : bibliothèque retirée" >&2
  rm -f "$cible/lib/libpdfium.so"
  exit 1
fi
