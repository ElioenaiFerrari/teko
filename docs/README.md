# Documentação do Teko

← [README principal](../README.md)

Tudo em pt-BR (código e identificadores ficam em inglês). Cada fluxo é um arquivo curto, com diagramas no tema do projeto ([TEMA](diagrams/THEME.md)).

## Guia de conceitos (para aprender)

**[Guia de conceitos](concepts/README.md)**: hash, assinatura, HRP/bech32m, carteira, M-de-N, consenso (PoA e BFT), livro-razão, halving, votação secreta (ElGamal, DKG) e segurança do dispositivo, cada um com diagrama e código Rust simples.

## Fluxos (como cada coisa funciona)

| # | Fluxo | Pergunta que responde |
|---|---|---|
| 00 | [Visão geral](flows/00-overview.md) | Quem fala com quem? |
| 01 | [Identidade e dispositivo](flows/01-identity-and-device.md) | Como o dispositivo é configurado e o que ele protege? |
| 02 | [Assinatura](flows/02-signing.md) | Por que o computador não engana a assinatura? |
| 03 | [Credenciais e reemissão](flows/03-credentials-and-reissue.md) | E se eu perder o dispositivo? |
| 04 | [Registro anônimo](flows/04-anonymous-registration.md) | Como provar elegibilidade sem se identificar? |
| 05 | [Votação cifrada](flows/05-encrypted-voting.md) | O que viaja na rede quando voto? |
| 06 | [Ciclo da eleição](flows/06-election-lifecycle.md) | Quais fases e quem as controla? |
| 07 | [Apuração e resultados](flows/07-tally-and-results.md) | Quando e como o total aparece? |
| 08 | [Cerimônia de chave](flows/08-key-ceremony.md) | Quem guarda a chave de decifração? |
| 09 | [Blocos e validação](flows/09-blocks-and-validation.md) | Como um bloco é aceito? |
| 10 | [Rede e sincronização](flows/10-network-and-sync.md) | Como os nós e os clientes se comportam? |
| 11 | [Consenso BFT](flows/11-bft-consensus.md) | Como a rede chega a um acordo? |
| 12 | [Livro-razão, moeda e bens](flows/12-ledger-coin-and-assets.md) | O que vem a seguir? (planejado) |
| 13 | [Suite desktop](flows/13-desktop-suite.md) | Como o app é montado? |
| 14 | [Carteira e frase de recuperação](flows/14-wallet-and-seed.md) | Como recupero meu dinheiro se perder o dispositivo? |
| 15 | [Explorador da chain](flows/15-explorer.md) | Como vejo blocos, transações e saldos sem confiar no nó? |

## Referência técnica
[ARCHITECTURE](ARCHITECTURE.md) · [THREAT-MODEL](THREAT-MODEL.md) · [KEY-HIERARCHY](KEY-HIERARCHY.md) · [DEVICE-PROTOCOL](DEVICE-PROTOCOL.md) · [ONBOARDING-FLOW](ONBOARDING-FLOW.md) · [RECOVERY](RECOVERY.md) · [NETWORKS](NETWORKS.md) · [PHASE-2-VOTING](PHASE-2-VOTING.md) · [RELEASE](RELEASE.md) · [TEST-MATRIX](TEST-MATRIX.md) · [GLOSSARY](GLOSSARY.md) · [ADRs](adr/README.md)

## Manter a documentação viva
Toda mudança de comportamento, comando `make`, crate, protocolo ou fluxo atualiza, **no mesmo trabalho**, o [README](../README.md) e o fluxo correspondente (texto + diagrama). Os diagramas são gerados por `make diagrams`; `make docs-check` confere links, imagens, texto alternativo e crates no README.
