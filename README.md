# Teko

Dispositivo de identidade cívica (emulado) + **Teko Suite** (Tauri v2), em Rust. Projeto de estudo.
Leia `CLAUDE.md` e `docs/` para a visão, o modelo de ameaças e o protocolo.

## Atalhos (Makefile)

`make` lista tudo. O essencial: `make setup` (uma vez), `make emulator` (terminal 1), `make suite` (terminal 2), `make check` (formato + clippy + testes).

## Rodando sem make (3 terminais, na raiz do repositório)

```bash
# 0) uma vez: cria chaves de teste, firmware v1/v2 assinados 2-de-3 e o material de "fábrica"
cargo run -p issuer-cli -- dev-setup

# 1) o dispositivo: a "tela e botões" ficam NESTE terminal (PIN e confirmações são digitados aqui)
cargo run -p device-emulator

# 2) a Suite (janela desktop)
cargo run -p teko-suite
```

Na Suite, siga **Configurar dispositivo**: autenticidade → firmware → PIN (digitado no emulador) → ID Cívico → credencial.
Depois: ver identidade, assinar documentos, provar atributos e **votar por pleito** (protótipo: Presidente para todos; governador e prefeito só do seu estado/município; a cédula é assinada pelo dispositivo mas ainda não é enviada). Sem emulador ligado, a Suite abre no **Explorador público** (placeholder).

- Apagar o estado do dispositivo (voltar a "virgem"): `cargo run -p device-emulator -- --reset`
- Outro endereço: `TEKO_DEVICE_ADDR=127.0.0.1:9000 cargo run -p teko-suite` (e `--listen` no emulador)

## Pré-visualizar a interface sem Tauri (só desenvolvimento)

```bash
python3 apps/suite/dev/preview.py   # http://127.0.0.1:8765/?mock=virgin|locked|ready|none
```
Usa um backend falso (`apps/suite/dev/mock.js`, **fora** do pacote do app). Serve apenas para ajustar o visual.

## Qualidade

```bash
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace
```

O teste ponta a ponta (`crates/teko-client/tests/e2e.rs`) sobe emulador + `issuer-cli` + cliente da Suite (sem janela) por TCP e executa o onboarding completo.

Mais: `make verify-fw` (confere o firmware sem a Suite), `make release-check` (build normal sem botões automáticos) e `make repro` (build reproduzível). Veja `docs/RELEASE.md` e `docs/TEST-MATRIX.md`.

> Chaves e PINs aqui são **de teste**. O emulador não tem isolamento de hardware (ver `docs/THREAT-MODEL.md`).
