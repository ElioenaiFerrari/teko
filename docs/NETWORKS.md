# Redes de teste (blockchain federada PoA)

Status: **implementado como prova de conceito local** (Fase 2, etapa 6). Decisão registrada em [ADR-0014](adr/0014-quadro-de-avisos-e-blockchain.md) como **aceita provisoriamente**.

## Perfis

| Rede | Para quê | Estado |
|---|---|---|
| `devnet` | Sua máquina: 3 validadores em loopback, bloco a cada ~2 s | ✅ `make devnet` |
| `testnet` | Várias máquinas, dados fictícios, regras iguais às de produção (bloco a cada ~6 s) | ✅ perfil e `init`; rodar em várias máquinas exige editar `peers.txt` e habilitar escuta fora de loopback (não é o padrão) |
| `mainnet` | A eleição de verdade | 🔒 **TRAVADA**: `NetworkProfile::new(Mainnet)` e `network=mainnet` no arquivo são recusados |

Cada rede tem um `chain_id` (`teko-devnet-1`, `teko-testnet-1`). Ele entra no **cabeçalho de cada bloco** e no **manifesto assinado da eleição** (`ElectionManifest.chain_id`). O dispositivo (`trust.txt: chain_id=`) recusa manifesto de outra rede, e a apuração (`Tallier::with_chain_id`) ignora e conta manifestos de outra rede.

## Como funciona

- **Validadores = instituições independentes.** Cada bloco precisa de **M assinaturas distintas de N** (devnet: 2 de 3), incluindo a do proponente. Reaproveita `GovernancePolicy` e `count_distinct_valid`.
- **Bloco** (`teko-chain`): `chain_id, height, prev_hash, timestamp, proposer, round, entries[], log_size, log_root, signatures`. As assinaturas cobrem tudo menos elas mesmas. `log_size/log_root` fixam o estado exato do log Merkle depois do bloco; uma réplica refaz o **mesmo** log a partir dos blocos.
- **Proponente em rodízio:** validador `(height + round) % N`. Se ele está fora do ar, a cada `3 × intervalo` esperando o turno passa ao próximo (`round`++), então a rede continua com 2 de 3.
- **Validador só assina o que verificou** (`Chain::sign` recusa bloco inválido) e **não assina dois blocos diferentes na mesma altura** (exceto de rodada maior, para não travar a rede).
- **Equívoco** (dois blocos conflitantes na mesma altura, ambos com quórum) é **detectado**, contado e mostrado (`status`), não impedido.
- **O cliente não confia em nenhum nó:** `NodeClient::sync_chain` baixa os blocos e re-verifica assinaturas, encadeamento e raiz Merkle. A Suite e o `election-admin` leem assim.
- **Persistência:** `dev/net/node-<i>/blocks.log`; no boot cada bloco é reverificado.

## Rodando

```
make devnet            # sobe 3 validadores (cria dev/net na 1ª vez)
make election          # liga a eleição à rede (chain_id) e fixa o dispositivo nela
                       # → reinicie o emulador (ele lê o trust.txt na partida)
make publish           # a eleição entra por blocos
make simulate-live     # votos sintéticos via rede
make devnet-status     # altura, mempool, pares de cada nó
make devnet-blocks     # blocos verificados no terminal
make devnet-stop | devnet-reset
```
Sem rede rodando, tudo continua funcionando com o arquivo local (`dev/board.log`). Para forçar o arquivo: `TEKO_BOARD=file`. Em modo rede o quadro é **append-only**: `reset-board` não faz nada (use `make devnet-reset`).

## O que PoA federada garante, e o que NÃO

Garante: nenhum bloco entra sem M instituições; réplicas independentes reconstroem o mesmo log e conferem tudo; um operador isolado não consegue mostrar logs diferentes sem que dois blocos conflitantes com quórum apareçam (V19, em parte); um nó fora do ar não para a rede.

**Não garante:** consenso bizantino completo (um proponente malicioso pode **atrasar ou censurar** no seu turno, e M validadores em conluio podem reescrever o futuro); finalidade provada; privacidade do voto (cédulas continuam públicas, decisão #14); proteção de rede (TCP simples em loopback, sem criptografia entre nós: a integridade vem das assinaturas dos blocos). Não há mineração nem tokens. **Sem auditoria.**
