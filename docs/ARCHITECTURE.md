# Arquitetura

Status: **rascunho para revisão**.

## 1. Visão geral

```
┌──────────────────────── HOST (NÃO CONFIÁVEL) ────────────────────────┐
│  Suite (Tauri v2)                                                    │
│  ┌──────────────┐  comandos Tauri  ┌───────────────────────────┐     │
│  │ Frontend web │ ───────────────▶ │ Backend Rust              │     │
│  │ (UI)         │ ◀─────────────── │ teko-protocol, transport  │     │
│  └──────────────┘                  └─────────────┬─────────────┘     │
└───────────────────────────────────────────────────┼──────────────────┘
                                   Transport (TCP loopback agora; USB HID depois)
┌──────────────────── DISPOSITIVO (CONFIÁVEL) ──────┼──────────────────┐
│  device-emulator (processo separado)              ▼                  │
│  ┌────────────────┐  HAL  ┌──────────────────────────────────┐       │
│  │ "Tela" + "Botões"│◀───▶│ teko-device-core (no_std)        │       │
│  │ (PIN, confirma.) │     │ estado, PIN, chaves, assinaturas │       │
│  └────────────────┘      └──────────────────────────────────┘       │
│  bootloader simulado: manifesto M-de-N + anti-downgrade              │
└──────────────────────────────────────────────────────────────────────┘

┌─ AUTORIDADE (simulada) ─────────┐     ┌─ Registro de commitments (stub) ─┐
│ issuer-cli: raízes de teste,    │     │ canal SEPARADO, sem vínculo com  │
│ firmware M-de-N, atestação,     │     │ a identidade (ver ADR-0008)      │
│ credenciais de cidadão          │     └──────────────────────────────────┘
└─────────────────────────────────┘
```

## 2. Fronteiras de confiança

| Zona | Componentes | Confiança |
|---|---|---|
| Dispositivo | `teko-device-core`, HAL, tela/botões do emulador, bootloader | Confiável (no emulador: só por convenção; ver THREAT-MODEL §Premissas) |
| Host | Suite (frontend + backend), transporte, SO | **Não confiável** |
| Autoridade | `issuer-cli`, chaves raiz | Confiável com restrições (M-de-N; ver THREAT-MODEL §Autoridade maliciosa) |

Regra: o host só vê dados públicos, assinaturas e provas. A Suite **não possui** UI nem canal para PIN.

## 3. Crates

| Crate | Responsabilidade | Depende de | `no_std` |
|---|---|---|---|
| `teko-protocol` | Mensagens tipadas, framing, versão, códigos de erro | — | sim (+alloc) |
| `teko-crypto` | Tagged hashes, derivação, ID Cívico bech32m, pairwise, stubs Poseidon/ZK/credenciais | — | sim (+alloc) |
| `teko-device-core` | Máquina de estados, PIN, armazenamento, tratamento de mensagens, HAL traits | protocol, crypto | sim (+alloc) |
| `teko-attestation` | Manifesto de firmware, M-de-N, anti-downgrade, cadeias de atestação | crypto | sim (+alloc) para verificação; emissão só `std` |
| `teko-transport` | `trait Transport`, implementação local (TCP) | protocol | não |
| `device-emulator` | Binário: janela/terminal com tela e botões; hospeda o core; bootloader simulado | core, attestation, transport | não |
| `issuer-cli` | Binário: autoridade simulada | attestation, crypto, protocol | não |
| `suite` | App Tauri v2 | protocol, transport, attestation (verificação), crypto (verificação) | não |

Grafo acíclico: `protocol` e `crypto` são folhas; `device-core` e `attestation` acima; binários no topo. `teko-protocol` **não** depende de tipos de segredo de `teko-crypto`.

**Adições da Fase 1:** `teko-client` (lógica do lado do host: atestação, firmware, assinatura, provas; usada pela Suite e testada headless). `teko-device-core` depende também de `teko-attestation` (o bootloader verifica manifestos). `device-emulator` e `issuer-cli` são **lib + bin** (a lib permite o teste e2e). O emulador usa **terminal** como tela/botões (ADR-0009); a feature `test-buttons` (só testes/demos) liga os botões automáticos e **não** entra no build normal.

**Fase 2 (parcial):** `teko-election` (`no_std`: manifesto assinado M-de-N e ciclo de vida) e `teko-bulletin` (`std`: log Merkle append-only atrás de `trait BulletinBoard`). `teko-attestation` ganhou `multisig` (M-de-N genérico, usado por firmware e eleição).

## 4. HAL do dispositivo

Traits em `teko-device-core`, implementadas pelo emulador agora e pelo MCU depois:

- `Display` — mostra texto/ID/hash e prompts.
- `Buttons` — leitura de eventos (confirmar/rejeitar/dígitos do PIN).
- `SecureStorage` — persistência atômica, com contador de tentativas à prova de rollback (no emulador: arquivo; limitação documentada).
- `Rng` — fonte de entropia (CSPRNG no emulador).
- `Clock` — monotônico, para atraso exponencial do PIN.

O PIN é lido por `Buttons` e nunca transita pelo `Transport`.

## 5. Bootloader simulado

Antes de "executar" o firmware: verifica manifesto (versão, hash do blob, ≥M assinaturas válidas de chaves distintas dentre as N raízes), compara a versão com o contador monotônico (recusa downgrade), só então marca como instalado. Falha → estado permanece, erro `Downgrade` / `BadSignature` / `InsufficientSignatures`.

## 6. Estratégia de testes

- Unit + `proptest` por invariante (lista em CLAUDE.md), cada um mapeado a uma ameaça em THREAT-MODEL.
- Core testado com HAL falso determinístico.
- E2E headless: emulador (feature `test-buttons` com botões scriptados) + `issuer-cli` + Suite sem janela, rodando o onboarding inteiro. A feature **não** é compilada no build normal; CI verifica sua ausência no binário de release.

## 7. Releases e verificação

- **Firmware:** open source, build reproduzível (toolchain fixada, `--locked`, caminhos normalizados, `SOURCE_DATE_EPOCH`). Manifesto com assinaturas M-de-N publicado junto do hash. O usuário/instituições reconstroem e comparam o hash.
- **Suite:** build reproduzível; artefatos assinados; hashes publicados em canal independente; instruções de verificação no README de release. A Suite também **verifica o firmware antes de enviar**, mas a decisão final é do bootloader.
- **Dependências:** `cargo-deny` e `cargo-vet` (ou equivalente) no CI.

## 8. Perguntas em aberto

1. **Frontend da Suite** (React / Svelte / TS puro). Alternativas: React (ecossistema), Svelte (leve, menos superfície de dependências), TS puro (mínima superfície). *Recomendação:* Svelte ou TS puro, pela menor superfície de supply chain. **Decisão: PENDENTE (dono do produto)**; só necessária na etapa da Suite.
2. **Emulador: janela ou terminal** → ADR-0009. **PENDENTE**.
3. **Transporte local** → ADR-0010. **PENDENTE**.
4. **Armazenamento do emulador e rollback:** arquivo simples não protege o contador de PIN contra restauração de backup. Alternativas: aceitar a limitação no emulador e documentar; ou simular contador monotônico em arquivo separado. *Recomendação:* aceitar e documentar; resolver no hardware (ADR-0012). **PENDENTE**.
5. **`teko-attestation` no device-core:** a verificação de manifesto precisa rodar no bootloader (`no_std`); a emissão, só no `issuer-cli`. *Recomendação:* dividir por feature `std`. **PENDENTE** (decisão técnica, baixa criticidade).
6. **Estrutura do repositório:** `tools/` e `apps/` como membros do workspace. *Recomendação:* sim, tudo no mesmo workspace. **PENDENTE**.
