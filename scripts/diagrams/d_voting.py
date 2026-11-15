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
