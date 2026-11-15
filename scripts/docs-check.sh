#!/usr/bin/env bash
# Confere a documentação viva (regra em CLAUDE.md): links, imagens, crates no README, texto alternativo e idioma.
set -u
cd "$(dirname "$0")/.."
erros=0
falha() { echo "docs-check: $1"; erros=$((erros + 1)); }

mds=$(find README.md docs -name '*.md' -not -path 'docs/adr/*' | sort)
mds="$mds
$(find docs/adr -name '*.md' | sort)"

# 1) links e imagens relativos precisam existir
for f in $mds; do
  dir=$(dirname "$f")
  grep -o '\](\([^)]*\))' "$f" | sed 's/^](//; s/)$//' | while read -r alvo; do
    case "$alvo" in http*|mailto:*|\#*|"") continue ;; esac
    caminho="${alvo%%#*}"
    [ -z "$caminho" ] && continue
    [ -e "$dir/$caminho" ] || echo "ALVO $f -> $alvo"
  done
done > /tmp/docs-check.$$ 2>&1
while read -r l; do falha "link quebrado: ${l#ALVO }"; done < <(grep '^ALVO' /tmp/docs-check.$$)
rm -f /tmp/docs-check.$$

# 2) imagens sem texto alternativo
for f in $mds; do
  grep -n '!\[\]' "$f" | while read -r l; do echo "ALT $f:$l"; done
done > /tmp/docs-check-alt.$$
while read -r l; do falha "imagem sem texto alternativo: ${l#ALT }"; done < <(cat /tmp/docs-check-alt.$$)
rm -f /tmp/docs-check-alt.$$

# 3) só PNG: SVG/HTML de diagramas não são guardados (o gerador os usa só como intermediário)
for p in docs/diagrams/*.svg docs/diagrams/*.html; do
  [ -e "$p" ] && falha "diagrama gerado não deve ser versionado: $p (guarde só o .png)"
done

# 4) todo crate do workspace aparece no README
for t in crates/*/Cargo.toml tools/*/Cargo.toml apps/suite/src-tauri/Cargo.toml; do
  nome=$(grep -m1 '^name' "$t" | cut -d'"' -f2)
  grep -q "$nome" README.md || falha "crate '$nome' não aparece no README.md"
done

# 5) fluxos: molde obrigatório e títulos em português
for f in docs/flows/[0-9]*.md; do
  for sec in "## Resumo" "## Passo a passo" "## Onde está no código" "## Limites honestos"; do
    grep -q "^$sec" "$f" || falha "$f sem a seção '$sec'"
  done
  grep -n -E '^#{1,3} (Overview|Summary|Participants|Steps|Limits|Limitations|Tests|Testing|Where)' "$f" \
    | while read -r l; do echo "EN $f:$l"; done
done > /tmp/docs-check-en.$$
while read -r l; do falha "título em inglês: ${l#EN }"; done < <(cat /tmp/docs-check-en.$$)
rm -f /tmp/docs-check-en.$$

if [ "$erros" -gt 0 ]; then echo "docs-check: $erros problema(s)"; exit 1; fi
echo "docs-check: ok"
