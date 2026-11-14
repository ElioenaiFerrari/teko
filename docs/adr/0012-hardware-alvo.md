# ADR-0012 — Hardware alvo (MCU / secure element)

**Status:** Proposto · **Decisão:** placeholder para fase futura

## Contexto
O núcleo do dispositivo é `no_std`-compatível para migrar a um microcontrolador. Escolha de hardware ainda não é necessária.

## A avaliar (não decidir agora)
- MCU com Rust embarcado (ex.: STM32, nRF, RP2350) com **secure element** externo ou TrustZone-M.
- Suporte nativo à curva escolhida (ADR-0002) e a RNG verdadeiro.
- Proteção de leitura de flash, contador monotônico à prova de rollback, tela e botões físicos, USB HID.
- Open hardware / auditabilidade.

## Consequências
Resolve as limitações assumidas no emulador (THREAT-MODEL T4; ARCHITECTURE, pergunta 4). Enquanto não decidido, todas as garantias de resistência física são aspiracionais.
