# Teko — atalhos. Rode `make` (ou `make help`) para ver tudo.
.DEFAULT_GOAL := help
.PHONY: candidates claim runoff issuer devnet-cosign help setup emulator suite reset demo issuer-help preview fmt lint test check build clean-dev verify-fw release-check repro fund docs-check diagrams coin-demo assets-demo devnet close devnet-dkg devnet-economy explorer devnet-equivocate devnet-veto tally-time devnet-founder devnet-stop devnet-status devnet-reset devnet-blocks devnet-blocks-sim dev testnet-help election publish simulate simulate-live tally reset-board

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

issuer: ## Abre o Teko Emissor: app separado em que órgãos emitem credenciais na chain (precisa do emulador e de make devnet)
	$(CARGO) run -q -p teko-issuer

NET ?= dev/net
# Instituições (nós) da rede de TESTE, definido na criação da rede. Com 4 uma pode ficar fora do ar sem parar a
# rede; com 3 o quórum (mais de 2/3) exige os 3 (ADR-0034). Para mudar depois: make devnet-stop devnet-reset e `rm -r dev/net`.
NODES ?= 4
NODE = target/debug/teko-node

devnet: ## Sobe a rede de TESTE local com NODES=4 instituições (votos e economia em BFT; nenhum bloco nasce sozinho). Primeira vez: make devnet FOUNDER=tekow1… [ALLOC=1000] [ISSUER=tekow1…, membro dos órgãos de credenciais; padrão: FOUNDER] (sua carteira recebe as primeiras moedas, na gênese)
	$(CARGO) build -q -p teko-node
	@test -f $(NET)/network.txt || $(NODE) init --dir $(NET) --network devnet --nodes $(NODES) $(if $(FOUNDER),--founder $(FOUNDER)) $(if $(ISSUER),--issuer $(ISSUER)) $(if $(FOUNDER_TARGET),--founder-target $(FOUNDER_TARGET)) $(if $(ALLOC),--alloc $(ALLOC))
	@if [ "$(origin NODES)" = "command line" ]; then n=$$(ls -d $(NET)/node-* 2>/dev/null | wc -l | tr -d ' '); [ "$$n" = "$(NODES)" ] || echo "AVISO: NODES=$(NODES) foi ignorado: a rede em $(NET) já existe com $$n nós (a quantidade é definida só na criação). Para recriar com $(NODES): make devnet-stop && mv $(NET) $(NET).bak && make devnet FOUNDER=… NODES=$(NODES)"; fi
	@$(NODE) stop --dir $(NET) >/dev/null 2>&1; \
	for d in $(NET)/node-*; do i=$${d##*node-}; $(NODE) run --dir $(NET) --index $$i >$(NET)/node-$$i/node.log 2>&1 & done; \
	sleep 1; $(NODE) status --dir $(NET)
	@echo; echo "Próximo: make election (liga a eleição à rede) → reinicie o emulador → make publish → abra a Suite"

devnet-dkg: ## Cerimônia de chave da eleição entre os validadores (cédulas cifradas). Rode depois de make devnet
	@$(NODE) dkg --dir $(NET)

devnet-stop: ## Para os validadores (os blocos ficam em dev/net/node-*/votes.blocks e economy.blocks)
	@$(NODE) stop --dir $(NET) 2>/dev/null || true
	@# Rede apagada ou .pid perdido: mata nós órfãos pelo nome (senão eles seguem ocupando as portas)
	@pkill -f 'teko-node run --dir $(NET)' 2>/dev/null && echo 'nós órfãos parados' || true

devnet-founder: ## Define a carteira do fundador (recebe as recompensas) e zera a economia. Uso: make devnet-founder FOUNDER=tekow1…
	@test -n "$(FOUNDER)" || { echo 'Uso: make devnet-founder FOUNDER=tekow1… (endereço da aba Carteira da Suite)'; exit 1; }
	@$(NODE) stop --dir $(NET) >/dev/null 2>&1; $(NODE) founder --dir $(NET) --address $(FOUNDER) $(if $(ALLOC),--alloc $(ALLOC))

devnet-equivocate: ## SIMULAÇÃO (só dev): o validador de teste assina 2 votos conflitantes e leva Slash. Uso: make devnet-equivocate [VALIDATOR=0]. Antes, trave stake na Suite (Validadores)
	$(CARGO) run -q -p devnet-sim -- equivocate --dir $(NET) --validator $(or $(VALIDATOR),0)

devnet-cosign: ## SIMULA os membros de teste dos órgãos aprovando credenciais pendentes (TYPE=cnh-b opcional); só dev
	@$(CARGO) run -q -p devnet-sim -- cosign --dir $(NET) $(if $(TYPE),--type $(TYPE))

candidates: ## SIMULA o órgão emitindo as candidaturas de exemplo na chain (ELECTION=geral-2026); depois: make election (só dev)
	@$(CARGO) run -q -p devnet-sim -- seed-candidates --dir $(NET) $(if $(ELECTION),--election $(ELECTION))

claim: ## Leva à chain os resultados assinados pelas instituições: o eleito recebe a credencial vitalícia (ADR-0040)
	@$(CARGO) run -q -p election-admin-cli -- claim-elected

runoff: ## Abre o 2º turno de um pleito (CONTEST=governador-sp); depois: make publish
	@$(CARGO) run -q -p election-admin-cli -- runoff --contest $(CONTEST)

devnet-veto: ## SIMULAÇÃO (só dev): o órgão simulado veta uma negociação de bem pendente (janela de veto). Uso: make devnet-veto CLASS=imovel ASSET=<id>
	$(CARGO) run -q -p devnet-sim -- veto --dir $(NET) --class $(CLASS) --asset $(ASSET)

explorer: ## Explorador da chain (web + API JSON somente leitura) em http://127.0.0.1:8545; precisa da rede: make devnet
	$(CARGO) run -q -p explorer-server -- --dir $(NET)

devnet-economy: ## Economia BFT: altura, fase de fundador, validadores e verificação da cadeia
	@$(NODE) economy --dir $(NET)

devnet-status: ## Altura, entradas, mempool e pares de cada validador
	@$(NODE) status --dir $(NET)

devnet-blocks: ## Lista os blocos, verificados aqui (assinaturas, encadeamento e raiz Merkle)
	@$(NODE) blocks --dir $(NET)

KIND  ?= economy
COUNT ?= 5
devnet-blocks-sim: ## SIMULAÇÃO (só dev): faz a devnet produzir blocos (só existem com atividade). Uso: make devnet-blocks-sim [KIND=economy|votes] [COUNT=5]
	@$(CARGO) run -q -p devnet-sim -- blocks --dir $(NET) --kind $(KIND) --count $(COUNT)

dev: ## Menu de desenvolvimento (setas): sobe/derruba a devnet, simula blocos e roda os outros comandos (tools/teko-dev)
	@$(CARGO) run -q -p teko-dev

devnet-reset: ## Apaga a cadeia de teste (só os blocos; mantém as chaves). Pare antes: make devnet-stop
	@$(NODE) reset --dir $(NET)

testnet-help: ## Como montar uma testnet em várias máquinas (a mainnet está TRAVADA)
	@echo "Testnet: teko-node init --network testnet --dir <pasta>; distribua node-<i>/validator.key a cada instituição;"
	@echo "edite peers.txt com os endereços reais. Por padrão o nó só escuta em loopback (de propósito)."
	@echo "Mainnet: travada nesta fase (docs/NETWORKS.md)."

election: ## Recria a eleição de exemplo (7 pleitos, datas novas), com os candidatos das credenciais ativas da chain (ELECTION=geral-2026); sem rede, usa a lista de exemplo. Apaga o quadro e o marcador de simulação
	@$(CARGO) run -q -p election-admin-cli -- build-election --election $(or $(ELECTION),geral-2026) 2>/tmp/teko-election.err || { echo "(sem rede ou sem candidaturas na chain: lista de exemplo)"; $(CARGO) run -q -p issuer-cli -- dev-election; }

publish: ## Publica a eleição de exemplo no quadro de avisos (dev/board.log). Idempotente
	$(CARGO) run -q -p election-admin-cli -- publish

simulate: ## Anexa N eleitores SINTÉTICOS ao quadro de uma vez (DADOS SIMULADOS). N=5000
	$(CARGO) run -q --release -p election-admin-cli -- simulate --voters $(N)

simulate-live: ## Anexa ~RATE cédulas/s por SECS s: abra Resultados ao vivo na Suite e veja andar. RATE=15 SECS=120
	$(CARGO) run -q --release -p election-admin-cli -- simulate --rate $(RATE) --duration $(SECS)

close: ## TESTE: a governança simulada fecha TODOS os pleitos agora; com a rede e a cerimônia, os validadores decifram e os totais aparecem
	$(CARGO) run -q -p election-admin-cli -- close-due --now 99999999999

tally-time: ## Apura o quadro (release) e mostra o tempo de verificação e de contagem; TEKO_VERIFY_THREADS=1 força o sequencial
	TEKO_VERIFY_TRACE=1 $(CARGO) run -q --release -p election-admin-cli -- tally

tally: ## Apura o quadro e mostra o top 10 de cada pleito no terminal
	$(CARGO) run -q -p election-admin-cli -- tally

reset-board: ## Apaga o quadro (cédulas) e o marcador de simulação
	$(CARGO) run -q -p election-admin-cli -- reset-board

reset: ## Volta o dispositivo a "virgem" e esquece cidadãos/registros/revogações de TESTE
	$(CARGO) run -q -p device-emulator -- --reset
	$(CARGO) run -q -p issuer-cli -- dev-forget

demo: ## Emulador com botões AUTOMÁTICOS (só teste/demo, não protege nada). PIN=482913
	$(CARGO) run -q -p device-emulator --features test-buttons -- --auto-pin $(PIN)

fund: ## TESTE (devnet): aloca moedas de teste a uma carteira. Uso: make fund ADDR=tekow1… AMOUNT=1000   (em TEKO; aceita decimais: AMOUNT=0,5)
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

diagrams: ## Regenera os diagrams (docs/diagrams/*.png) a partir de scripts/diagrams (precisa do Brave)
	python3 scripts/diagrams/generate.py

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
