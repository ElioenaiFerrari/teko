# Teko

**Teko** (guarani: "modo de ser", identidade) é um projeto de estudo: um **dispositivo de identidade cívica** em formato USB (hoje emulado), a **Teko Suite** (app desktop, Tauri v2) e uma **rede federada de validadores** que guarda uma votação com cédulas cifradas. Tudo em Rust.

> Protótipo de estudo. Chaves e PINs são **de teste**; a criptografia é própria e **sem auditoria**; não há valor real nem validade jurídica.

## Arquitetura em uma imagem

![Arquitetura do Teko: dispositivo, Suite, emissor, órgãos eleitorais, validadores, verificador independente e os livros de votos e de economia](docs/diagrams/architecture-overview.png)

O computador (Suite) **nunca** vê chaves nem PIN. O dispositivo cifra e assina a cédula; a Suite só a repassa; os validadores conferem provas e gravam em blocos assinados M-de-N; qualquer um pode refazer a verificação.

## Mapa de crates

![Camadas dos crates, da fundação até os binários e apps](docs/diagrams/crate-layers.png)

| Crate | Papel |
|---|---|
| `teko-protocol` | mensagens, framing e versões do protocolo Suite ↔ dispositivo |
| `teko-crypto` | derivação, hashes etiquetados, ID Cívico (bech32m), assinaturas, credenciais |
| `teko-device-core` | máquina de estados, PIN e armazenamento do dispositivo (`no_std`) |
| `teko-wallet` | carteira: frase BIP-39 de 24 palavras (gerada no dispositivo), derivação SLIP-10 P-256 e endereço `tekow1…`; independente da identidade |
| `teko-attestation` | manifestos de firmware, M-de-N e cadeias de atestação |
| `teko-election` | manifesto da eleição, ciclo de vida dos pleitos, apuração |
| `teko-registration` | registro anônimo por assinatura cega (RFC 9474) |
| `teko-ballot-crypto` | ElGamal de limiar P-256, provas 0/1 e soma, DKG de Feldman |
| `teko-explorer` | índice do explorador: reexecuta e verifica a cadeia de economia e responde blocos, transações, contas, validadores, bens e busca (sem I/O) |
| `devnet-sim` | simuladores só para a rede de teste local; hoje, a equivocação de um validador para ver a penalidade (`make devnet-equivocate`); nunca em validador ou release |
| `explorer-server` | explorador da chain: página web local + API JSON somente leitura sobre uma cadeia verificada (`make explorer`) |
| `teko-economy` | economia sobre consenso BFT: blocos com certificado de commit, conjunto de validadores por stake com fase de fundador, verificação por clientes |
| `teko-bulletin` | quadro de avisos: log Merkle append-only, provas e head assinado |
| `teko-chain` | blocos M-de-N, perfis devnet/testnet (mainnet travada), réplica verificada |
| `teko-bft` | consenso BFT estilo Tendermint ponderado por stake (ainda fora do nó) |
| `teko-ledger` | livro-razão genérico: transações assinadas, módulos, blocos atômicos e raiz de estado (base da moeda e dos bens) |
| `teko-coin` | moeda de teto fixo: recompensa com *halving*, taxa por byte, stake, penalidade por equivocação e faucet de teste (só devnet) |
| `teko-assets` | bens: classes (imóvel e veículo) governadas por órgão M-de-N, metadados fora da chain com hash amarrado ao consentimento, transferência com dono + comprador + órgão, gravames e venda contra pagamento atômica |
| `teko-transport` | trait de transporte e implementação local |
| `teko-client` | cliente da Suite: onboarding, assinatura, eleição |
| `device-emulator` | "tela e botões" do dispositivo, em processo separado |
| `issuer-cli` | autoridade simulada: credenciais, reemissão e revogação |
| `election-admin-cli` | órgãos eleitorais: publicar, simular votos (rotulados), apurar |
| `teko-node` | validador (gossip, rodízio, DKG) e cliente que não confia em nós |
| `teko-suite` (`apps/suite`) | app Tauri v2 |

## Início rápido (do zero)

Pré-requisitos: Rust (o `rust-toolchain.toml` fixa a versão) e, para a Suite, as dependências do Tauri v2. `make` lista todos os atalhos.

```bash
make clean-dev      # opcional: apaga o estado local de testes
make check          # opcional: formato + clippy + testes + docs
make setup          # 1x: chaves de teste, firmware assinado e "fábrica" (pasta dev/)
make devnet FOUNDER=tekow1…   # rede de TESTE: votos e economia em BFT (a cadeia só cresce com voto ou transação) (FOUNDER = endereço da aba Carteira; recebe as recompensas)
make devnet-dkg     # cerimônia de chave entre os validadores (cédulas cifradas)
make election       # cria a eleição de exemplo (7 pleitos)
make emulator       # terminal 1: tela e botões do dispositivo (PIN aqui). Reinicie após make election
make publish        # publica a eleição no quadro
make suite          # terminal 2: onboarding (PIN, carteira com frase de 24 palavras, ID Cívico) e votar
make simulate N=200 # opcional: eleitores SINTÉTICOS (sempre rotulados)
make devnet-status  # estado da rede     |  make devnet-blocks  # blocos
make tally          # apuração no terminal
make close          # encerra a votação; depois a Suite mostra os gráficos
make devnet-stop    # para os validadores
make fund ADDR=tekow1… AMOUNT=1000   # TEKO de TESTE para a sua carteira, aparecem em "Carteira" na Suite (aceita decimais: AMOUNT=0,5; 1 TEKO = 1.000.000 ypy, ADR-0029)
make tally-time     # tempo de verificação do quadro de votos (paralelo; ADR-0033)
make devnet-equivocate  # SIMULA a equivocação do validador de teste e mostra a penalidade (trave stake na Suite antes)
make coin-demo      # demonstração da moeda em memória: teto, halving, taxa, stake e slash
make assets-demo    # demonstração dos bens: registro com quórum, gravame, venda contra pagamento
```

Demo sem teclado (**nunca** em produção): `cargo run -p device-emulator --features test-buttons -- --auto-pin 482913`.

## Estado atual
- **Fase 1** (dispositivo, Suite, emissor): completa.
- **Fase 2** (votação): eleição com vários pleitos e jurisdições, registro anônimo, **cédulas cifradas** até o fechamento, rede de votos em **BFT institucional** (finalidade; sem blocos vazios; ADR-0034), reemissão sem seed (identidade), resultados em gráficos e **carteira com frase de recuperação de 24 palavras** (criada e confirmada no dispositivo, estilo Trezor; ADR-0023).
- **Economia sobre BFT (M6a):** a moeda, os bens e o stake rodam em consenso BFT nos nós (`make devnet-economy`), com **fase de fundador** sem pré-mineração (as recompensas vão para a carteira do dono) e validadores por stake depois (ADR-0028); a Suite verifica a cadeia dos nós.
- **Antes disso** (programa do livro-razão): consenso BFT (`teko-bft`) e núcleo do livro (`teko-ledger`) **moeda** (`teko-coin`) e **bens** (`teko-assets`) prontos como biblioteca; a **carteira** já tem endereço, frase de recuperação (M5a) e **envia moedas assinando no dispositivo** (M5b, contra a devnet local de teste); o dispositivo também **assina o seu consentimento na venda de bens** (M5c) a Suite abre numa tela **Início** (resumo) e tem as telas **Validadores** (stake assinado no dispositivo) e **Bens** (venda e compra, com órgão e contraparte **simulados**, só em desenvolvimento); negociação entre pessoas reais e os votos sobre o BFT (M6) ainda só **planejados**.
- **Fora de escopo**: hardware real, validade jurídica (ICP-Brasil), prova ZK de elegibilidade, explorador público (só placeholder).

## Documentação
**Quer entender os conceitos** (hash, assinatura, HRP, consenso BFT, DKG, halving, M-de-N…)? Leia o [guia de conceitos](docs/concepts/README.md), com diagramas e código Rust. Comece por [docs/README.md](docs/README.md): um fluxo curto por assunto, todos em pt-BR e com diagramas ([visão geral](docs/flows/00-overview.md), [assinatura](docs/flows/02-signing.md), [votação](docs/flows/05-encrypted-voting.md), [blocos](docs/flows/09-blocks-and-validation.md), [BFT](docs/flows/11-bft-consensus.md)...). Referência: [ARCHITECTURE](docs/ARCHITECTURE.md), [THREAT-MODEL](docs/THREAT-MODEL.md), [TEST-MATRIX](docs/TEST-MATRIX.md), [ADRs](docs/adr/README.md). Contexto para o assistente: [CLAUDE.md](CLAUDE.md).

## Qualidade e documentação viva
`make check` roda formato, clippy (`-D warnings`), testes e `make docs-check`. Também: `make verify-fw`, `make release-check` (o build normal não tem botões automáticos nem `dev-simulation`) e `make repro` (build reproduzível, [RELEASE](docs/RELEASE.md)). Os diagramas vêm de `scripts/diagrams/` (`make diagrams`; tema em [TEMA](docs/diagrams/THEME.md)). **Regra:** mudou comportamento, comando, crate ou fluxo? Atualize este README e o fluxo correspondente no mesmo trabalho.

> Chaves e PINs aqui são **de teste**. O emulador não tem isolamento de hardware (ver [THREAT-MODEL](docs/THREAT-MODEL.md)).
