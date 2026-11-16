# Redes de teste (votos em BFT institucional, economia em BFT por stake)

Status: **implementado como prova de conceito local**. A rede de votos roda em **consenso BFT entre as instituições** (ADR-0034, que substitui o PoA M-de-N do ADR-0014 nesta rede); a economia roda em BFT ponderado por stake (ADR-0028). Os dois usam o mesmo motor (`teko-bft`) e o mesmo hospedeiro (`tools/teko-node/src/lane.rs`). **Nenhum bloco nasce sozinho:** a cadeia só cresce quando há entrada (voto) ou transação.

## Perfis

| Rede | Para quê | Estado |
|---|---|---|
| `devnet` | Sua máquina: 3 validadores em loopback (3 instituições **não toleram falha**; use 4 para tolerar 1) | ✅ `make devnet` |
| `testnet` | Várias máquinas, dados fictícios, regras iguais às de produção | ✅ perfil e `init`; rodar em várias máquinas exige editar `peers.txt` e habilitar escuta fora de loopback (não é o padrão) |
| `mainnet` | A eleição de verdade | 🔒 **TRAVADA**: `NetworkProfile::new(Mainnet)` e `network=mainnet` no arquivo são recusados |

Cada rede tem um `chain_id` (`teko-devnet-1`, `teko-testnet-1`). Ele entra no **cabeçalho de cada bloco** e no **manifesto assinado da eleição** (`ElectionManifest.chain_id`). O dispositivo (`trust.txt: chain_id=`) recusa manifesto de outra rede, e a apuração (`Tallier::with_chain_id`) ignora e conta manifestos de outra rede.

## Como funciona (rede de votos)

- **Validadores = instituições independentes, poder igual** (1 instituição = 1 voto, sem moeda nem stake). O conjunto é fixo no genesis (`network.txt`). Um bloco é **final** com o **certificado de commit**: mais de 2/3 das instituições deram precommit. Com 3 instituições o quórum é 3 (nenhuma falha tolerada); com 4, 3 (tolera 1).
- **Bloco** (`teko-chain`): `chain_id, height (a partir de 1), prev_hash, timestamp, proposer, entries[], log_size, log_root`; a rede decide o bloco **mais** o certificado (`DecidedBlock`). `log_size/log_root` fixam o estado exato do log Merkle depois do bloco; uma réplica refaz o **mesmo** log a partir dos blocos.
- **A cadeia só ordena.** Ela não julga cédulas: quem conta e rejeita (e conta as rejeitadas) é o `Tallier`, sobre o log. O validador confere só cadeia, altura, pai, proponente, tamanhos, horário e raiz Merkle.
- **Proponente sorteado pelo motor** (ponderado pelo poder, aqui igual). Se ele está fora do ar ou censura, estoura o *timeout* e a **rodada seguinte** tem outro proponente. Sem quórum a rede **pausa**; nunca bifurca.
- **Horário do bloco = o do proponente, conferido:** cada instituição só vota num bloco cujo horário difere do **próprio relógio** em até 60 s, e o horário é estritamente maior que o do bloco anterior (o replay do cliente confere só isso). Um relógio errado tem a proposta recusada.
- **Equívoco** (dois votos assinados em conflito) é evidência. A rede de votos não tem moeda, mas cada instituição põe uma **caução** na economia (ADR-0035): o nó que detecta a evidência a leva à economia como `SlashVotes` e **10% da caução é queimada** (some de circulação, ninguém recebe; a caução não dá poder de voto). Também é contado e mostrado no `status`.
- **Sem blocos vazios:** a altura só começa quando há entrada no mempool (após um curto intervalo, `block_interval_ms`, para agrupar uma rajada) ou quando os pares já a começaram (o nó ocioso também vota e se atualiza).
- **O cliente não confia em nenhum nó:** `NodeClient::sync_chain` baixa os blocos e re-verifica certificado, proponente, encadeamento e raiz Merkle. A Suite e o `election-admin` leem assim.
- **Persistência:** `dev/net/node-<i>/votes.blocks` (blocos finais, reverificados no boot) e `votes.wal` (rodada e *lock* do BFT, gravados antes de cada voto sair, ADR-0030). Redes criadas antes do ADR-0034 (`blocks.log`) precisam ser recriadas: `make devnet-reset`.

## Economia sobre BFT (M6a, ADR-0028)
**Primeiras moedas.** Como nenhum bloco nasce sozinho (ADR-0034), a rede precisa de moeda antes da primeira transação. O `teko-node init --founder tekow1… --alloc TEKO` (`make devnet FOUNDER=… ALLOC=…`, padrão 1.000 TEKO de teste) grava essa alocação **na gênese**, na carteira do fundador; todo nó e todo cliente a reproduz do arquivo `economy.txt` (`alloc=…`), no estado de altura 0. Ela conta para o teto e para a auditoria de oferta (`criadas = saldos + stakes + queimadas`), é rotulada como TESTE e só existe em redes de teste (a mainnet está travada). `--alloc 0` não dá moedas; outras carteiras podem receber alocações editando as linhas `alloc=` do `economy.txt` antes de subir. Sem `--founder` não há alocação (a conta do nó não é movimentada por ninguém).

Ao lado da cadeia de votos, cada nó roda o **livro de economia** (moeda, bens, stake) em consenso **BFT** ponderado por stake. Só o **validador fundador** (nó 0) vota no começo; os nós 1 e 2 **replicam**. As recompensas de bloco vão para a **carteira do fundador** (`make devnet FOUNDER=tekow1…`, o endereço da tela Carteira da Suite). Quando o stake total chega a N (1.000 TEKO na devnet local; `make devnet FOUNDER=… FOUNDER_TARGET=…` muda, em TEKO; ADR-0029), os validadores passam a ser os que têm stake. `make devnet-economy` mostra altura, fase e validadores e **verifica a cadeia**; `make devnet-founder FOUNDER=…` troca o fundador e zera a economia. Blocos vazios a cada 3 s (`TEKO_EMPTY_BLOCK_MS`) pagam as recompensas. Detalhes: [fluxo 12](flows/12-ledger-coin-and-assets.md).

## Rodando

**Ordem importa:** o fundador é a **sua carteira**, que só existe depois do onboarding no dispositivo e na Suite (`make emulator` e `make suite`; o endereço `tekow1…` está na aba Carteira). Só então suba a rede com `make devnet FOUNDER=tekow1…`. Se subiu a rede antes, `make devnet-stop && make devnet-founder FOUNDER=tekow1…` troca o fundador e zera a economia.

```
make devnet            # sobe 3 validadores (cria dev/net na 1ª vez; nenhum bloco nasce até haver transação ou voto)
make devnet-dkg       # (cédulas cifradas) cerimônia de chave entre os 3 validadores
make election          # liga a eleição à rede (chain_id) e fixa o dispositivo nela
                       # → reinicie o emulador (ele lê o trust.txt na partida)
make publish           # a eleição entra por blocos
make simulate-live     # votos sintéticos via rede
make devnet-status     # altura, mempool, pares de cada nó
make devnet-blocks     # blocos verificados no terminal
make devnet-equivocate  # SIMULAÇÃO: o validador de teste assina 2 votos conflitantes e leva Slash (trave stake na Suite antes)
make explorer          # explorador web (inclui a página de cada bem, com os metadados verificados em devnet) (blocos, transações, carteiras, validadores) em http://127.0.0.1:8545
make devnet-economy    # economia BFT: altura, fase de fundador, validadores (cadeia verificada)
make devnet-stop | devnet-reset
```
Sem rede rodando, tudo continua funcionando com o arquivo local (`dev/board.log`). Para forçar o arquivo: `TEKO_BOARD=file`. Em modo rede o quadro é **append-only**: `reset-board` não faz nada (use `make devnet-reset`).

## O que o BFT institucional garante, e o que NÃO

Garante: **finalidade** (um bloco decidido não é desfeito); nenhum bloco sem mais de 2/3 das instituições; censura por um proponente deixa de bastar (a rodada seguinte tem outro); sem quórum a rede pausa em vez de bifurcar; réplicas independentes reconstroem o mesmo log e conferem tudo; um nó fora do ar não para a rede (com 4 instituições ou mais).

**Não garante:** segurança com 1/3 ou mais das instituições em conluio (podem parar a rede; com 2/3 ou mais podem decidir o que quiserem); privacidade do voto além da cifra das cédulas ([ADR-0019](adr/0019-cedulas-cifradas-elgamal-de-limiar.md)); proteção de rede (TCP simples em loopback, sem criptografia entre nós: a integridade vem dos certificados); troca de instituições depois do genesis (outro ADR). Não há mineração nem tokens na rede de votos. Sem auditoria.
