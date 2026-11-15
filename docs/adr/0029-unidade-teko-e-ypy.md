# ADR-0029 — Unidade da moeda: TEKO e ypy (6 casas decimais)

**Status:** Aceito (2026-11-14, dono do produto). **Implementado:** `teko-crypto::amount`, telas do dispositivo, CLIs e Suite.

## Contexto
Os valores eram inteiros sem unidade e a Suite só mostrava pontos de milhar. O dono do produto quer frações de verdade, como o Bitcoin tem satoshis.

## Decisão
- A moeda se chama **teko** (símbolo **TEKO**). A menor unidade é o **ypy** (tupi: começo, origem, princípio). **1 TEKO = 1.000.000 ypy** (6 casas decimais).
- **O livro guarda inteiros `u64` em ypy.** Nada de ponto flutuante, nem em Rust nem em JS. A assinatura cobre o inteiro em ypy; o protocolo não muda.
- **Formatação e leitura exatas**, num só lugar: `teko_crypto::amount` (`format_amount`, `parse_amount`; `no_std`). Padrão pt-BR: `1.234,5 TEKO`, sem zeros à direita e **sem arredondar**. Entrada com mais de 6 casas é recusada, nunca truncada. A Suite espelha com aritmética inteira (`money()`/`parseTeko()` em `app.js`), exata abaixo de 2^53.
- **A tela do dispositivo mostra o valor exato em TEKO**, derivado do mesmo `u64` que é assinado (`Valor: 0,5 TEKO`, `Taxa: 0,000162 TEKO`). Linhas com mais de 28 colunas quebram em rótulo e valor.
- **Parâmetros no estilo Bitcoin** na rede de teste: teto 21.000.000 TEKO, recompensa 50 TEKO por bloco, *halving* a cada 210.000 blocos. A taxa por byte é em ypy (`fee_per_byte`; 0 na devnet local, onde as partes simuladas não têm moedas).
- **N da fase de fundador** (ADR-0028): o padrão documentado para testnet é 1.000.000 TEKO; a **devnet local** usa **1.000 TEKO** (cerca de 20 blocos). `teko-node init --founder-target TEKO` muda o valor.
- `economy.txt` ganha `units=ypy6`; um arquivo sem essa linha (escala antiga) é recusado com instrução para recriar a rede.

## Consequências
- O teto (2,1·10¹³ ypy) cabe em `u64` e em inteiros exatos do JS (< 9·10¹⁵).
- Uma devnet criada antes desta mudança precisa de `make devnet-reset`.
- `make fund AMOUNT=…` e `issuer-cli economy-fund --amount` passam a ser em **TEKO** e aceitam decimais.

## Testes
Propriedade `parse(format(x)) == x` (proptest, inclusive no teto); telas do dispositivo (`tests.rs`, `e2e.rs`); `parseTeko`/`money` conferidos contra os mesmos casos do Rust.
