from lib import *

SEQ_LEG = [(MUTED, "pedido", False), (GREEN, "resposta aceita", False), (MUTED, "retorno", True), (RED, "recusa", False)]

def onboarding():
    d = D("onboarding", 1000, 0, "Configurar o dispositivo (onboarding)", "Autenticidade, firmware, PIN, carteira, ID Cívico e credencial — nesta ordem", "IDENTIDADE")
    A = [("s", "Suite", "host não confiável", "link", 140), ("d", "Dispositivo", "tela e botões", "onca", 500), ("e", "Emissor", "autoridade", "external", 860)]
    M = [("s", "d", "AttestationChallenge", "default", False), ("d", "s", "cadeia de atestação", "ok", True),
         ("s", "d", "FirmwareUpdate", "default", False), ("d", "s", "FirmwareStatus", "ok", True),
         ("s", "d", "SetupPin", "default", False), ("d", "s", "PinSet", "ok", True),
         ("s", "d", "SetupWallet", "default", False), ("d", "s", "WalletReady{endereço}", "ok", True),
         ("s", "d", "GetCivicId", "default", False), ("d", "s", "CivicId", "ok", True),
         ("s", "e", "pedido de credencial", "link", False), ("e", "s", "credencial assinada", "link", True),
         ("s", "d", "StoreCredential", "default", False)]
    N = [(0, "s", "Suite confere a cadeia M-de-N", 220), (2, "d", "Tela: instalar vX? hash ab12… (botão)", 260),
         (4, "d", "PIN digitado 2x NO dispositivo", 230),
         (6, "d", "24 palavras SÓ na tela; você confirma posições", 280), (8, "d", "Tela mostra o ID completo para conferir", 260)]
    d.sequence(A, M, notas=N, passo=34)
    d.legend(SEQ_LEG)
    return d

def estados_dispositivo():
    d = D("estados-dispositivo", 1000, 520, "Estados do dispositivo", "Cada mensagem só é aceita em certos estados — o resto responde InvalidState", "IDENTIDADE")
    xs = [40, 360, 680]
    d.arrow([(260, 200), (360, 200)], "default", label="firmware M-de-N", at=(310, 186))
    d.arrow([(580, 200), (680, 200)], "default", label="SetupPin", at=(630, 186))
    d.arrow([(790, 238), (790, 300)], "default", label="StoreCredential", at=(790, 269)) if False else d.arrow([(790, 238), (790, 300)], "default", label="credencial", at=(850, 269))
    d.arrow([(680, 338), (580, 338)], "default", label="bloqueia", at=(630, 324))
    d.arrow([(360, 338), (260, 338)], "ok", label="Unlock", at=(310, 324))
    d.arrow([(470, 376), (470, 420), (20, 420), (20, 200), (40, 200)], "bad")
    d.label("PIN errado N vezes: apaga tudo e volta a Virgem", (245, 406), RED)
    d.node(40, 162, 220, 76, "Virgem", "sem firmware", "input", "estado")
    d.node(360, 162, 220, 76, "Firmware instalado", "aceita só SetupPin", "plain", "estado")
    d.node(680, 162, 220, 76, "Inicializado", "sem carteira, sem ID Cívico", "focal", "estado")
    d.node(680, 300, 220, 76, "Credenciais emitidas", "ID Cívico + credencial", "onca", "estado")
    d.node(360, 300, 220, 76, "Bloqueado", "espera o PIN", "security", "estado")
    d.node(40, 300, 220, 76, "Desbloqueado", "assina com confirmação", "ok", "estado")
    d.legend([(MUTED, "transição normal", False), (GREEN, "transição de uso", False), (RED, "falha de PIN", False)])
    return d

def hierarquia_chaves():
    d = D("hierarquia-chaves", 1100, 560, "Hierarquia de chaves", "Nada aqui é derivável da identidade para o voto, nem o contrário", "IDENTIDADE")
    d.arrow([(550, 176), (550, 214), (210, 214), (210, 262)], "default")
    d.arrow([(550, 176), (550, 214), (550, 262)], "default")
    d.arrow([(550, 176), (550, 214), (890, 214), (890, 262)], "default")
    d.arrow([(210, 338), (210, 400)], "default")
    d.arrow([(890, 338), (890, 400)], "boto")
    d.node(400, 100, 300, 76, "Semente do dispositivo", "nasce no dispositivo · nunca sai", "focal", "raiz")
    d.node(100, 262, 220, 76, "Chave de identidade", "ID Cívico (bech32m)", "onca", "identidade")
    d.node(440, 262, 220, 76, "Chaves pairwise", "uma por serviço", "plain", "identidade")
    d.node(780, 262, 220, 76, "Chave de eleitor", "uma por pleito", "boto", "voto", sub2="sem ligação com a identidade")
    d.node(100, 400, 220, 76, "Assinatura de documento", "SignDocument", "plain", "uso")
    d.node(780, 400, 220, 76, "Cédula cifrada + assinatura", "SignBallot", "plain", "uso")
    d.text(440, 440, "Muro de separação:", 11, INK, weight=600)
    d.text(440, 458, "o voto usa outra ramificação;", 11, MUTED)
    d.text(440, 474, "ninguém liga ID Cívico a cédula.", 11, MUTED)
    d.legend([(MUTED, "deriva", False), (BOTO, "ramo do voto", False)])
    return d

def assinatura_documento():
    d = D("assinatura-documento", 1000, 0, "Assinar um documento", "O que é assinado nasce dentro do dispositivo, a partir do que a tela mostra", "ASSINATURA")
    A = [("u", "Cidadão", "olha a tela", "input", 120), ("s", "Suite", "host não confiável", "link", 400), ("d", "Dispositivo", "tela e botões", "onca", 700), ("v", "Verificador", "terceiro", "ok", 900)]
    M = [("s", "d", "SignDocument{hash,resumo}", "default", False), ("d", "u", "mostra resumo + hash", "accent", False),
         ("u", "d", "botão: confirmar", "accent", False), ("d", "s", "DocumentSignature", "ok", True),
         ("s", "v", "documento + assinatura", "link", False)]
    N = [(0, "d", "Resumo vem do computador: marcado 'informado pelo host'", 340),
         (3, "v", "Confere a assinatura com a chave pública", 260)]
    d.sequence(A, M, notas=N, passo=34)
    d.legend(SEQ_LEG)
    return d

def fronteiras_confianca():
    d = D("fronteiras-confianca", 1000, 520, "O que o computador nunca vê", "Camadas de confiança: quanto mais por dentro, mais protegido", "ASSINATURA")
    d.region(40, 110, 920, 340, "Mundo externo (não confiável): rede, sistema operacional, Suite", "optional")
    d.region(80, 160, 620, 270, "Dispositivo", "onca")
    d.region(120, 210, 340, 190, "Compartimento seguro", "security")
    d.node(140, 250, 150, 64, "Semente e chaves", "nunca saem", "focal")
    d.node(310, 250, 130, 64, "PIN", "só no dispositivo", "bad")
    d.node(140, 330, 150, 52, "Tela e botões", "confirmação", "ok")
    d.node(510, 250, 170, 64, "Assinaturas e provas", "dados públicos", "plain")
    d.arrow([(680, 282), (740, 282)], "ok")
    d.node(740, 250, 190, 64, "Teko Suite", "só recebe o público", "link")
    d.legend([(MUTED, "só dados públicos cruzam a fronteira", False)])
    return d

def reemissao_revogacao():
    d = D("reemissao-revogacao", 1000, 0, "Reemissão presencial com revogação", "Perdeu o dispositivo? Mesma identidade, sem frase-semente", "CREDENCIAIS")
    A = [("c", "Cidadão", "no balcão", "input", 120), ("e", "Emissor", "issuer-cli", "external", 420), ("n", "Lista de revogação", "assinada", "security", 720), ("d", "Novo dispositivo", "ID Cívico novo", "onca", 920)]
    M = [("c", "e", "comparece com documento", "default", False), ("e", "n", "revoga o ID Cívico antigo", "bad", False),
         ("n", "e", "RevocationList assinada", "ok", True), ("e", "d", "credencial nova (mesmo citizen_ref)", "link", False)]
    N = [(0, "e", "Primeira emissão para quem já existe: recusada", 280), (3, "n", "Dispositivos e verificadores consultam a lista", 300)]
    d.sequence(A, M, notas=N, passo=36)
    d.legend(SEQ_LEG)
    return d

def registro_cego():
    d = D("registro-cego", 1000, 0, "Registro anônimo de eleitor", "Assinatura cega RFC 9474: a autoridade assina sem ver a chave de eleitor", "REGISTRO")
    A = [("d", "Dispositivo", "chave de eleitor", "onca", 130), ("a", "Autoridade", "chave RSA por pleito", "external", 450), ("q", "Quadro", "público", "boto", 820)]
    M = [("d", "a", "BeginRegistration: valor cego + credencial", "default", False), ("a", "d", "assinatura cega", "link", True),
         ("d", "d", "desfaz o cegamento", "default", False), ("d", "q", "publica (chave de eleitor, token)", "boto", False)]
    N = [(0, "a", "Checa elegibilidade e 1 registro por cidadão", 300), (3, "q", "Token válido sem identidade", 220)]
    d.sequence(A, M, notas=N, passo=38)
    d.legend(SEQ_LEG)
    return d


def carteira_criar():
    d = D("carteira-criar", 1000, 0, "Criar a carteira e confirmar a frase", "As 24 palavras só existem na tela do dispositivo e no seu papel", "CARTEIRA")
    A = [("s", "Suite", "só espera", "link", 120), ("d", "Dispositivo", "gera e mostra", "onca", 480), ("u", "Você", "papel e caneta", "input", 860)]
    M = [("s", "d", "SetupWallet", "default", False), ("d", "u", "mostra 24 palavras (6 por página)", "accent", False),
         ("u", "d", "anota em papel e confirma", "accent", False), ("d", "u", "pede 3 posições sorteadas", "default", False),
         ("u", "d", "digita as palavras pedidas", "accent", False), ("d", "s", "WalletReady{endereço}", "ok", True)]
    N = [(0, "d", "Gera a frase com o RNG do próprio dispositivo", 320),
         (4, "d", "Errou? Mostra a frase de novo. Acertou: salva", 330),
         (5, "s", "A Suite só recebe o endereço público", 260)]
    d.sequence(A, M, notas=N, passo=36)
    d.legend([(MUTED, "pedido", False), (GOLD, "você e o dispositivo (tela e botões)", False), (GREEN, "resposta pública", True)])
    return d

def carteira_arvore():
    d = D("carteira-arvore", 1100, 560, "De onde vem a chave da carteira", "A frase não deriva da identidade nem do voto, e vice-versa", "CARTEIRA")
    d.region(24, 100, 1052, 170, "Carteira: recuperável pela frase de 24 palavras", "onca")
    d.arrow([(240, 188), (300, 188)], "default"); d.arrow([(520, 188), (580, 188)], "default")
    d.arrow([(800, 188), (860, 188)], "default")
    d.node(40, 150, 200, 76, "Entropia própria", "32 bytes do dispositivo", "focal", "nasce aqui")
    d.node(300, 150, 220, 76, "Frase BIP-39", "24 palavras (+ passphrase)", "onca", "só na tela")
    d.node(580, 150, 220, 76, "SLIP-10 P-256", "m/7240'/0'/0'/0'/i'", "plain", "teko-wallet")
    d.node(860, 150, 196, 76, "Endereço tekow1…", "bech32m, público", "ok", "público")
    d.line_h = None
    d.region(24, 330, 1052, 170, "Identidade e voto: SEM seed (reemissão presencial)", "optional")
    d.node(40, 380, 220, 76, "Raiz de identidade", "K_root, nunca sai", "plain", "identidade")
    d.node(320, 380, 220, 76, "Segredo de eleitor", "s, entropia separada", "boto", "voto")
    d.text(600, 410, "Nenhuma das duas raízes deriva da frase,", 11, INK, weight=600)
    d.text(600, 428, "nem a frase deriva delas: vender ou perder", 11, MUTED)
    d.text(600, 446, "a frase não revela quem você é nem seu voto.", 11, MUTED)
    d.arrow([(550, 270), (550, 330)], "bad", dashed=True)
    d.label("sem nenhuma derivação", (550, 300), RED)
    d.legend([(MUTED, "deriva", False), (RED, "nunca se liga", True)])
    return d


def carteira_enviar():
    d = D("carteira-enviar", 1000, 0, "Enviar moedas com o dispositivo", "O dispositivo monta, mostra e assina a transação; a Suite só prepara e entrega", "CARTEIRA")
    A = [("s", "Suite", "prepara os campos", "link", 120), ("d", "Dispositivo", "mostra e assina", "onca", 480), ("l", "Livro de economia", "devnet local", "boto", 860)]
    M = [("s", "l", "lê nonce e saldo; calcula a taxa", "default", False), ("l", "s", "nonce, saldo, taxa", "default", True),
         ("s", "d", "SignTransfer{destino, valor, taxa, nonce}", "default", False), ("d", "d", "mostra destino, valor, taxa e rede", "accent", False),
         ("d", "s", "TransferSigned{assinatura}", "ok", True), ("s", "l", "transação assinada", "boto", False)]
    N = [(3, "d", "Você confere e confirma no botão. Recusou: nada é assinado", 380),
         (5, "l", "Valida assinatura, nonce, taxa e saldo; senão recusa", 320)]
    d.sequence(A, M, notas=N, passo=36)
    d.legend([(MUTED, "pedido", False), (GOLD, "tela e botão do dispositivo", False), (BOTO, "gravação no livro", False), (GREEN, "resposta", True)])
    return d
