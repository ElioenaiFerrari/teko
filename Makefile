# Teko — atalhos. Rode `make` (ou `make help`) para ver tudo.
.DEFAULT_GOAL := help
.PHONY: help setup emulator suite reset demo issuer-help preview fmt lint test check build clean-dev verify-fw release-check repro

CARGO ?= cargo
PIN   ?= 482913

help: ## Mostra esta ajuda
	@echo "Teko — comandos:"; echo
	@grep -E '^[a-zA-Z_-]+:.*## ' $(MAKEFILE_LIST) | awk 'BEGIN {FS = ":.*## "} {printf "  \033[1m%-14s\033[0m %s\n", $$1, $$2}'
	@echo; echo "Primeira vez:  make setup   →   make emulator (terminal 1)   →   make suite (terminal 2)"

setup: ## 1x: cria chaves de teste, firmware v1/v2 e material de fábrica (pasta dev/)
	$(CARGO) run -q -p issuer-cli -- dev-setup

emulator: ## Liga o dispositivo emulado (PIN e confirmações são digitados NESTE terminal)
	$(CARGO) run -q -p device-emulator

suite: ## Abre a Teko Suite (janela desktop)
	$(CARGO) run -q -p teko-suite

reset: ## Volta o dispositivo a "virgem" (apaga o estado do emulador)
	$(CARGO) run -q -p device-emulator -- --reset

demo: ## Emulador com botões AUTOMÁTICOS (só teste/demo, não protege nada). PIN=482913
	$(CARGO) run -q -p device-emulator --features test-buttons -- --auto-pin $(PIN)

issuer-help: ## Ajuda da autoridade simulada (issuer-cli)
	$(CARGO) run -q -p issuer-cli -- --help

preview: ## Pré-visualiza a UI no navegador com backend falso (http://127.0.0.1:8765/?mock=virgin)
	python3 apps/suite/dev/preview.py

fmt: ## Formata o código
	$(CARGO) fmt --all

lint: ## Clippy sem avisos (-D warnings)
	$(CARGO) clippy --workspace --all-targets -- -D warnings

test: ## Roda todos os testes (unit, proptest e e2e)
	$(CARGO) test --workspace

check: ## Tudo que deve passar antes de commitar: formato + clippy + testes
	$(CARGO) fmt --all --check
	$(MAKE) lint
	$(MAKE) test

verify-fw: ## Verifica firmware v1 e v2 de forma independente da Suite (M-de-N, hash, downgrade)
	$(CARGO) run -q -p issuer-cli -- verify-firmware --manifest dev/firmware-v1.manifest --blob dev/firmware-v1.blob
	$(CARGO) run -q -p issuer-cli -- verify-firmware --manifest dev/firmware-v2.manifest --blob dev/firmware-v2.blob

release-check: ## Confirma que o build de release NÃO contém os botões automáticos (test-buttons)
	./scripts/release-check.sh

repro: ## Confere se o build é reproduzível (compila 2x em pastas diferentes e compara hashes)
	./scripts/repro-build.sh

build: ## Compila tudo (debug)
	$(CARGO) build --workspace

clean-dev: ## Apaga a pasta dev/ (chaves de teste e estado). Depois rode `make setup`
	@printf "Apagar dev/ (chaves de teste e estado do dispositivo)? [s/N] "; read a; \
	if [ "$$a" = "s" ] || [ "$$a" = "S" ]; then find dev -mindepth 1 ! -name .gitkeep -delete && echo "dev/ limpa."; else echo "cancelado."; fi
