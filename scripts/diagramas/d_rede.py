from lib import *

def camadas_crates():
    d = D("camadas-crates", 1100, 790, "Camadas dos crates", "Cada camada só depende das de baixo; detalhes no mapa de crates do README", "VISÃO GERAL")
    camadas = [("Binários e apps", [("device-emulator", "onca"), ("issuer-cli", "plain"), ("election-admin-cli", "plain"), ("teko-node", "boto"), ("teko-suite", "link")]),
               ("Rede e clientes", [("teko-chain", "boto"), ("teko-client", "link"), ("teko-transport", "plain")]),
               ("Livro-razão e consenso", [("teko-bft", "focal"), ("teko-ledger", "focal"), ("teko-coin", "focal"), ("teko-assets", "focal")]),
               ("Domínio", [("teko-election", "plain"), ("teko-device-core", "onca"), ("teko-registration", "plain"), ("teko-bulletin", "boto")]),
               ("Primitivas", [("teko-ballot-crypto", "plain"), ("teko-attestation", "plain")]),
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

def producao_bloco():
    d = D("producao-bloco", 1100, 0, "Produzir um bloco", "Rodízio de proponente; se ele falhar, a rodada avança", "REDE")
    A = [("s", "Suite", "cliente", "link", 110), ("p", "Proponente da vez", "round-robin", "focal", 380), ("b", "Validador B", "teko-node", "boto", 680), ("c", "Validador C", "teko-node", "boto", 960)]
    M = [("s", "p", "cédula (mempool)", "default", False), ("p", "b", "bloco proposto", "default", False),
         ("p", "c", "bloco proposto", "default", False), ("b", "p", "assinatura", "ok", True), ("c", "p", "assinatura", "ok", True),
         ("p", "b", "bloco com M assinaturas", "boto", False)]
    N = [(4, "p", "Com M-de-N assinaturas o bloco é válido", 300), (5, "c", "Proponente sumiu? round+1 e o próximo propõe", 340)]
    d.sequence(A, M, notas=N, passo=34)
    d.legend([(MUTED, "pedido", False), (GREEN, "assinatura", False), (BOTO, "bloco final", False), (MUTED, "retorno", True)])
    return d

def bloco_validacao():
    d = D("bloco-validacao", 1100, 560, "Validar um bloco", "Qualquer réplica refaz estas checagens; falhou uma, o bloco é rejeitado", "BLOCOS")
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

def sincronizacao():
    d = D("sincronizacao", 1000, 0, "Sincronizar sem confiar", "O cliente refaz a verificação de cada bloco: nó mentiroso não engana", "REDE")
    A = [("c", "Cliente", "NodeClient", "ok", 130), ("a", "Nó A", "honesto", "boto", 500), ("b", "Nó B", "adulterado", "bad", 870)]
    M = [("c", "a", "pede blocos desde a altura h", "default", False), ("a", "c", "blocos", "ok", True),
         ("c", "b", "pede os mesmos blocos", "default", False), ("b", "c", "blocos com assinatura falsa", "bad", True)]
    N = [(1, "c", "Chain::apply confere M-de-N: cadeia de A aceita", 330), (3, "c", "Mesma checagem: cadeia de B rejeitada", 300)]
    d.sequence(A, M, notas=N, passo=36)
    d.legend([(MUTED, "pedido", False), (GREEN, "aceito", True), (RED, "rejeitado", True)])
    return d

def bft_rodadas():
    d = D("bft-rodadas", 1100, 500, "Rodadas do consenso BFT", "Estilo Tendermint, com quórum ponderado por stake (> 2/3)", "CONSENSO")
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

def bft_certificado():
    d = D("bft-certificado", 1100, 480, "Certificado de commit e evidência", "Prova portátil de que a decisão foi tomada; e prova de quem traiu", "CONSENSO")
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

def livro_dominios():
    d = D("livro-dominios", 1100, 600, "Dois livros, os mesmos validadores", "Votos e economia (moeda e bens) prontos como biblioteca; a rede de validadores compartilhada vem no M6", "LIVRO-RAZÃO")
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

def venda_contra_pagamento():
    d = D("venda-contra-pagamento", 1100, 0, "Venda de bem contra pagamento", "Troca atômica: ou tudo acontece, ou nada (módulo de bens + moeda)", "BENS")
    A = [("v", "Vendedor", "dispositivo", "onca", 110), ("c", "Comprador", "dispositivo", "onca", 380), ("o", "Órgão", "M-de-N", "plain", 650), ("l", "Livro de economia", "Swap atômico", "focal", 940)]
    M = [("v", "l", "propõe: bem X por moeda Y", "default", False), ("c", "l", "aceita e assina", "default", False),
         ("o", "l", "aprova a transferência", "default", False), ("l", "v", "recebe a moeda", "ok", True), ("l", "c", "recebe o bem", "ok", True)]
    N = [(2, "l", "Atômico: tudo ou nada, com gravames checados", 320)]
    d.sequence(A, M, notas=N, passo=34)
    d.legend([(MUTED, "assinatura no dispositivo", False), (GREEN, "efeito", True)])
    return d

def suite_arquitetura():
    d = D("suite-arquitetura", 1100, 480, "Dentro da Teko Suite", "Interface fina, regras no Rust; o PIN nunca passa por aqui", "SUITE")
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
