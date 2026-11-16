# Rede e sincronização

← [Índice da documentação](../README.md)

## Resumo
Os validadores (`teko-node`) trocam entradas, transações e votos do consenso por *gossip* TCP e decidem os blocos por **BFT** (ADR-0034): o proponente é sorteado por rodada e, se falhar, a rodada seguinte tem outro. **Nenhum bloco nasce sozinho.** Clientes (Suite, `election-admin`, verificador) **não confiam em nenhum nó**: baixam os blocos e refazem a verificação.

## Quem participa
Suite ou cliente, proponente da rodada, demais instituições.

## Diagramas

![Sequência da decisão de um bloco de votos: a Suite envia a cédula, o proponente propõe, as instituições dão prevote e precommit, o bloco com certificado de mais de 2/3 é distribuído; se o proponente falhar, a rodada avança](../diagrams/block-production.png)

![Sincronização sem confiar: o cliente aceita a cadeia do nó honesto e rejeita a do nó adulterado](../diagrams/sync.png)

## Passo a passo
1. A entrada (ou transação) chega ao *mempool* de um nó, que a espalha aos pares. Só então a altura começa (após um curto intervalo que agrupa uma rajada); com tudo parado a cadeia fica parada.
2. O proponente sorteado monta o bloco; as instituições dão prevote e precommit; com mais de 2/3 de precommits o bloco é final e o **certificado de commit** vai junto.
3. Se o proponente não responde ou censura, estoura o *timeout* e a rodada seguinte tem outro proponente. Sem quórum a rede pausa, nunca bifurca.
4. O cliente usa `NodeClient::sync_chain` / `sync_into` e aplica cada bloco por `Chain::apply`.

## Como operar (make)
`make devnet` → `make devnet-dkg` → `make devnet-status` / `make devnet-blocks` → `make devnet-stop`. Limpeza: `make devnet-reset`. Detalhes: [NETWORKS](../NETWORKS.md).

## O que é verificado e por quem
O cliente verifica toda a cadeia (certificado, proponente, encadeamento, raiz Merkle); nó com certificado falso é descartado.

## O que pode dar errado
Nós órfãos de uma execução antiga ("Address already in use"): `make devnet-stop` encerra os órfãos. Com 3 instituições, um nó fora do ar **pausa** a rede (não há quórum): use 4 para tolerar 1. Redes criadas antes do ADR-0034 (`blocks.log`) precisam de `make devnet-reset`.

## Onde está no código
`tools/teko-node` (`node.rs`, `client.rs`, `wire.rs`), `crates/teko-chain`.

## Testes que cobrem
`tools/teko-node/tests/network.rs` (portas alocadas globalmente para não colidir).

## Limites honestos
PoA federada, não finalidade bizantina. Sem rede entre máquinas ainda.
