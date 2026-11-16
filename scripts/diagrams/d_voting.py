from lib import *
SEQ_LEG = [(MUTED, "pedido", False), (GREEN, "resposta aceita", False), (BOTO, "gravação", False), (MUTED, "retorno", True)]

def encrypted_ballot():
    d = D("encrypted-ballot", 1100, 0, "Votar: a cédula cifrada", "A cédula é montada, cifrada e assinada dentro do dispositivo", "VOTAÇÃO")
    A = [("d", "Dispositivo", "chave de eleitor", "onca", 120), ("s", "Suite", "só repassa", "link", 400), ("v", "Validadores", "teko-node", "focal", 690), ("q", "Quadro", "bloco M-de-N", "boto", 970)]
    M = [("s", "d", "SignBallot{pleito, opção}", "default", False), ("d", "s", "cédula cifrada + provas + assinatura", "ok", True),
         ("s", "v", "envia a cédula", "default", False), ("v", "q", "cédula válida entra no bloco", "boto", False)]
    N = [(0, "d", "Tela mostra o nome da opção; botão confirma", 300), (2, "v", "Confere provas 0/1 e soma=1, token e voto único", 330)]
    d.sequence(A, M, notas=N, passo=36)
    d.legend(SEQ_LEG)
    return d

def ballot_anatomy():
    d = D("ballot-anatomy", 1100, 470, "Anatomia da cédula", "O que viaja na rede: nada que revele a opção nem a identidade", "VOTAÇÃO")
    d.region(40, 110, 1020, 290, "Cédula (≈2,4 kB para 12 opções)", "boto")
    d.node(70, 150, 300, 90, "Chave de eleitor (pública)", "uma por pleito", "boto", "voto")
    d.node(400, 150, 300, 90, "Token do registro", "assinatura cega RFC 9474", "plain", "registro")
    d.node(730, 150, 300, 90, "Assinatura do eleitor", "feita no dispositivo", "ok", "autoria")
    d.node(70, 270, 300, 90, "Uma cifra por opção", "ElGamal exponencial P-256", "focal", "sigilo")
    d.node(400, 270, 300, 90, "Prova 0/1 por opção", "Chaum-Pedersen disjuntiva", "plain", "validade")
    d.node(730, 270, 300, 90, "Prova soma = 1", "exatamente uma opção", "plain", "validade")
    d.legend([(BOTO, "tudo isto é público no quadro", False)])
    return d

def election_lifecycle():
    d = D("election-lifecycle", 1100, 520, "Ciclo de vida do pleito", "Cada transição exige M-de-N órgãos, ou o prazo, e fica no histórico", "ELEIÇÃO")
    d.arrow([(240, 178), (300, 178)], "default")
    d.arrow([(500, 178), (560, 178)], "default")
    d.arrow([(760, 178), (820, 178)], "default")
    d.arrow([(920, 216), (920, 300)], "default", label="prazo ou M-de-N", at=(1000, 258))
    d.arrow([(820, 338), (760, 338)], "default")
    d.arrow([(560, 338), (500, 338)], "default")
    d.arrow([(140, 216), (140, 300)], "bad", dashed=True, label="cancelar", at=(190, 258))
    d.node(40, 140, 200, 76, "Rascunho", "Draft", "input", "fase 0")
    d.node(300, 140, 200, 76, "Candidaturas publicadas", "CandidatesPublished", "plain", "fase 1")
    d.node(560, 140, 200, 76, "Registro aberto", "RegistrationOpen", "plain", "fase 2")
    d.node(820, 140, 200, 76, "Votação aberta", "VotingOpen", "focal", "fase 3")
    d.node(820, 300, 200, 76, "Votação encerrada", "VotingClosed", "plain", "fase 4")
    d.node(560, 300, 200, 76, "Apurada", "Tallied", "ok", "fase 5")
    d.node(300, 300, 200, 76, "Auditada", "Audited", "ok", "fase 6")
    d.node(40, 300, 200, 76, "Cancelada", "Cancelled", "bad", "fase 7")
    d.text(40, 430, "Regra de sigilo: nenhum resultado aparece enquanto a votação está aberta; após o prazo, o dispositivo recusa votos.", 11, MUTED)
    d.legend([(MUTED, "avanço", False), (RED, "cancelamento", True)])
    return d

def tally():
    d = D("tally", 1100, 600, "Apuração e revelação", "Só o total é decifrado, e só depois do fechamento verificado", "APURAÇÃO")
    d.arrow([(260, 178), (300, 178)], "default"); d.arrow([(520, 178), (560, 178)], "default")
    d.arrow([(760, 178), (820, 178)], "ok", label="sim", at=(790, 164))
    d.arrow([(660, 216), (660, 300)], "bad", label="não", at=(690, 258))
    d.arrow([(920, 216), (920, 300)], "default")
    d.arrow([(920, 376), (920, 460)], "boto", label="limiar M-de-N", at=(1000, 418))
    d.arrow([(820, 498), (760, 498)], "ok")
    d.node(40, 140, 220, 76, "Quadro de votos", "log Merkle append-only", "boto")
    d.node(300, 140, 220, 76, "Apurador verifica", "Tallier: pura função do log", "plain")
    d.node(560, 140, 200, 76, "Pleito encerrado?", "fase VotingClosed", "focal")
    d.node(820, 140, 200, 76, "Soma homomórfica", "por opção", "plain")
    d.node(560, 300, 200, 76, "Nada é revelado", "cédulas seguem cifradas", "bad")
    d.node(820, 300, 200, 76, "Decifração por limiar", "M-de-N com prova CP", "boto")
    d.node(820, 460, 200, 76, "Totais verificáveis", "EntryKind::RESULT", "ok")
    d.node(560, 460, 200, 76, "Suite mostra gráficos", "por cargo e estado", "link")
    d.legend([(MUTED, "fluxo", False), (GREEN, "caminho feliz", False), (RED, "bloqueio", False), (BOTO, "decifração", False)])
    return d

def election_to_elected():
    d = D("election-to-elected", 1100, 0, "Da candidatura ao eleito", "Candidatos vêm de credenciais; as instituições certificam o resultado; a economia emite a credencial vitalícia", "ELEIÇÕES")
    A = [("o", "Órgão", "candidaturas", "plain", 100), ("e", "Chain de economia", "credenciais", "focal", 380), ("v", "Chain de votos", "instituições (BFT)", "boto", 690), ("x", "Qualquer um", "envia o resultado", "link", 970)]
    M = [("o", "e", "candidato: credencial pública (cargo, número, nome)", "default", False),
         ("e", "v", "make election: opções ligadas às credenciais ativas", "link", False),
         ("v", "v", "voto cifrado, fechamento, decifração por limiar", "accent", False),
         ("v", "v", "cada instituição assina o MESMO resultado", "accent", False),
         ("x", "e", "ClaimElected: resultado + assinaturas (> 2/3)", "default", False),
         ("e", "e", "eleito: credencial vitalícia; o anterior é sucedido", "ok", True),
         ("v", "o", "sem maioria: 2º turno entre os dois mais votados", "bad", False)]
    N = [(1, "e", "Pleito assinado pela governança: não muda depois", 330), (4, "e", "A economia confere as chaves das instituições (gênese)", 330)]
    d.sequence(A, M, notas=N, passo=34)
    d.legend([(MUTED, "transação ou entrada", False), (GREEN, "efeito", True), (RED, "2º turno", False)])
    return d

def dkg_ceremony():
    d = D("dkg-ceremony", 1100, 0, "Cerimônia de chave (DKG)", "A chave de decifração nasce dividida: ninguém a possui inteira", "CERIMÔNIA")
    A = [("a", "Validador A", "teko-node", "boto", 130), ("b", "Validador B", "teko-node", "boto", 400), ("c", "Validador C", "teko-node", "boto", 670), ("q", "Quadro", "público", "plain", 950)]
    M = [("a", "q", "DkgRecord: compromissos + prova", "boto", False), ("b", "q", "DkgRecord: compromissos + prova", "boto", False),
         ("c", "q", "DkgRecord: compromissos + prova", "boto", False), ("a", "b", "parte secreta (loopback)", "default", False),
         ("b", "c", "parte secreta (loopback)", "default", False)]
    N = [(4, "c", "Cada um confere as partes contra os compromissos de Feldman", 400), (4, "q", "Chave pública do pleito = soma das contribuições", 340)]
    d.sequence(A, M, notas=N, passo=36)
    d.legend([(BOTO, "publicação no quadro", False), (MUTED, "canal entre nós", False)])
    return d


def board_verification():
    d = D("board-verification", 1100, 520, "Verificação do quadro: em paralelo e em passos", "As provas são conferidas em várias threads; a contagem segue a ordem do quadro", "VOTAÇÃO")
    d.arrow([(230, 178), (300, 178)], "default", label="lotes", at=(265, 164))
    d.arrow([(520, 178), (590, 178)], "ok", label="vereditos", at=(555, 164))
    d.arrow([(810, 178), (880, 178)], "default", label="em ordem", at=(845, 164))
    d.arrow([(990, 216), (990, 300)], "default", label="lê", at=(1020, 258))
    d.arrow([(880, 338), (810, 338)], "link", label="mostra", at=(845, 324))
    d.arrow([(410, 300), (410, 216)], "onca", dashed=True, label="dispara", at=(445, 258))
    d.node(20, 140, 210, 76, "Quadro", "6.640 entradas (nós)", "boto", "verificado em blocos")
    d.node(300, 140, 220, 76, "Lote de 256", "assinaturas, provas, tokens", "plain", "puro")
    d.node(590, 140, 220, 76, "N threads", "confere em paralelo", "focal", "21,8 s → 3,7 s")
    d.node(880, 140, 200, 76, "Contagem", "uma thread, em ordem", "ok", "38 ms")
    d.node(880, 300, 200, 76, "Estado único", "compartilhado e em passos", "link", "Resultados + Exploração")
    d.node(300, 300, 220, 76, "Aquecimento", "ao abrir a Suite", "onca", "em segundo plano")
    d.node(590, 300, 220, 76, "Barra de progresso", "3.200 de 6.640 (48%)", "plain", "na tela")
    d.text(20, 420, "A ordem do quadro continua mandando: a primeira cédula do eleitor vale e as repetidas são rejeitadas. O veredito de uma cédula só é usado se for do mesmo conteúdo", 11, MUTED)
    d.text(20, 440, "e da mesma chave da eleição em vigor; senão a conferência é refeita. O resultado é idêntico ao da conferência sequencial (testado).", 11, MUTED)
    d.legend([(MUTED, "fluxo", False), (GREEN, "vereditos", False), (LINK, "estado", False)])
    return d
