#!/usr/bin/env bash
# Assemble a portable folder from a release build.
#
#   ./scripts/portable.sh
#
# Output: release/NSCB Desktop/
set -euo pipefail
cd "$(dirname "$0")/.."

PRODUCT_NAME="NSCB Desktop"
OUT="release/$PRODUCT_NAME"

cargo build --release

case "$(uname -s)" in
  MINGW*|MSYS*|CYGWIN*|Windows_NT) EXE="nscb-desktop.exe" ;;
  *) EXE="nscb-desktop" ;;
esac

if [[ ! -f "target/release/$EXE" ]]; then
  echo "ERROR: target/release/$EXE not found. Did the build succeed?" >&2
  exit 1
fi

rm -rf "$OUT"
mkdir -p "$OUT"
cp "target/release/$EXE" "$OUT/$EXE"
echo "  $EXE"
echo
echo "Portable folder ready: $OUT/"
echo "Zip this folder to distribute."
