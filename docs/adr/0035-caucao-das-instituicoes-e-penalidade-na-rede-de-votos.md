# ADR-0035 — Caução das instituições e penalidade na rede de votos

**Status:** Proposto (2026-11-15) · **Decisão do dono do produto:** alternativa 2 (caução) aceita; penalidade de **10%**. **Em aberto (PENDENTE):** recompensa por disponibilidade das instituições. Complementa o [ADR-0034](0034-votos-em-bft-institucional-e-sem-blocos-vazios.md).

## Contexto
No ADR-0034 a rede de votos ficou **sem moeda**: as instituições votam com poder igual, sem recompensa nem penalidade, e a evidência de equivocação (dois votos assinados em conflito pela mesma instituição) só é **contada e mostrada**. Isso desacopla voto de moeda (princípio 5), mas deixa uma lacuna: uma instituição que equivoca não perde nada dentro do sistema, e nada incentiva manter o nó no ar.

## Alternativas
1. **Sem recompensa nem penalidade** (estado do ADR-0034): responsabilização só por governança.
2. **Caução (escolhida):** cada instituição trava moeda na **economia** só para ser **penalizável**, sem ganhar poder de voto por isso.
3. **Recompensa por bloco** como na economia: rejeitada. Paga quem produz mais blocos (incentivo a incluir lixo), acopla a infraestrutura eleitoral ao preço da moeda e cobraria taxa do eleitor.

## Decisão
1. **Caução na economia.** Cada instituição da rede de votos tem uma conta na economia e trava stake nela (`Stake`), ligando a sua **chave de consenso de votos** à conta (como o `SetValidatorKey` já faz para a economia). A caução **não dá poder de voto** na rede de votos (continua 1 instituição = 1 voto) e não entra no conjunto de validadores da economia por si só.
2. **Evidência vira penalidade.** Quando um nó da rede de votos detecta duas mensagens assinadas em conflito (mesma altura, rodada e tipo) de uma instituição, qualquer um pode submeter a evidência à **economia** (`Slash` com a chave de votos), exatamente como já ocorre com a evidência da economia (ADR-0030). A evidência é verificável por qualquer um com a chave pública, então não há como forjá-la.
3. **Penalidade: 10%** (`slash_bps = 1000`) da caução, **queimada** (as moedas somem de circulação, não vão para nenhuma conta; a oferta cai e o teto de criação não muda). Mesma regra e janela de evidência (`unbonding_period`) da economia, e a mesma ofensa só é penalizada uma vez.
4. **Sem pagamento por bloco.** Nenhuma recompensa em TEKO por blocos de votos. Eleitor não paga taxa para votar.
5. **Recompensa por disponibilidade: PENDENTE.** Proposta mínima: **nenhuma** nesta fase; só caução e penalidade. Se vier a existir, será um valor **fixo** por período, vindo de um orçamento (não por bloco, para não incentivar spam), e precisa de uma decisão sua sobre quem paga e de onde sai.

## Consequências
- Equivocar na rede de votos passa a ter custo real e automático, sem acoplar o poder de voto à moeda.
- Para uma instituição ser penalizável é preciso que ela tenha caução; sem caução a evidência ainda é contada, mas não há o que queimar (a governança decide).
- A rede de votos continua funcionando sem a economia no ar; a penalidade é aplicada quando a evidência chega à economia.
- Limites honestos: a economia é uma cadeia separada (a evidência precisa ser levada até ela por alguém); o valor mínimo da caução e a janela de evidência são parâmetros a calibrar; sem auditoria.

## Plano de implementação (após aprovação)
1. Testes primeiro: equivocação detectada na rede de votos vira `Slash` na economia e queima 10% da caução; a mesma ofensa não é penalizada duas vezes; evidência forjada é recusada; a caução não dá voto.
2. `teko-coin`: vínculo da chave de consenso de votos à conta e reaproveitamento do `Slash` (hoje só reconhece a chave de consenso da economia).
3. `teko-node`: a lane de votos passa a gerar a transação de `Slash` para a economia (hoje `on_evidence` é vazio) e a gossipá-la.
4. Docs (NETWORKS, THREAT-MODEL, TEST-MATRIX, fluxo de consenso) e, se couber, diagrama.

## Decisão
Aceita provisoriamente nos itens 1 a 4 (caução, evidência como `Slash`, 10% queimado, sem pagamento por bloco). Item 5 (recompensa de disponibilidade): **PENDENTE**.
