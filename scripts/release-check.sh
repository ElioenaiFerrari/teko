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

# A Suite de PRODUÇÃO não pode carregar a autoridade/operador de teste (feature dev-simulation).
# Detector com controle: o build de desenvolvimento DEVE listar issuer-cli, o de produção NÃO.
echo "→ Suite: dependências sem e com dev-simulation…"
prod=$(cargo tree --locked -p teko-suite --no-default-features --features custom-protocol -e normal | grep -c "issuer-cli" || true)
dev=$(cargo tree --locked -p teko-suite -e normal | grep -c "issuer-cli" || true)
echo "issuer-cli na Suite de produção: $prod (esperado 0) | de desenvolvimento: $dev (esperado >=1)"
if [ "$prod" -ne 0 ] || [ "$dev" -lt 1 ]; then
  echo "FALHOU: a autoridade de teste vazou para a Suite de produção (ou o detector está quebrado)"; exit 1
fi
echo "OK: a Suite de produção não contém a autoridade/operador de teste"
