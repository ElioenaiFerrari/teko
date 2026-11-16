# ADR-0034 — Rede de votos em BFT institucional e economia sem blocos vazios

> **Complemento (ADR-0040):** a chain de votos continua só **ordenando**; o resultado de cada pleito é **assinado por cada instituição** e publicado no quadro (`EntryKind::OUTCOME`), e a economia o aceita com **mais de 2/3** das assinaturas.

**Status:** Aceito provisoriamente (2026-11-15, dono do produto: "faça tudo o que você recomendou") · **Decisão:** poder igual e conjunto fixo no genesis; o validador só ordena; tolerância de relógio de 60 s · Atualiza o [ADR-0014](0014-quadro-de-avisos-e-blockchain.md) (quadro de avisos em PoA) para a rede de **votos**; não muda o log Merkle, o `Tallier` nem as cédulas cifradas ([ADR-0019](0019-cedulas-cifradas-elgamal-de-limiar.md)).

## Contexto
Hoje há duas redes: a **economia** (moeda, bens, stake) já roda em consenso BFT ponderado por stake ([ADR-0028](0028-economia-sobre-bft-e-fase-de-fundador.md)); a de **votos** é o PoA federado de `teko-chain` (M assinaturas de N instituições, proponente em rodízio). O `docs/NETWORKS.md` registra o que o PoA **não** garante: um proponente malicioso pode atrasar ou censurar no seu turno, M instituições em conluio reescrevem o futuro, não há finalidade provada e a equivocação é só detectada, não impedida. Para uma eleição isso é a lacuna mais importante da rede.

Um estudo paralelo (o rewrite em Elixir, abandonado) validou o desenho abaixo e mostrou também que a economia produz **blocos vazios** por temporizador (`empty_block_interval_ms`), o que enche a cadeia de ruído e complica o tempo dos blocos.

## Alternativas
1. **Manter PoA** e só documentar os limites. Barato; deixa a lacuna.
2. **BFT com peso por stake** também nos votos. Rejeitada: quem ordena cédulas não pode ser quem tem mais moeda; a legitimidade vem de instituições, não de capital; votos e moeda devem ficar desacoplados (princípio 5).
3. **BFT institucional (recomendada):** o mesmo motor `teko-bft`, validadores = instituições de **poder igual** (1 instituição = 1 voto), sem moeda nem stake.
4. *Proof of History*: descartada. Prova ordem e passagem de tempo, mas **não dá acordo bizantino** e concentra o trabalho num líder, o contrário de validadores iguais.

## Decisão proposta
1. **Rede de votos em BFT institucional.** Bloco = lista ordenada de `Entry` do quadro (como hoje), com `log_size`/`log_root`, **sem as assinaturas M-de-N**: no lugar, o **certificado de commit** (mais de 2/3 do poder) do `teko-bft`. Proponente sorteado pelo motor. O conjunto de validadores é a lista de instituições do arquivo de rede (`GovernancePolicy`), poder 1 cada, fixo por genesis (troca de instituições por decisão da governança fica fora deste ADR). Com 3 instituições nenhuma falha é tolerada; com 4, uma.
2. **A cadeia só ordena; a apuração continua no `Tallier`.** O log continua sendo a fonte de verdade e a apuração continua rejeitando e **contando** cédulas inválidas (`RejectCounts`), auditável por qualquer réplica. O validador confere só o que o PoA já conferia (cadeia, altura, pai, proponente, tamanhos, raiz Merkle).
3. **Sem blocos vazios, nas duas redes.** Nenhum bloco nasce por temporizador: a altura só começa quando há algo no mempool (um intervalo curto agrupa uma rajada), e um nó sem transação própria entra na altura que os pares iniciaram, para votar e se atualizar. Na economia sai `empty_block_interval_ms`; a gênese é o arquivo, não um bloco. Consequência: a recompensa do fundador só rende com atividade, e o fim da fase de fundador só é registrado no bloco seguinte ao stake que atingiu a meta.
4. **Horário do bloco pelo proponente, conferido.** O validador só vota num bloco cujo horário difere do **próprio relógio** em no máximo 60 s; o horário é estritamente maior que o do bloco anterior (o replay do cliente exige só isso, sem relógio). Um validador com relógio errado tem a proposta recusada. As janelas de votação passam a ser conferidas nesse horário. (Com blocos só sob demanda, o "tempo BFT" por mediana do commit anterior ficaria velho depois de uma pausa; por isso o proponente.)
5. **O cliente não confia em nó nenhum.** `NodeClient::sync_chain` refaz a cadeia de votos verificando certificado contra o conjunto de instituições, encadeamento e raiz Merkle, como já faz com a economia.
6. **Migração.** `teko-chain` ganha o bloco com certificado e uma réplica verificada equivalente; o PoA M-de-N sai da rede de votos (o código antigo é removido, sem modo duplo). `make devnet` continua subindo 3 validadores (a economia e os votos usam a mesma malha); `make devnet-dkg` e a eleição seguem iguais para quem usa.

## Consequências
- Finalidade: um bloco decidido é final (sem reorganização), e dois blocos conflitantes na mesma altura exigiriam mais de 1/3 das instituições assinando ambos (detectável e punível por evidência).
- Censura por um proponente deixa de bastar: na rodada seguinte outro propõe.
- A rede **pausa** (não bifurca) sem quórum.
- Custo: mais mensagens por bloco que o PoA; a verificação de certificado é barata (Ed25519/ECDSA por instituição).
- `docs/NETWORKS.md`, fluxo de consenso, THREAT-MODEL e TEST-MATRIX mudam junto.

## Pontos em aberto para você decidir
1. Poder igual e conjunto fixo no genesis, como acima? (Recomendado: sim; rotação de instituições é outro ADR.)
2. O validador deve **também** conferir as provas das cédulas antes de ordená-las (recusa spam inválido, custa CPU de cada instituição), ou só ordenar e deixar o `Tallier` contar rejeitadas (recomendado agora: só ordenar, com limite de tamanho por entrada)?
3. 60 s de tolerância do relógio está bom para a devnet/testnet?
4. Fora deste ADR e continuam PENDENTES: prova ZK e nullifier (0013, 0006), coerção (0015).

## Plano de implementação (após aprovação)
1. Testes primeiro (proptest e e2e): conferem finalidade, pausa sem quórum, censura contornada, horário fora do relógio recusado, replay exige certificado, nenhum bloco sem transação.
2. `teko-economy`/`teko-node`: remover o temporizador de bloco vazio e a conferência de horário (proponente).
3. `teko-chain`: bloco com certificado, `VoteApp` (`teko_bft::App`), réplica verificada; `teko-node`: rede de votos sobre o motor BFT; `NodeClient`, `election-admin-cli` e Suite sem mudança de uso.
4. Docs e diagramas (`make diagrams`), `make docs-check`.

## Decisão
Aceita provisoriamente com as três recomendações (1: poder igual e conjunto fixo; 2: só ordenar; 3: 60 s). Escopo inclui os nós e a rede P2P (`tools/teko-node`).

## Implementado
- `teko-chain`: `Block` (sem assinaturas), `DecidedBlock` (bloco + `Commit`), `Chain` (réplica verificada: certificado, proponente, horário, conteúdo, raiz Merkle), `VoteApp` (`teko_bft::App` com mempool e conferência de relógio de 60 s). `teko-economy`: horário estritamente crescente e conferido contra o relógio (60 s).
- `teko-node`: `lane.rs` é o hospedeiro BFT **genérico** (timers, gossip, persistência, WAL, sincronização) das duas cadeias; `economy.rs` e `votes.rs` só trazem o que é de cada uma. Nenhum bloco por temporizador; `block_interval_ms` do perfil virou o intervalo de agrupamento de uma rajada. Arquivos: `votes.blocks`/`votes.wal` e `economy.blocks`/`economy.wal`.
- A penalidade das instituições (caução e `Slash` na economia) está no [ADR-0035](0035-caucao-das-instituicoes-e-penalidade-na-rede-de-votos.md).
- Removidos: o PoA M-de-N de `teko-chain` (assinatura de bloco, rodízio `(height+round) % N`, troca de turno por tempo) e `empty_block_interval_ms` da economia.
- Explorador e Suite mostram precommits e quórum no lugar de assinaturas M-de-N.
- Redes de teste antigas (`blocks.log`) precisam ser recriadas (`make devnet-reset`).
- Testes: ver `docs/TEST-MATRIX.md` (etapa 6 e "Economia sem blocos vazios").
- **Primeiras moedas na gênese:** sem blocos vazios o fundador não tem como ganhar a primeira recompensa, então `teko-node init --founder … --alloc TEKO` (padrão 1.000 TEKO de teste) põe as primeiras moedas **na gênese**, na carteira dele (decisão do dono do produto: "alloc como argumento ao subir a gênese"). A fundadora é a carteira do dono, criada no dispositivo e na Suite **antes** de subir a rede (README).

