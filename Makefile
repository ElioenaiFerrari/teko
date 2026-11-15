# Teko — atalhos. Rode `make` (ou `make help`) para ver tudo.
.DEFAULT_GOAL := help
.PHONY: help setup emulator suite reset demo issuer-help preview fmt lint test check build clean-dev verify-fw release-check repro fund docs-check diagramas coin-demo assets-demo devnet close devnet-dkg devnet-economy devnet-founder devnet-stop devnet-status devnet-reset devnet-blocks testnet-help election publish simulate simulate-live tally reset-board

CARGO ?= cargo
PIN   ?= 482913
N     ?= 5000
RATE  ?= 15
SECS  ?= 120

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

NET ?= dev/net
NODE = target/debug/teko-node

devnet: ## Sobe a rede de TESTE local (votos PoA 2-de-3 + economia BFT). Primeira vez: make devnet FOUNDER=tekow1… (sua carteira)
	$(CARGO) build -q -p teko-node
	@test -f $(NET)/network.txt || $(NODE) init --dir $(NET) --network devnet $(if $(FOUNDER),--founder $(FOUNDER))
	@$(NODE) stop --dir $(NET) >/dev/null 2>&1; \
	for i in 0 1 2; do $(NODE) run --dir $(NET) --index $$i >$(NET)/node-$$i/node.log 2>&1 & done; \
	sleep 1; $(NODE) status --dir $(NET)
	@echo; echo "Próximo: make election (liga a eleição à rede) → reinicie o emulador → make publish → abra a Suite"

devnet-dkg: ## Cerimônia de chave da eleição entre os 3 validadores (cédulas cifradas). Rode depois de make devnet
	@$(NODE) dkg --dir $(NET)

devnet-stop: ## Para os 3 validadores (os blocos ficam em dev/net/node-*/blocks.log)
	@$(NODE) stop --dir $(NET) 2>/dev/null || true
	@# Rede apagada ou .pid perdido: mata nós órfãos pelo nome (senão eles seguem ocupando as portas)
	@pkill -f 'teko-node run --dir $(NET)' 2>/dev/null && echo 'nós órfãos parados' || true

devnet-founder: ## Define a carteira do fundador (recebe as recompensas) e zera a economia. Uso: make devnet-founder FOUNDER=tekow1…
	@test -n "$(FOUNDER)" || { echo 'Uso: make devnet-founder FOUNDER=tekow1… (endereço da aba Carteira da Suite)'; exit 1; }
	@$(NODE) stop --dir $(NET) >/dev/null 2>&1; $(NODE) founder --dir $(NET) --address $(FOUNDER)

devnet-economy: ## Economia BFT: altura, fase de fundador, validadores e verificação da cadeia
	@$(NODE) economy --dir $(NET)

devnet-status: ## Altura, entradas, mempool e pares de cada validador
	@$(NODE) status --dir $(NET)

devnet-blocks: ## Lista os blocos, verificados aqui (assinaturas, encadeamento e raiz Merkle)
	@$(NODE) blocks --dir $(NET)

devnet-reset: ## Apaga a cadeia de teste (só os blocos; mantém as chaves). Pare antes: make devnet-stop
	@$(NODE) reset --dir $(NET)

testnet-help: ## Como montar uma testnet em várias máquinas (a mainnet está TRAVADA)
	@echo "Testnet: teko-node init --network testnet --dir <pasta>; distribua node-<i>/validator.key a cada instituição;"
	@echo "edite peers.txt com os endereços reais. Por padrão o nó só escuta em loopback (de propósito)."
	@echo "Mainnet: travada nesta fase (docs/NETWORKS.md)."

election: ## Recria a eleição de exemplo (7 pleitos, datas novas). Apaga o quadro e o marcador de simulação
	$(CARGO) run -q -p issuer-cli -- dev-election

publish: ## Publica a eleição de exemplo no quadro de avisos (dev/board.log). Idempotente
	$(CARGO) run -q -p election-admin-cli -- publish

simulate: ## Anexa N eleitores SINTÉTICOS ao quadro de uma vez (DADOS SIMULADOS). N=5000
	$(CARGO) run -q --release -p election-admin-cli -- simulate --voters $(N)

simulate-live: ## Anexa ~RATE cédulas/s por SECS s: abra Resultados ao vivo na Suite e veja andar. RATE=15 SECS=120
	$(CARGO) run -q --release -p election-admin-cli -- simulate --rate $(RATE) --duration $(SECS)

close: ## TESTE: a governança simulada fecha TODOS os pleitos agora; com a rede e a cerimônia, os validadores decifram e os totais aparecem
	$(CARGO) run -q -p election-admin-cli -- close-due --now 99999999999

tally: ## Apura o quadro e mostra o top 10 de cada pleito no terminal
	$(CARGO) run -q -p election-admin-cli -- tally

reset-board: ## Apaga o quadro (cédulas) e o marcador de simulação
	$(CARGO) run -q -p election-admin-cli -- reset-board

reset: ## Volta o dispositivo a "virgem" e esquece cidadãos/registros/revogações de TESTE
	$(CARGO) run -q -p device-emulator -- --reset
	$(CARGO) run -q -p issuer-cli -- dev-forget

demo: ## Emulador com botões AUTOMÁTICOS (só teste/demo, não protege nada). PIN=482913
	$(CARGO) run -q -p device-emulator --features test-buttons -- --auto-pin $(PIN)

fund: ## TESTE (devnet): aloca moedas de teste a uma carteira. Uso: make fund ADDR=tekow1… AMOUNT=1000
	$(CARGO) run -q -p issuer-cli -- economy-fund --address $(ADDR) --amount $(or $(AMOUNT),1000)

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

check: ## Tudo que deve passar antes de commitar: formato + clippy + testes + docs
	$(CARGO) fmt --all --check
	$(MAKE) lint
	$(MAKE) test
	$(MAKE) docs-check

docs-check: ## Confere a documentação: links, imagens, texto alternativo, crates no README, títulos em pt-BR
	./scripts/docs-check.sh

coin-demo: ## Demonstração da moeda (teto, halving, taxa, stake, slash) rodando alguns blocos em memória
	$(CARGO) run -q -p teko-coin --example demo

assets-demo: ## Demonstração dos bens (registro com quórum do órgão, gravame, venda contra pagamento atômica)
	$(CARGO) run -q -p teko-assets --example demo

diagramas: ## Regenera os diagramas (docs/diagramas/*.svg|html|png) a partir de scripts/diagramas (precisa do Brave)
	python3 scripts/diagramas/gerar.py

verify-fw: ## Verifica firmware v1 e v2 de forma independente da Suite (M-de-N, hash, downgrade)
	$(CARGO) run -q -p issuer-cli -- verify-firmware --manifest dev/firmware-v1.manifest --blob dev/firmware-v1.blob
	$(CARGO) run -q -p issuer-cli -- verify-firmware --manifest dev/firmware-v2.manifest --blob dev/firmware-v2.blob

release-check: ## Confirma que o build de release NÃO contém os botões automáticos (test-buttons)
	./scripts/release-check.sh

repro: ## Confere se o build é reproduzível (compila 2x em pastas diferentes e compara hashes)
	./scripts/repro-build.sh

build: ## Compila tudo (debug)
	$(CARGO) build --workspace

clean-dev: devnet-stop ## Apaga a pasta dev/ (chaves de teste e estado). Para a rede antes. Depois rode `make setup`
	@printf "Apagar dev/ (chaves de teste e estado do dispositivo)? [s/N] "; read a; \
	if [ "$$a" = "s" ] || [ "$$a" = "S" ]; then find dev -mindepth 1 ! -name .gitkeep -delete && echo "dev/ limpa."; else echo "cancelado."; fi
