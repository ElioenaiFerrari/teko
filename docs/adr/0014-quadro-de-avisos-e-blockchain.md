# ADR-0014 — Quadro de avisos (blockchain) da votação

**Status:** Proposto · **Decisão: PENDENTE (dono do produto)** · Fase 2 (ver `docs/PHASE-2-VOTING.md` §5)

## Contexto
Precisamos de um registro **append-only e verificável** para manifestos, raízes da árvore de eleitores, cédulas, nullifiers, provas e resultado.

## Alternativas
| | Cadeia EVM (L2/permissionada) | Cadeia própria (Substrate/Cosmos) | Log transparente Merkle atrás de `trait BulletinBoard` |
|---|---|---|---|
| Parece "blockchain" | Sim | Sim | Parcialmente (sem consenso distribuído) |
| Esforço | Alto | Muito alto | **Baixo** |
| Verificação de prova on-chain | Sim | Sim | Não (verificadores off-chain, independentes) |
| Isola a decisão de cadeia | Não | Não | **Sim** |

## Recomendação
Começar pelo **log Merkle** (`teko-bulletin`, Rust) com provas de inclusão e consistência, atrás de uma `trait` que depois ganha backend EVM. Assim a criptografia e o produto avançam sem travar na escolha de infraestrutura.

## Perguntas
Quem opera o quadro? Como evitar censura/queda na janela de votação? Quantos *witnesses*/espelhos independentes?
