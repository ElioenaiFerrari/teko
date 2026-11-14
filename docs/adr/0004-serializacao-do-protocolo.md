# ADR-0004 — Serialização do protocolo

**Status:** Aceito provisoriamente (2026-11-13: dono do produto mandou seguir; recomendação adotada: prost (derive manual, sem protoc)) · revisável

## Contexto
O protocolo (DEVICE-PROTOCOL) precisa de mensagens tipadas e versionáveis, parser total, e compatibilidade com `no_std` para o MCU. O host futuramente pode estar em outras linguagens.

## Alternativas
| | protobuf (`prost`) | `postcard` (serde) | CBOR (`minicbor`/`ciborium`) |
|---|---|---|---|
| `no_std` + `alloc` | Sim | Sim (inclusive sem alloc) | Sim (`minicbor`) |
| Esquema independente de linguagem | **Sim** (`.proto`) | Não (esquema = tipos Rust) | Parcial (CDDL) |
| Evolução (campos opcionais) | Excelente | Frágil (posicional) | Boa |
| Tamanho/CPU | Bom | Mínimo | Bom |
| Precedente | Trezor | Embedded Rust | FIDO/COSE |
| Canonicidade | Não garantida | Sim (determinístico) | Possível (deterministic CBOR) |
| Dependência `build.rs`/`protoc` | Sim | Não | Não |

## Recomendação
`prost` pelo esquema multi-linguagem, evolução segura e precedente (Trezor), **com** o framing próprio da §2 do DEVICE-PROTOCOL. `postcard` é a alternativa se a simplicidade e a ausência de `protoc` pesarem mais. Coisas a assinar (manifestos, credenciais) **não** devem depender da serialização do protocolo: usar codificação canônica própria e determinística.

## Consequências
Com `prost`, exige `protoc` (ou `protox`) no build e fixar versão para builds reproduzíveis.
