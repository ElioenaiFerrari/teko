from lib import *

def crate_layers():
    d = D("crate-layers", 1100, 790, "Camadas dos crates", "Cada camada só depende das de baixo; detalhes no mapa de crates do README", "VISÃO GERAL")
    camadas = [("Binários e apps", [("device-emulator", "onca"), ("issuer-cli", "plain"), ("election-admin-cli", "plain"), ("teko-node", "boto"), ("teko-suite", "link")]),
               ("Rede e clientes", [("teko-chain", "boto"), ("teko-client", "link"), ("teko-transport", "plain")]),
               ("Livro-razão e consenso", [("teko-bft", "focal"), ("teko-ledger", "focal"), ("teko-coin", "focal"), ("teko-assets", "focal")]),
               ("Domínio", [("teko-election", "plain"), ("teko-device-core", "onca"), ("teko-registration", "plain"), ("teko-bulletin", "boto")]),
               ("Primitivas", [("teko-ballot-crypto", "plain"), ("teko-attestation", "plain"), ("teko-wallet", "onca")]),
               ("Fundação", [("teko-crypto", "focal"), ("teko-protocol", "plain")])]
    y = 100
    for nome, chips in camadas:
        d.region(40, y, 1020, 90, nome)
        x = 64
        for c, k in chips:
            d.node(x, y + 28, 176, 46, c, None, k)
            x += 192
        y += 104
    d.text(40, 624, "", 1)
    d.legend([(BOTO, "livro e rede", False), (ONCA, "dispositivo", False), (LINK, "Suite", False), (GOLD, "base de confiança", False)])
    return d

def block_production():
    d = D("block-production", 1100, 0, "Produzir um bloco", "Rodízio de proponente; se ele falhar, a rodada avança", "REDE")
    A = [("s", "Suite", "cliente", "link", 110), ("p", "Proponente da vez", "round-robin", "focal", 380), ("b", "Validador B", "teko-node", "boto", 680), ("c", "Validador C", "teko-node", "boto", 960)]
    M = [("s", "p", "cédula (mempool)", "default", False), ("p", "b", "bloco proposto", "default", False),
         ("p", "c", "bloco proposto", "default", False), ("b", "p", "assinatura", "ok", True), ("c", "p", "assinatura", "ok", True),
         ("p", "b", "bloco com M assinaturas", "boto", False)]
    N = [(4, "p", "Com M-de-N assinaturas o bloco é válido", 300), (5, "c", "Proponente sumiu? round+1 e o próximo propõe", 340)]
    d.sequence(A, M, notas=N, passo=34)
    d.legend([(MUTED, "pedido", False), (GREEN, "assinatura", False), (BOTO, "bloco final", False), (MUTED, "retorno", True)])
    return d

def block_validation():
    d = D("block-validation", 1100, 560, "Validar um bloco", "Qualquer réplica refaz estas checagens; falhou uma, o bloco é rejeitado", "BLOCOS")
    for (x1, x2) in [(260, 300), (520, 560), (780, 820)]:
        d.arrow([(x1, 178), (x2, 178)], "default")
    d.arrow([(920, 216), (920, 300)], "default")
    for (x1, x2) in [(820, 780), (560, 520), (300, 260)]:
        d.arrow([(x1, 338), (x2, 338)], "default")
    d.node(40, 140, 220, 76, "Bloco recebido", "Block", "input")
    d.node(300, 140, 220, 76, "1. Cadeia e altura", "chain_id · height · prev_hash", "plain")
    d.node(560, 140, 220, 76, "2. Proponente", "da vez, conforme a rodada", "plain")
    d.node(820, 140, 200, 76, "3. Assinaturas", "M-de-N distintas e válidas", "focal")
    d.node(820, 300, 200, 76, "4. Entradas", "regras de cada tipo", "plain")
    d.node(560, 300, 220, 76, "5. Merkle", "log_size e log_root", "plain")
    d.node(300, 300, 220, 76, "6. Aplica", "Chain::apply", "ok")
    d.node(40, 300, 220, 76, "Altura avança", "estado novo", "ok")
    d.node(40, 430, 980, 44, "Qualquer falha = bloco rejeitado com erro tipado; nada é aplicado pela metade", None, "bad")
    d.legend([(MUTED, "sequência das checagens", False)])
    return d

def sync():
    d = D("sync", 1000, 0, "Sincronizar sem confiar", "O cliente refaz a verificação de cada bloco: nó mentiroso não engana", "REDE")
    A = [("c", "Cliente", "NodeClient", "ok", 130), ("a", "Nó A", "honesto", "boto", 500), ("b", "Nó B", "adulterado", "bad", 870)]
    M = [("c", "a", "pede blocos desde a altura h", "default", False), ("a", "c", "blocos", "ok", True),
         ("c", "b", "pede os mesmos blocos", "default", False), ("b", "c", "blocos com assinatura falsa", "bad", True)]
    N = [(1, "c", "Chain::apply confere M-de-N: cadeia de A aceita", 330), (3, "c", "Mesma checagem: cadeia de B rejeitada", 300)]
    d.sequence(A, M, notas=N, passo=36)
    d.legend([(MUTED, "pedido", False), (GREEN, "aceito", True), (RED, "rejeitado", True)])
    return d

def bft_rounds():
    d = D("bft-rounds", 1100, 500, "Rodadas do consenso BFT", "Estilo Tendermint, com quórum ponderado por stake (> 2/3)", "CONSENSO")
    d.arrow([(220, 178), (310, 178)], "default", label="válido", at=(265, 164))
    d.arrow([(490, 178), (580, 178)], "default", label="> 2/3", at=(535, 164))
    d.arrow([(760, 178), (850, 178)], "ok", label="> 2/3", at=(805, 164))
    d.arrow([(400, 216), (400, 300)], "bad", dashed=True, label="timeout", at=(445, 258))
    d.arrow([(670, 216), (670, 300)], "bad", dashed=True, label="timeout", at=(715, 258))
    d.arrow([(340, 338), (130, 338), (130, 216)], "default", label="rodada r+1", at=(235, 324))
    d.node(40, 140, 180, 76, "Proposta", "propõe o bloco", "plain", "etapa 1")
    d.node(310, 140, 180, 76, "Prevote", "voto de intenção", "plain", "etapa 2")
    d.node(580, 140, 180, 76, "Precommit", "trava no bloco", "focal", "etapa 3")
    d.node(850, 140, 180, 76, "Commit", "certificado", "ok", "final")
    d.node(340, 300, 360, 76, "Nova rodada", "novo proponente · timeout maior", "bad", "recuperação")
    d.text(40, 410, "Quórum > 2/3 do stake em cada etapa (prevote e precommit).", 11, MUTED)
    d.text(40, 430, "Votos conflitantes do mesmo validador não travam a rede e viram evidência.", 11, MUTED)
    d.legend([(MUTED, "caminho normal", False), (GREEN, "decide", False), (RED, "falha de prazo", True)])
    return d

def bft_certificate():
    d = D("bft-certificate", 1100, 480, "Certificado de commit e evidência", "Prova portátil de que a decisão foi tomada; e prova de quem traiu", "CONSENSO")
    d.arrow([(260, 178), (340, 178)], "default"); d.arrow([(560, 178), (640, 178)], "ok")
    d.arrow([(260, 338), (340, 338)], "bad"); d.arrow([(560, 338), (640, 338)], "bad", dashed=True)
    d.node(40, 140, 220, 76, "Precommits assinados", "> 2/3 do stake", "plain")
    d.node(340, 140, 220, 76, "Certificado de commit", "Commit::verify", "focal")
    d.node(640, 140, 220, 76, "Qualquer verificador", "nó novo ou cliente", "ok")
    d.node(40, 300, 220, 76, "Dois votos conflitantes", "mesmo validador, mesma rodada", "bad")
    d.node(340, 300, 220, 76, "Evidência de equivocação", "Evidence", "plain")
    d.node(640, 300, 220, 76, "Penalidade (slash)", "prevista na moeda", "optional", "planejado")
    d.legend([(MUTED, "fluxo", False), (GREEN, "verificação", False), (RED, "traição", False), (RED, "efeito planejado", True)])
    return d

def ledger_domains():
    d = D("ledger-domains", 1100, 600, "Dois livros, os mesmos validadores", "Votos e economia (moeda e bens) prontos como biblioteca; a rede de validadores compartilhada vem no M6", "LIVRO-RAZÃO")
    d.arrow([(500, 196), (500, 230), (410, 230), (410, 290)], "boto")
    d.arrow([(600, 196), (600, 230), (670, 230), (670, 290)], "default")
    d.arrow([(240, 328), (300, 328)], "link")
    d.arrow([(860, 328), (780, 328)], "ok")
    d.arrow([(620, 366), (620, 450)], "default")
    d.arrow([(740, 366), (740, 408), (950, 408), (950, 450)], "default")
    d.node(430, 120, 240, 76, "Validadores", "stake + histórico", "focal", "compartilhados")
    d.node(300, 290, 220, 76, "Livro de votos", "implementado", "boto")
    d.node(560, 290, 220, 76, "Livro de economia", "moeda + bens", "ok", "biblioteca")
    d.node(40, 290, 200, 76, "Emissor", "elegibilidade", "external")
    d.node(860, 290, 200, 76, "Órgão registrador", "quórum M-de-N", "ok", "implementado")
    d.node(560, 450, 220, 76, "Moeda", "teko-coin · teto fixo", "ok", "implementado")
    d.node(840, 450, 220, 76, "Bens", "teko-assets · gravames", "ok", "implementado")
    d.legend([(BOTO, "livro de votos", False), (MUTED, "livro de economia", False), (LINK, "elegibilidade", False), (GREEN, "aprovação do órgão", False)])
    return d

def delivery_vs_payment():
    d = D("delivery-vs-payment", 1100, 0, "Venda de bem contra pagamento", "Troca atômica: ou tudo acontece, ou nada (módulo de bens + moeda)", "BENS")
    A = [("v", "Vendedor", "dispositivo", "onca", 110), ("c", "Comprador", "dispositivo", "onca", 380), ("o", "Órgão", "M-de-N", "plain", 650), ("l", "Livro de economia", "Swap atômico", "focal", 940)]
    M = [("v", "l", "propõe: bem X por moeda Y", "default", False), ("c", "l", "aceita e assina", "default", False),
         ("o", "l", "aprova a transferência", "default", False), ("l", "v", "recebe a moeda", "ok", True), ("l", "c", "recebe o bem", "ok", True)]
    N = [(2, "l", "Atômico: tudo ou nada, com gravames checados", 320)]
    d.sequence(A, M, notas=N, passo=34)
    d.legend([(MUTED, "assinatura no dispositivo", False), (GREEN, "efeito", True)])
    return d

def suite_architecture():
    d = D("suite-architecture", 1100, 480, "Dentro da Teko Suite", "Interface fina, regras no Rust; o PIN nunca passa por aqui", "SUITE")
    d.arrow([(240, 178), (310, 178)], "default", label="IPC Tauri", at=(275, 164))
    d.arrow([(510, 178), (580, 178)], "default")
    d.arrow([(780, 178), (850, 178)], "default", label="TCP", at=(815, 164))
    d.arrow([(410, 216), (410, 300)], "bad", dashed=True)
    d.arrow([(680, 216), (680, 300)], "boto")
    d.node(40, 140, 200, 76, "Interface", "HTML · CSS · JS", "plain", "pt-BR")
    d.node(310, 140, 200, 76, "Comandos Tauri", "commands.rs", "link")
    d.node(580, 140, 200, 76, "Cliente", "teko-client", "plain")
    d.node(850, 140, 200, 76, "Dispositivo", "teko-transport", "onca")
    d.node(310, 300, 200, 76, "dev-simulation", "só em desenvolvimento", "security", "feature")
    d.node(580, 300, 200, 76, "Quadro e rede", "teko-bulletin · NodeClient", "boto")
    d.text(40, 430, "O build de produção não contém a autoridade nem o operador de teste (conferido por scripts/release-check.sh).", 11, MUTED)
    d.legend([(MUTED, "chamada", False), (RED, "só em desenvolvimento", True), (BOTO, "leitura do livro", False)])
    return d


def assets_suite():
    d = D("assets-suite", 1100, 560, "Bens na Suite (ambiente de teste)", "Seu consentimento é real, no dispositivo; órgão e contraparte são simulados", "BENS")
    d.arrow([(240, 178), (310, 178)], "default", label="negócio", at=(275, 164))
    d.arrow([(510, 178), (580, 178)], "ok", label="consentimento", at=(545, 164))
    d.arrow([(410, 216), (410, 300)], "bad", dashed=True, label="órgão e contraparte assinam", at=(520, 258))
    d.arrow([(510, 338), (580, 338)], "boto", label="troca atômica", at=(545, 324))
    d.node(40, 140, 200, 76, "Tela Bens", "venda, compra, gravame", "link", "Suite")
    d.node(310, 140, 200, 76, "Comandos", "asset_trade", "plain", "Tauri")
    d.node(580, 140, 200, 76, "Seu dispositivo", "mostra e assina", "onca", "consentimento")
    d.node(310, 300, 200, 76, "Órgão e contraparte", "simulados · chaves de teste", "security", "dev-simulation")
    d.node(580, 300, 200, 76, "Livro local", "teko-assets + teko-coin", "boto", "dev/economy.*")
    d.text(40, 430, "A troca é atômica: bem e moeda mudam de mãos juntos, ou nada acontece. O build de produção não tem órgão nem contraparte simulados.", 11, MUTED)
    d.legend([(MUTED, "chamada", False), (GREEN, "consentimento no dispositivo", False), (RED, "só em desenvolvimento", True), (BOTO, "gravação", False)])
    return d


def founder_phases():
    d = D("founder-phases", 1100, 560, "Da fase de fundador à validação por stake", "Opção B: sem pré-mineração; as recompensas viram o primeiro stake", "ECONOMIA")
    d.arrow([(310, 178), (400, 178)], "default", label="recompensas", at=(355, 164))
    d.arrow([(620, 178), (710, 178)], "ok", label="stake ≥ N", at=(665, 164))
    d.arrow([(875, 216), (875, 300)], "default", label="todos validam", at=(955, 258))
    d.node(60, 140, 250, 76, "Fundador valida sozinho", "único validador da gênese", "onca", "fase de fundador")
    d.node(400, 140, 220, 76, "Dono trava as moedas", "SignStake no dispositivo", "plain", "carteira do dono")
    d.node(710, 140, 330, 76, "Trava no estado", "founder_phase_over (irreversível)", "focal", "fim da fase")
    d.node(725, 300, 300, 76, "Validadores por stake", "poder ∝ stake, no máx. K", "ok", "conjunto por altura")
    d.text(60, 440, "As recompensas de bloco vão à CARTEIRA do dono (não à chave do nó). O conjunto da altura h vem do estado após h−1.", 11, MUTED)
    d.legend([(MUTED, "fluxo", False), (GREEN, "transição", False)])
    return d


def economy_block():
    d = D("economy-block", 1000, 0, "Um bloco de economia sob consenso BFT", "Proposta, votos ponderados e certificado de commit", "ECONOMIA")
    A = [("c", "Cliente", "assina no dispositivo", "link", 110), ("p", "Proponente", "validador da vez", "focal", 380), ("v", "Demais validadores", "e réplicas", "boto", 680), ("l", "Livro", "moeda + bens", "plain", 920)]
    M = [("c", "p", "transação assinada", "default", False), ("p", "v", "proposta: txs + raiz do estado", "default", False),
         ("v", "p", "prevote e precommit (> 2/3)", "ok", True), ("p", "v", "bloco + certificado de commit", "boto", False),
         ("v", "l", "reexecuta e confere a raiz", "default", False)]
    N = [(1, "v", "Cada um refaz a execução antes de votar", 300), (3, "l", "Recompensa e taxas vão ao DONO do proponente", 330)]
    d.sequence(A, M, notas=N, passo=36)
    d.legend([(MUTED, "mensagem", False), (GREEN, "votos", True), (BOTO, "decisão final", False)])
    return d


def explorer_architecture():
    d = D("explorer-architecture", 1100, 470, "Arquitetura do explorador", "A chain é baixada dos nós e verificada pelo próprio explorador; a API só lê", "EXPLORADOR")
    d.arrow([(230, 178), (310, 178)], "default", label="blocos", at=(270, 164))
    d.arrow([(530, 178), (610, 178)], "ok", label="verificados", at=(570, 164))
    d.arrow([(830, 178), (900, 178)], "link", label="JSON", at=(865, 164))
    d.arrow([(720, 216), (720, 300)], "default", label="GET", at=(750, 258))
    d.node(20, 140, 210, 76, "Nós da rede", "teko-node (3 validadores)", "boto", "devnet")
    d.node(310, 140, 220, 76, "Baixa blocos", "NodeClient · só os novos", "plain")
    d.node(610, 140, 220, 76, "Índice verificado", "teko-explorer · Index", "focal", "não confia no nó")
    d.node(900, 140, 180, 76, "API JSON", "GET /api/v1/…", "link", "somente leitura")
    d.node(610, 300, 220, 76, "Página web", "HTML · CSS · JS", "ok", "no navegador")
    d.node(20, 300, 510, 76, "Recusa tudo que não fecha", "certificado > 2/3, proponente, execução e raiz do estado", "bad", "o que é verificado")
    d.text(20, 420, "Um nó que mente (bloco falso ou sem certificado) é recusado: o selo da página avisa e o índice continua com a cadeia que já verificou.", 11, MUTED)
    d.legend([(MUTED, "fluxo", False), (GREEN, "verificado", False), (LINK, "público", False)])
    return d


def explorer_pages():
    d = D("explorer-pages", 1100, 500, "Páginas e rotas do explorador", "Cada página usa uma rota da API; tudo é leitura", "EXPLORADOR")
    d.node(30, 140, 220, 64, "Início", "GET /status · /blocks · /txs", "focal")
    d.node(30, 230, 220, 64, "Bloco", "GET /blocks/{altura ou hash}", "plain")
    d.node(30, 320, 220, 64, "Transação", "GET /txs/{id}", "plain")
    d.node(300, 140, 250, 64, "Carteira", "GET /accounts/{tekow1…}", "plain", "saldo · stake · bens")
    d.node(300, 230, 250, 64, "Validadores", "GET /validators", "plain", "fase de fundador")
    d.node(300, 320, 250, 64, "Bens", "GET /assets", "plain", "classes e gravames")
    d.node(600, 140, 230, 64, "Eventos", "GET /events?topic=", "plain")
    d.node(600, 230, 230, 64, "Votos (PoA)", "GET /votes", "boto", "só cabeçalhos")
    d.node(600, 320, 230, 64, "Busca", "GET /search?q=", "ok", "altura · hash · endereço")
    d.node(880, 140, 190, 244, "Valores", "em ypy (inteiro)", "store", "e em TEKO exato")
    d.text(30, 430, "Qualquer método que não seja GET recebe 405. Entrada inválida recebe 400 com a explicação; o que não existe recebe 404.", 11, MUTED)
    d.legend([(MUTED, "página → rota", False)])
    return d
