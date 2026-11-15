# ADR-0021 — Moeda de teto fixo, stake, taxas e penalidades (`teko-coin`)

**Status:** Aceito provisoriamente (2026-11-14): o programa foi aprovado pelo dono do produto (teto fixo com *halving*; validadores por stake e penalidade; taxa por byte paga ao proponente, opção A). **Implementado** o módulo (M3). **Decidido (2026-11-14, dono do produto): opção B**, com o próprio dono do produto como **primeiro fundador** (detalhes em [ADR-0028](0028-economia-sobre-bft-e-fase-de-fundador.md)).

## Decisões
- **Teto fixo.** Tudo que já foi criado (`minted`) nunca passa de `cap`. A moeda só nasce por (1) **recompensa de bloco**, que cai pela metade a cada `halving_interval` blocos, (2) **faucet** e (3) **alocações de gênese**; estas duas só na devnet e rotuladas (evento `faucet-TEST-COINS`). O código recusa gênese de outra rede com faucet ou alocações (`OnlyDevnet`). Sem pré-mineração fora da devnet.
- **Invariante de oferta** (`audit`): criado − queimado = saldos + stake + em desvinculação. Testado com proptest.
- **Taxas (opção A).** `fee = fee_per_byte × bytes da transação`, paga **ao proponente** do bloco. O campo `fee` entra na assinatura. O núcleo do livro cobra antes de executar o módulo; sem saldo, a transação é recusada. Mudança para taxa base queimada (estilo EIP-1559) é possível depois, porque o gancho (`Module::min_fee`/`charge_fee`) já existe.
- **Stake.** `Stake` move saldo para o stake e registra a chave pública do validador. `Unstake` coloca o valor em desvinculação por `unbonding_period` blocos; `Claim` devolve o que já amadureceu. Nesse período o valor ainda pode ser penalizado. `validators()` entrega (chave, stake) para montar o conjunto de validadores do BFT.
- **Penalidade.** Uma `Evidence` de equivocação do BFT, verificada, queima `slash_bps` (base 10.000) do stake + desvinculação do validador, **uma vez** por (validador, altura, rodada, tipo), e só dentro da janela de desvinculação. A prova pode ser enviada por qualquer um (ainda sem recompensa ao denunciante: ela seria queimada).
- **Sem reputação/histórico no M3.** O peso extra por histórico entra no M6 (comitê do livro de votos).

## Início da rede: opção B (decidido)
Quem produz os primeiros blocos se ninguém tem stake? **Opção B, sem pré-mineração:** uma **lista de validadores de gênese** (fundadores) produz os blocos; as **recompensas de bloco** viram o primeiro stake; o conjunto passa a ser derivado do stake quando o **stake total atinge N = 1.000.000** (unidades de teste). O **primeiro fundador é o dono do produto**: as recompensas vão para a **carteira dele** (endereço do dispositivo). As opções A (alocação de gênese pública) e C (venda inicial) foram descartadas. A fase de fundador é **centralizada por desenho** e temporária.

## Limites
Sem auditoria. Sem validação de que `cap` seja coerente com `initial_reward × halving_interval × 2` (a emissão total tende a esse valor; o teto só corta o excesso). Sem preço de mercado nem câmbio: **sem valor real**. A raiz do estado é O(n).
