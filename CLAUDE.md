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
tools/device-emulator/     4. "tela" e "botões" simulados, processo separado
crates/teko-attestation/   5. manifestos de firmware, M-de-N, cadeias de atestação
tools/issuer-cli/          5. autoridade simulada
crates/teko-transport/     (junto de 4) trait Transport + implementação local
apps/suite/                6. Tauri v2
```

Cada etapa só começa com a anterior com testes passando. Ao fim de cada etapa, atualizar os docs e resumir o que mudou.

## Princípios inegociáveis e a regra operacional de cada um

1. **O computador é não confiável.** Nada que a Suite/host faça pode extrair chaves ou alterar o que é assinado. → O host só recebe assinaturas, provas e dados públicos. O que será assinado é derivado **dentro do dispositivo** a partir do que ele mostra na tela.
2. **Confirmação sensível só no dispositivo** (PIN, conteúdo a assinar, voto). → Nenhuma mensagem do protocolo carrega PIN. A Suite nunca tem campo de entrada de PIN. Assinatura só é produzida após confirmação física.
3. **Chaves nascem no dispositivo e nunca são exportadas.** → Tipos de segredo não implementam `Serialize`, `Debug` (ou implementam redigido), `Clone` desnecessário; `zeroize` no `Drop`. `teko-protocol` não depende de tipos de segredo.
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
make                                   # lista todos os atalhos (check, repro, release-check, verify-fw...)
# demo sem teclado (NUNCA em produção): cargo run -p device-emulator --features test-buttons -- --auto-pin 482913
```

## Documentos

- [ARCHITECTURE](docs/ARCHITECTURE.md) · [THREAT-MODEL](docs/THREAT-MODEL.md) · [KEY-HIERARCHY](docs/KEY-HIERARCHY.md)
- [DEVICE-PROTOCOL](docs/DEVICE-PROTOCOL.md) · [ONBOARDING-FLOW](docs/ONBOARDING-FLOW.md) · [GLOSSARY](docs/GLOSSARY.md)
- [TEST-MATRIX](docs/TEST-MATRIX.md) (invariante → teste) · [RELEASE](docs/RELEASE.md) (build reproduzível, verificação de firmware/release)
- [ADRs](docs/adr/README.md)

## Fora de escopo nesta fase

Hardware real, assinatura com validade jurídica (ICP-Brasil), prova ZK real, blockchain de votação, governança eleitoral, explorador público (só placeholder).
