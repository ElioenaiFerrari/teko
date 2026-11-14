#!/usr/bin/env bash
# Compila device-emulator e issuer-cli DUAS vezes, em diretórios de alvo diferentes,
# e compara os hashes. Se forem iguais, o build é reproduzível neste ambiente.
# (A Suite/Tauri ainda não foi verificada: ver docs/RELEASE.md.)
set -euo pipefail
cd "$(dirname "$0")/.."
SRC="$PWD"
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-1700000000}"
export CARGO_INCREMENTAL=0
export RUSTFLAGS="--remap-path-prefix=$SRC=/teko --remap-path-prefix=${CARGO_HOME:-$HOME/.cargo}=/cargo --remap-path-prefix=${RUSTUP_HOME:-$HOME/.rustup}=/rustup"
TMP="${REPRO_TMP:-$(mktemp -d)}"
BINS=(device-emulator issuer-cli)

build() { CARGO_TARGET_DIR="$TMP/$1" cargo build -q --release --locked -p device-emulator -p issuer-cli; }
sha() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }

echo "→ build A…"; build a
echo "→ build B (outro diretório)…"; build b
ok=1
for b in "${BINS[@]}"; do
  ha=$(sha "$TMP/a/release/$b"); hb=$(sha "$TMP/b/release/$b")
  if [ "$ha" = "$hb" ]; then echo "  IGUAL      $b  $ha"; else echo "  DIFERENTE  $b  $ha  $hb"; ok=0; fi
done
rm -rf "$TMP"
[ "$ok" = 1 ] && echo "OK: build reproduzível (mesmo hash em dois diretórios)" || { echo "FALHOU: build não reproduzível"; exit 1; }
