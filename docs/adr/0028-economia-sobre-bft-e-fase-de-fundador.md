# ADR-0028 — Economia sobre consenso BFT, validadores por stake e fase de fundador

**Status:** Aceito provisoriamente (2026-11-14, dono do produto). **Implementado** (M6a): `teko-economy`, serviço de economia no `teko-node`, Suite lendo a cadeia verificada. Resolve a decisão pendente do [ADR-0021](0021-moeda-teto-fixo-stake-e-taxas.md) pela **opção B**.

## Decisão
- **O livro de economia** (moeda, bens e stake) passa a rodar em **consenso BFT** (`teko-bft`) dentro do `teko-node`, com blocos de transações e **certificado de commit** (> 2/3 do poder de voto). O **livro de votos continua PoA** nesta etapa; migrá-lo é o M6b.
- **Fase de fundador (opção B).** O conjunto de validadores começa com os **fundadores da gênese**. O primeiro fundador é o **dono do produto**: as **recompensas** dos blocos que o validador dele produz vão para a **carteira dele** (o dono). Sem pré-mineração: não há alocação de gênese para o fundador.
- **Fim da fase.** Quando o **stake total** chega a **N = 1.000.000 TEKO** (na devnet local, 1.000 TEKO; ver [ADR-0029](0029-unidade-teko-e-ypy.md)), uma trava no estado (`founder_phase_over`, irreversível) encerra a fase; a partir do bloco seguinte o conjunto de validadores é derivado do stake. O fundador precisa **travar** (stake) as recompensas para a fase terminar.
- **Chave de consenso ≠ chave da carteira.** A carteira fica no dispositivo (cada voto BFT exigiria o botão); o validador usa a **chave de consenso** do nó. A operação `SetValidatorKey` (assinada pela carteira, no dispositivo) vincula dono e chave; o estado guarda o vínculo. A **recompensa vai ao dono**, e a penalidade (*slash*) por equivocação resolve a chave de consenso para o dono. Na fase de fundador o vínculo vem da gênese.
- **Conjunto por altura, derivado do estado:** o conjunto da altura *h* é função do estado após o bloco *h−1* (fundadores durante a fase; depois, validadores por stake, ordenados por chave, no máximo K, com mínimo por validador). Nunca fica vazio: `Unstake` que deixe o stake total abaixo de `min_total_stake` é recusado.
- **Poder de voto quantizado:** `poder = max(1, stake × 10.000 / stake_total)`, porque o proponente do BFT é um rodízio ponderado por blocos contíguos e stakes grandes dariam sequências enormes ao primeiro validador.
- **Blocos vazios** são permitidos e o nó os propõe a cada `empty_block_interval_ms` quando não há transações (sem eles o fundador não ganha recompensa).
- **Reinício de validador:** o nó grava a altura/rodada em que votou e, ao reiniciar, retoma na **rodada seguinte** para não votar duas vezes na mesma rodada (isso evitaria a auto-penalidade). Perder o *lock* num reinício é um limite conhecido do BFT sem *write-ahead log* completo.
- **Verificação por clientes:** o cliente baixa os blocos e **reproduz o estado** (`Ledger`), verificando cada certificado contra o conjunto daquela altura. Não confia em nenhum nó.

## Como ficou
- `teko-coin`: `founder_target`, `min_total_stake`, `min_validator_stake`, `max_validators`; `genesis_founders`; `SetValidatorKey`; `validator_set`; recompensa ao dono; penalidade pela chave de consenso. `teko-ledger`: `check_block` (execução de ensaio). `teko-bft`: proponente por **sorteio por hash proporcional ao poder** (o rodízio contíguo foi trocado), `App::validate_from` (o bloco confere o proponente) e `start_height_at` (retomada em rodada posterior).
- `teko-economy` (sem I/O): `EconomyChain`, `EconApp`, certificado de commit por bloco, `replay` para clientes. `teko-node`: motor BFT por nó, fio (`ECON_*`), persistência `economy.blocks` reverificada no boot, `economy.wal`, pacing de blocos vazios (`TEKO_EMPTY_BLOCK_MS`, padrão 3 s), sincronização por pares e `NodeWorld`.
- **Taxas desligadas na rede de teste local** (`fee_per_byte=0` em `economy.txt`): o órgão e a contraparte simulados não têm moedas para pagá-las (não há pré-mineração). O mecanismo de taxas está testado no módulo de moeda; dá para ligá-lo editando o arquivo.

## Limites honestos
- A **fase de fundador é centralizada** (um validador) até o stake total chegar a N; é a escolha B.
- Só **loopback** (devnet/testnet), **mainnet travada**, sem auditoria, sem valor real.
- O DKG e o livro de votos continuam presos ao PoA com M-de-N fixo (nós 0–2) até o M6b.
- **Equivocação:** o nó conta a evidência e **envia a transação `Slash` sozinho** (ADR-0030).
- O *lock* do BFT agora é gravado no `economy.wal` antes do voto sair e restaurado no reinício (ADR-0030).
- Sem reputação/histórico como peso extra; sem delegação de stake.
