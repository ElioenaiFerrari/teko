# Teko — contexto persistente

**Teko** (guarani: "modo de ser", identidade) é um projeto de estudo: um **dispositivo de identidade cívica** em formato USB (estilo Trezor/Ledger) e a **Suite** desktop que o acompanha (Tauri v2, estilo Trezor Suite), em Rust. Nesta fase o dispositivo é um **emulador**. A blockchain de votação e a governança eleitoral vêm em fases futuras e consomem o que é construído aqui.

Idioma dos documentos: pt-BR. Identificadores, mensagens, crates e código: inglês.

## Estado atual

- **Fase 1 implementada** (specs aprovados em 2026-11-13 com "manda bala"): todos os crates, emulador, `issuer-cli` e Suite Tauri funcionam; 84 testes (unit, proptest, e2e) passam.
- Decisões adotadas provisoriamente: ver `docs/adr/README.md` e a memória do projeto. O que continua PENDENTE: ADR-0005 (formato final de credencial), 0006, 0007, 0008 (registro de commitment), 0011, 0012.

## Workspace-alvo e ordem de implementação

```
crates/teko-protocol/      1. mensagens, framing, versionamento
crates/teko-crypto/        2. derivação, tagged hashes, ID Cívico (bech32m), stubs Poseidon/ZK/credenciais
crates/teko-device-core/   3. máquina de estados, PIN, armazenamento (no_std-friendly)
crates/teko-election/      (Fase 2) manifesto de eleição M-de-N + ciclo de vida (no_std)
crates/teko-registration/  (Fase 2) assinatura cega RFC 9474 (registro anônimo de eleitor, ADR-0008)
crates/teko-ballot-crypto/  (Fase 2) ElGamal exponencial de limiar P-256, provas 0/1 e soma, DKG Feldman (sem auditoria)
crates/teko-ledger/        (Fase 2, M2) livro-razão genérico: Tx assinada, módulos, blocos atômicos, raiz de estado (ADR-0020)
crates/teko-coin/          (Fase 2, M3) moeda de teto fixo: halving, taxa por byte, stake, slash, faucet devnet (ADR-0021)
crates/teko-assets/        (Fase 2, M4) bens: classes com órgão M-de-N, negociação direta com janela de veto (oferta, aceite com escrow, veto, finalização; ADR-0036), gravames
crates/teko-credentials/   (Fase 2, etapa A) credenciais na chain: tipos com órgão M-de-N, janela de contestação, validade, veto, revogação (ADR-0037)
crates/teko-wallet/        (Fase 2, M5a) carteira: BIP-39 24 palavras (só na tela do dispositivo), SLIP-10 P-256, endereço tekow1…; independente da identidade (ADR-0023)
crates/teko-economy/       (Fase 2, M6a) economia sobre BFT: blocos com certificado, validadores por stake, fase de fundador (ADR-0028)
crates/teko-chain/         (Fase 2) rede de votos em **BFT institucional** (ADR-0034): perfis devnet/testnet (mainnet TRAVADA), blocos com certificado de commit, réplica verificada
crates/teko-explorer/      (Fase 2) índice verificado da cadeia de economia: blocos, txs, contas, validadores, busca (ADR-0031)
tools/devnet-sim/           (Fase 2) simuladores SÓ de dev: equivocação do validador de teste (`make devnet-equivocate`), blocos sob demanda (`make devnet-blocks-sim`)
tools/teko-dev/            (Fase 2) menu de desenvolvimento com setas (ratatui) sobre o Makefile: `make dev`; só dev, nunca em release
tools/explorer-server/     (Fase 2) explorador: web local + API JSON somente leitura (`make explorer`)
tools/teko-node/           (Fase 2) validador (gossip, rodízio, troca de turno) + cliente que não confia em nós
crates/teko-bulletin/      (Fase 2) log Merkle append-only, provas, head assinado, leitor tolerante
tools/election-admin-cli/  (Fase 2) publicar no quadro, simular votos (rotulados), apurar
tools/device-emulator/     4. "tela" e "botões" simulados, processo separado
crates/teko-attestation/   5. manifestos de firmware, M-de-N, cadeias de atestação
tools/issuer-cli/          5. autoridade simulada de TESTE (chaves, firmware, eleição de exemplo, registro anônimo; a emissão do ID cívico saiu: é do Emissor, ADR-0038)
crates/teko-transport/     (junto de 4) trait Transport + implementação local
apps/suite/                6. Tauri v2
apps/issuer/               (Fase 2, etapa A/D) Teko Emissor: app separado, órgãos emitem credenciais e o ID cívico na chain (`make issuer`, ADR-0037/0038)
```

Cada etapa só começa com a anterior com testes passando. Ao fim de cada etapa, atualizar os docs e resumir o que mudou.

## Princípios inegociáveis e a regra operacional de cada um

1. **O computador é não confiável.** Nada que a Suite/host faça pode extrair chaves ou alterar o que é assinado. → O host só recebe assinaturas, provas e dados públicos. O que será assinado é derivado **dentro do dispositivo** a partir do que ele mostra na tela.
2. **Confirmação sensível só no dispositivo** (PIN, conteúdo a assinar, voto). → Nenhuma mensagem do protocolo carrega PIN. A Suite nunca tem campo de entrada de PIN. Assinatura só é produzida após confirmação física.
3. **Chaves nascem no dispositivo e nunca são exportadas.** (Única exceção, ADR-0023: a frase de recuperação da **carteira** é mostrada ao humano, só na tela do dispositivo, uma vez; a identidade e o voto seguem sem seed.) → Tipos de segredo não implementam `Serialize`, `Debug` (ou implementam redigido), `Clone` desnecessário; `zeroize` no `Drop`. `teko-protocol` não depende de tipos de segredo.
4. **Firmware só roda se assinado M-de-N** por chaves de instituições independentes; open source; build reproduzível. → Nunca existe caminho de código com chave única de firmware. Downgrade é recusado.
5. **Separação total identidade ↔ voto.** → O segredo de eleitor não é derivável da identidade nem vice-versa; nenhuma mensagem/estrutura contém ambos; o registro de commitment usa canal separado.
6. **Atestação sem rastreamento.** → Chaves de lote (muitos dispositivos por chave); a atestação nunca acompanha um voto.

## Regras de engenharia

- Rust stable. `cargo fmt` e `cargo clippy --workspace --all-targets -- -D warnings` limpos.
- Sem `unwrap`/`expect`/`panic!`/indexação que possa entrar em pânico em **código de biblioteca** (permitido em testes). Habilitar `clippy::unwrap_used`, `clippy::expect_used` nos crates de lib.
- Erros com `thiserror` (em `no_std`, usar `thiserror` com suporte `no_std` ou enum manual — decidir na etapa 1).
- Libs de referência: `sha3`, `ed25519-dalek` e/ou `p256` (ver ADR-0002), `bech32`, `zeroize`, `subtle` (comparação em tempo constante, inclusive do PIN), `thiserror`, `proptest`.
- `teko-device-core` é `no_std`-compatível (`alloc` permitido); `teko-protocol` e `teko-crypto` também, quando possível. HAL por traits (`Display`, `Buttons`, `SecureStorage`, `Rng`, `Clock`).
- **TDD** com `proptest` cobrindo: PIN errado N vezes apaga o dispositivo; firmware sem M-de-N é recusado; downgrade é recusado; nenhuma mensagem do protocolo expõe chave privada ou PIN; IDs pairwise de serviços distintos são distintos e não correlacionáveis; ID Cívico tem checksum válido e detecta erros de digitação; assinatura só sai depois da confirmação no dispositivo.
- E2E: emulador + `issuer-cli` + Suite (headless) executando o onboarding completo. Botões automatizados do emulador existem **somente** sob feature de teste, nunca no build normal.

## Documentação viva (regra permanente)

- Toda mudança de **comportamento, comando `make`, crate, protocolo ou fluxo** atualiza, **no mesmo trabalho**, o `README.md` (estado atual, mapa de crates, início rápido) e o fluxo correspondente em `docs/flows/` (texto **e** diagrama). Quando couber, também `TEST-MATRIX`, `THREAT-MODEL` e ADR.
- Tudo em **pt-BR** (código e identificadores em inglês). **Nomes de pastas e de arquivos sempre em inglês** (`docs/flows`, `docs/diagrams`, `docs/concepts`, `scripts/diagrams`); em pt-BR fica **só o conteúdo** dos documentos. Fluxos seguem o molde: resumo; quem participa; diagramas; passo a passo; o que é verificado e por quem; o que pode dar errado; onde está no código; testes que cobrem; limites honestos. Funcionalidade só planejada leva o aviso "planejado".
- **Diagramas sempre pela skill `diagram-design`**, no tema Teko (`docs/diagrams/THEME.md`): edite o gerador em `scripts/diagrams/` e rode `make diagrams` (gera só o PNG; SVG/HTML não são guardados). Não edite os arquivos gerados à mão. Embuta o PNG no markdown com texto alternativo em pt-BR.
- `make docs-check` (links, imagens, texto alternativo, crates no README, títulos em pt-BR) faz parte de `make check` e do CI.

## Fluxo de trabalho

1. Spec (docs) → aprovação do dono do produto → teste → código → atualizar docs.
2. Toda decisão de segurança ou produto vira **ADR** em `docs/adr/`.
3. **Nunca decidir sozinho** itens marcados `Decisão: PENDENTE`. Propor alternativas e recomendação; esperar resposta.
4. Não commitar sem pedido explícito.

## Comandos (preencher conforme existirem)

```
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo run -p issuer-cli -- dev-setup   # 1x: chaves de teste, firmware, fábrica (pasta dev/, ignorada pelo git)
cargo run -p device-emulator           # terminal 1: tela e botões do dispositivo (PIN aqui!)
cargo run -p teko-suite                # terminal 2: a Suite (Tauri v2)
make docs-check                        # confere links, imagens e crates no README
make fund ADDR=tekow1… AMOUNT=1000    # TEKO de TESTE na carteira (devnet local; decimais ok; 1 TEKO = 1.000.000 ypy, ADR-0029)
make diagrams                         # regenera os diagramas (docs/diagrams)
make devnet FOUNDER=tekow1… [NODES=4]  # (1ª vez; NODES = instituições, padrão 4) FOUNDER = sua carteira: recebe as recompensas do fundador (ADR-0028)
make candidates                        # SIMULA o órgão emitindo candidaturas (ADR-0040); só dev
make claim                             # resultados assinados pelas instituições -> credencial vitalícia `eleito`
make runoff CONTEST=governador-sp      # abre o 2º turno
make issuer                            # app Emissor: órgãos emitem credenciais na chain (ADR-0037)
make devnet-cosign                     # SIMULA membros de teste dos órgãos aprovando credenciais; só dev
make devnet-veto CLASS=imovel ASSET=<id>  # SIMULA o órgão vetando uma negociação de bem pendente (ADR-0036); só dev
make dev                               # menu com setas: sobe/derruba a devnet, simula blocos, roda os outros atalhos (tools/teko-dev)
make tie CONTEST=governador-sp VOTES=5,5   # SIMULA um empate (planta votos exatos); depois make close e veja o sorteio (ADR-0043)
make devnet-blocks-sim KIND=economy COUNT=5  # SIMULA N blocos (economy|votes); só dev
make devnet-equivocate                 # SIMULA a equivocação do validador de teste (penalidade/Slash); só dev
make tally-time                        # apura o quadro (release) e mostra o tempo; TEKO_VERIFY_THREADS=1 = sequencial
make explorer                          # explorador da chain em http://127.0.0.1:8545 (precisa de make devnet)
make devnet-economy                    # economia BFT: altura, fase de fundador, validadores
make devnet                            # blockchain de TESTE local: 3 validadores em BFT, sem blocos vazios (docs/NETWORKS.md)
make                                   # lista todos os atalhos (check, repro, release-check, verify-fw...)
# demo sem teclado (NUNCA em produção): cargo run -p device-emulator --features test-buttons -- --auto-pin 482913
```

## Documentos

- [README](README.md) · [Índice da documentação e fluxos](docs/README.md)

- [Guia de conceitos](docs/concepts/README.md) (hash, assinatura, HRP, BFT, DKG, halving, M-de-N… com Rust) · [ARCHITECTURE](docs/ARCHITECTURE.md) · [THREAT-MODEL](docs/THREAT-MODEL.md) · [KEY-HIERARCHY](docs/KEY-HIERARCHY.md)
- [DEVICE-PROTOCOL](docs/DEVICE-PROTOCOL.md) · [ONBOARDING-FLOW](docs/ONBOARDING-FLOW.md) · [GLOSSARY](docs/GLOSSARY.md)
- [TEST-MATRIX](docs/TEST-MATRIX.md) (invariante → teste) · [RELEASE](docs/RELEASE.md) (build reproduzível, verificação de firmware/release)
- [ADRs](docs/adr/README.md)

## Próxima fase

**Fase 2 (votação e governança eleitoral; etapa C: o eleitor confere o próprio voto, ADR-0041: desafio de Benaloh + `CheckMyBallot` no dispositivo; branco e nulo como opções explícitas fora da base da maioria, ADR-0042; desempate por sorteio verificável (VRF de limiar), ADR-0043): especificação em [docs/PHASE-2-VOTING.md](docs/PHASE-2-VOTING.md), aguardando revisão. Etapas 1-8 implementadas (a 8 = **cédulas cifradas** ElGamal de limiar P-256 + DKG entre validadores, ADR-0019; `make devnet-dkg`; só os totais são decifrados no fechamento) (a 7 = recuperação **sem seed**: reemissão presencial com revogação, [docs/RECOVERY.md](docs/RECOVERY.md); feature `dev-simulation` isola a autoridade/operador de teste do build de produção) (a 6 = rede de teste federada PoA, ver [docs/NETWORKS.md](docs/NETWORKS.md); `make devnet`) (`teko-election` com vários pleitos/jurisdições e apuração incremental, `teko-bulletin`, dispositivo + Suite votando por pleito, quadro local, resultados ao vivo por cargo/estado, **registro anônimo por assinatura cega RFC 9474** em `teko-registration` com chave RSA por pleito, ADR-0008 aceito provisoriamente; a cédula entra no quadro **local** do protótipo, ainda sem prova ZK nem nullifier); o resto depende das decisões do §10.** Decisões pendentes listadas em §10 de lá. Achado importante: o stub `GetVoterCommitment` da Fase 1 será substituído por credencial de eleitor baseada em par de chaves.

## Fora de escopo nesta fase

Hardware real, assinatura com validade jurídica (ICP-Brasil), prova ZK real, blockchain de votação, governança eleitoral, explorador público (só placeholder).
