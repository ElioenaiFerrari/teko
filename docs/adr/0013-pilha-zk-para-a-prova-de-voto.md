# ADR-0013 — Sistema de prova para o voto (zkVM, Circom, Halo2/Noir)

**Status:** Proposto · **Decisão: PENDENTE (dono do produto)** · Fase 2 (ver `docs/PHASE-2-VOTING.md` §4.3)

## Contexto
A prova de voto (gerada no **host**, sem ver a chave de eleitor) deve afirmar: assinatura válida do dispositivo sobre a cédula, chave na árvore de eleitores, nullifier correto, cédula válida. A curva do projeto é P-256 (ADR-0002).

## Alternativas
| | zkVM em Rust (RISC Zero / SP1) | Circom + Groth16 | Halo2 / Noir |
|---|---|---|---|
| Escrita da prova | Código Rust comum (reusa `p256`) | Circuito DSL | Circuito DSL / Noir |
| ECDSA P-256 | Natural; relatos de aceleração (**verificar**) | Caro em circuito | Possível, caro |
| Tamanho/verificação da prova | Maior; verificação on-chain mais cara | **Pequena e barata em EVM** | Média |
| Trusted setup | Não (STARK) ou conforme backend | **Sim, por circuito** | Não / universal |
| Tempo de prova no host | Alto | Médio | Médio |
| Encaixe com o projeto | **Alto** | Médio | Médio |

## Recomendação
Prototipar em **zkVM** (Rust, sem cerimônia de setup, reaproveita o `p256`), medir tempo e tamanho, e só então decidir. Esconder atrás de um `trait VoteProver`/`VoteVerifier` com um **provador simulado** primeiro, para fechar fluxo e UI.

## Consequências
Prova pesada (segundos a minutos no desktop). Verificação on-chain pode exigir um passo de "wrapping" para uma prova pequena.
