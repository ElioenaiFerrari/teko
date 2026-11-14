#!/usr/bin/env bash
# Garante que o build NORMAL do emulador não contém os botões automáticos (test-buttons).
# Prova o detector construindo também a variante com a feature (que DEVE conter as marcas).
set -euo pipefail
cd "$(dirname "$0")/.."
MARKS=("MODO TESTE" "[auto] CONFIRMADO" "[auto] PIN digitado")
BIN=target/release/device-emulator

has_marks() { local n=0; for m in "${MARKS[@]}"; do grep -aqF -- "$m" "$BIN" && n=$((n+1)); done; echo "$n"; }

echo "→ build release NORMAL…"
cargo build -q --release --locked -p device-emulator
normal=$(has_marks)
echo "→ build release COM test-buttons (controle do detector)…"
cargo build -q --release --locked -p device-emulator --features test-buttons
with=$(has_marks)
# restaura o binário normal para quem for usá-lo
cargo build -q --release --locked -p device-emulator

echo "marcas no build normal: $normal (esperado 0) | com feature: $with (esperado ${#MARKS[@]})"
if [ "$normal" -ne 0 ] || [ "$with" -ne "${#MARKS[@]}" ]; then
  echo "FALHOU: botões automáticos vazaram para o build normal (ou o detector está quebrado)"; exit 1
fi
echo "OK: o build normal não contém botões automáticos"
