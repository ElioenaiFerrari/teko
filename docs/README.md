# Documentação do Teko

← [README principal](../README.md)

Tudo em pt-BR (código e identificadores ficam em inglês). Cada fluxo é um arquivo curto, com diagramas no tema do projeto ([TEMA](diagramas/TEMA.md)).

## Fluxos (como cada coisa funciona)

| # | Fluxo | Pergunta que responde |
|---|---|---|
| 00 | [Visão geral](fluxos/00-visao-geral.md) | Quem fala com quem? |
| 01 | [Identidade e dispositivo](fluxos/01-identidade-e-dispositivo.md) | Como o dispositivo é configurado e o que ele protege? |
| 02 | [Assinatura](fluxos/02-assinatura.md) | Por que o computador não engana a assinatura? |
| 03 | [Credenciais e reemissão](fluxos/03-credenciais-e-reemissao.md) | E se eu perder o dispositivo? |
| 04 | [Registro anônimo](fluxos/04-registro-anonimo.md) | Como provar elegibilidade sem se identificar? |
| 05 | [Votação cifrada](fluxos/05-votacao-cifrada.md) | O que viaja na rede quando voto? |
| 06 | [Ciclo da eleição](fluxos/06-ciclo-da-eleicao.md) | Quais fases e quem as controla? |
| 07 | [Apuração e resultados](fluxos/07-apuracao-e-resultados.md) | Quando e como o total aparece? |
| 08 | [Cerimônia de chave](fluxos/08-cerimonia-de-chave.md) | Quem guarda a chave de decifração? |
| 09 | [Blocos e validação](fluxos/09-blocos-e-validacao.md) | Como um bloco é aceito? |
| 10 | [Rede e sincronização](fluxos/10-rede-e-sincronizacao.md) | Como os nós e os clientes se comportam? |
| 11 | [Consenso BFT](fluxos/11-consenso-bft.md) | Como a rede chega a um acordo? |
| 12 | [Livro-razão, moeda e bens](fluxos/12-livro-razao-moeda-e-bens.md) | O que vem a seguir? (planejado) |
| 13 | [Suite desktop](fluxos/13-suite-desktop.md) | Como o app é montado? |
| 14 | [Carteira e frase de recuperação](fluxos/14-carteira-e-seed.md) | Como recupero meu dinheiro se perder o dispositivo? |

## Referência técnica
[ARCHITECTURE](ARCHITECTURE.md) · [THREAT-MODEL](THREAT-MODEL.md) · [KEY-HIERARCHY](KEY-HIERARCHY.md) · [DEVICE-PROTOCOL](DEVICE-PROTOCOL.md) · [ONBOARDING-FLOW](ONBOARDING-FLOW.md) · [RECOVERY](RECOVERY.md) · [NETWORKS](NETWORKS.md) · [PHASE-2-VOTING](PHASE-2-VOTING.md) · [RELEASE](RELEASE.md) · [TEST-MATRIX](TEST-MATRIX.md) · [GLOSSARY](GLOSSARY.md) · [ADRs](adr/README.md)

## Manter a documentação viva
Toda mudança de comportamento, comando `make`, crate, protocolo ou fluxo atualiza, **no mesmo trabalho**, o [README](../README.md) e o fluxo correspondente (texto + diagrama). Os diagramas são gerados por `make diagramas`; `make docs-check` confere links, imagens, texto alternativo e crates no README.
