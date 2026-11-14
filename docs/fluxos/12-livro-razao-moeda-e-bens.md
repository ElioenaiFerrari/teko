# Livro-razão, moeda e bens (planejado)

← [Índice da documentação](../README.md)

> **Planejado.** Nada deste fluxo existe no código ainda, além do consenso BFT ([fluxo 11](11-consenso-bft.md)). Serve para fixar a direção; pode mudar.

## Resumo
Dois livros separados, com os **mesmos validadores**: o de **votos** (implementado) e o de **economia** (moeda e bens). Separar evita que regras de dinheiro e de propriedade contaminem a privacidade do voto.

## Quem participa
Validadores (stake + histórico), emissor (elegibilidade de eleitores), órgão registrador M-de-N (bens), vendedor e comprador (dispositivos).

## Diagramas

![Dois livros com os mesmos validadores: votos (implementado) e economia (planejado), com moeda, bens, emissor e órgão registrador](../diagramas/livro-dominios.png)

*Fontes: [SVG](../diagramas/livro-dominios.svg) · [HTML](../diagramas/livro-dominios.html)*

![Venda de bem contra pagamento: vendedor, comprador e órgão aprovam; o livro faz a troca atômica](../diagramas/venda-contra-pagamento.png)

*Fontes: [SVG](../diagramas/venda-contra-pagamento.svg) · [HTML](../diagramas/venda-contra-pagamento.html)*

## Passo a passo (previsto)
1. **M2** `teko-ledger`: envelope de transação, máquina de estados, raiz de estado.
2. **M3** `teko-coin`: teto fixo, recompensa que cai pela metade (*halving*), stake e penalidades, sem pré-mineração; *faucet* só na devnet, rotulada.
3. **M4** `teko-assets`: classes de bens com órgão M-de-N; transferência exige dono + comprador + órgão; gravames; troca atômica (*Swap*).
4. **M5** dispositivo ganha `GetAddress` e `SignTransaction`; a Suite, telas de carteira e patrimônio.
5. **M6** o livro de votos passa a rodar sobre o BFT.

## O que será verificado e por quem
Toda assinatura no dispositivo; regras de bens pelo órgão; validadores refazem tudo.

## O que pode dar errado
Problema do oráculo (o registro digital não prova o mundo físico), concentração de stake, aposta de que a moeda tenha valor.

## Onde está no código
Ainda não existe. O ponto de partida é `crates/teko-bft`.

## Testes que cobrem
Nenhum ainda.

## Limites honestos
Sem valor real, sem validade jurídica e sem integração com registros oficiais (por exemplo, cartórios).
