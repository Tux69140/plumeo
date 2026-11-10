#!/usr/bin/env bash
# Télécharge pdfium (binaires précompilés) dans vendor/pdfium/.
set -euo pipefail
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
