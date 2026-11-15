from lib import *


def concepts_map():
    d = D("concepts-map", 1100, 330, "Como os conceitos se encaixam", "Cada camada usa as de baixo; o guia de conceitos explica uma por uma", "CONCEITOS")
    xs = [25, 245, 465, 685, 905]
    nomes = [("Primitivas", "hash · assinatura", "plain", "cap. 1-2"), ("Identificadores", "bech32m · HRP · carteira", "link", "cap. 3-4"),
             ("Confiança", "M-de-N · consenso", "focal", "cap. 5-6"), ("Livro e moeda", "blocos · halving · stake", "boto", "cap. 7-8"),
             ("Voto secreto", "ElGamal · DKG · cega", "onca", "cap. 9-10")]
    for i, (x, (n, s, k, t)) in enumerate(zip(xs, nomes)):
        d.node(x, 140, 170, 76, n, s, k, t)
        if i < 4:
            d.arrow([(x + 170, 178), (xs[i + 1], 178)], "default")
    d.text(25, 262, "Setas: quem depende de quem. Tudo apoia-se em hash e assinatura; só o voto usa criptografia de soma cifrada.", 11, MUTED)
    d.legend([(MUTED, "dependência", False)])
    return d


def tagged_hash():
    d = D("tagged-hash", 1100, 420, "Hash com tag (separação de domínio)", "A mesma mensagem, com tags diferentes, nunca produz o mesmo hash", "CRIPTOGRAFIA")
    d.arrow([(240, 178), (300, 178)], "default"); d.arrow([(520, 178), (580, 178)], "default"); d.arrow([(800, 178), (860, 178)], "ok")
    d.node(20, 140, 220, 76, "Tag + partes", "\"teko/v1/doc-sign\" · msg", "input")
    d.node(300, 140, 220, 76, "Prefixo de tamanho", "len(tag) tag len(p) p…", "plain", "sem ambiguidade")
    d.node(580, 140, 220, 76, "SHA3-256", "função de hash", "focal")
    d.node(860, 140, 220, 76, "32 bytes", "impressão digital", "ok")
    d.node(300, 280, 500, 70, "Mesma mensagem, OUTRA tag → outro hash", "uma assinatura de um uso não vale em outro", "bad", "por que usar tag")
    d.legend([(MUTED, "fluxo", False), (GREEN, "resultado", False)])
    return d


def ecdsa_signature():
    d = D("ecdsa-signature", 1100, 510, "Assinar e verificar (ECDSA P-256)", "A chave privada nasce no dispositivo; qualquer um verifica só com a pública", "CRIPTOGRAFIA")
    d.arrow([(240, 178), (310, 178)], "default"); d.arrow([(520, 178), (590, 178)], "onca"); d.arrow([(800, 178), (870, 178)], "default")
    d.arrow([(980, 216), (980, 300)], "default", label="enviada", at=(1015, 258))
    d.arrow([(800, 338), (870, 338)], "link", label="confere", at=(835, 324))
    d.node(20, 140, 220, 76, "Mensagem", "o que será assinado", "input")
    d.node(310, 140, 210, 76, "Hash com tag", "digest de 32 bytes", "plain")
    d.node(590, 140, 210, 76, "Assina", "chave PRIVADA (secreta)", "onca", "dispositivo")
    d.node(870, 140, 210, 76, "Assinatura", "64 bytes (r, s)", "focal")
    d.node(590, 300, 210, 76, "Chave PÚBLICA", "33 bytes (SEC1)", "link")
    d.node(870, 300, 210, 76, "Verifica", "válida ou inválida", "ok", "qualquer um")
    d.text(20, 420, "A pública só verifica; não permite assinar. Trocar um bit da mensagem invalida a assinatura.", 11, MUTED)
    d.legend([(MUTED, "fluxo", False), (ONCA, "segredo", False), (LINK, "público", False)])
    return d


def bech32m_anatomy():
    d = D("bech32m-anatomy", 1100, 480, "Anatomia de um endereço bech32m", "tekow1 + dados + checksum: o HRP diz para que serve", "ENDEREÇOS")
    d.node(30, 140, 220, 76, "HRP", "\"teko\" · \"tekow\" · \"tekop\"", "focal", "prefixo legível")
    d.node(270, 140, 90, 76, "\"1\"", "separador", "plain")
    d.node(380, 140, 450, 76, "Dados em base32", "20 bytes derivados do hash da chave pública", "link", "conteúdo")
    d.node(850, 140, 220, 76, "Checksum", "6 caracteres (BCH)", "ok", "detecta erro")
    d.text(30, 262, "teko → ID Cívico (identidade) · tekow → carteira (moeda) · tekop → ID pairwise (um por serviço).", 11, MUTED)
    d.text(30, 286, "Colar um endereço tekow onde se espera teko é recusado: o HRP não bate, mesmo com checksum correto.", 11, MUTED)
    d.node(30, 330, 520, 60, "Errou uma letra ao digitar?", "o checksum não fecha e o endereço é recusado", "bad")
    d.legend([(MUTED, "partes do endereço", False)])
    return d


def m_of_n():
    d = D("m-of-n", 1100, 480, "M-de-N: limiar de assinaturas", "Nenhuma instituição sozinha decide; M de N precisam concordar", "CONFIANÇA")
    d.arrow([(240, 158), (300, 158), (300, 208), (400, 208)], "default")
    d.arrow([(240, 238), (400, 238)], "default")
    d.arrow([(240, 318), (300, 318), (300, 258), (400, 258)], "default")
    d.arrow([(640, 220), (700, 220), (700, 160), (780, 160)], "ok", label="≥ M", at=(740, 146))
    d.arrow([(640, 256), (700, 256), (700, 316), (780, 316)], "bad", dashed=True, label="< M", at=(740, 302))
    d.node(40, 130, 200, 56, "Instituição A", "assina (ou não)", "boto")
    d.node(40, 210, 200, 56, "Instituição B", "assina (ou não)", "boto")
    d.node(40, 290, 200, 56, "Instituição C", "assina (ou não)", "boto")
    d.node(400, 190, 240, 76, "Verificador", "conta assinaturas válidas distintas", "focal")
    d.node(780, 130, 230, 60, "Aceita", "firmware · bloco · bem", "ok")
    d.node(780, 286, 230, 60, "Recusa", "sem caminho com chave única", "bad")
    d.text(40, 392, "Exemplo 2-de-3: duas instituições bastam, e uma sozinha é inútil. Chaves repetidas contam uma vez só.", 11, MUTED)
    d.legend([(MUTED, "assinaturas", False), (GREEN, "M ou mais", False), (RED, "menos de M", True)])
    return d


def two_thirds_quorum():
    d = D("two-thirds-quorum", 1100, 470, "Por que o quórum é MAIS de 2/3", "Dois quóruns se sobrepõem em mais de 1/3, e esse trecho inclui alguém honesto", "CONSENSO")
    d.arrow([(260, 168), (295, 168), (295, 212), (330, 212)], "default")
    d.arrow([(260, 288), (295, 288), (295, 246), (330, 246)], "default")
    d.arrow([(560, 228), (620, 228)], "default")
    d.arrow([(830, 228), (880, 228)], "ok")
    d.node(40, 130, 220, 76, "Quórum do bloco X", "> 2/3 do poder votou X", "plain")
    d.node(40, 250, 220, 76, "Quórum do bloco Y", "> 2/3 do poder votou Y", "plain")
    d.node(330, 190, 230, 76, "Interseção > 1/3", "votaram X e Y", "focal")
    d.node(620, 190, 210, 76, "Só traidores", "honesto vota 1x por rodada", "bad")
    d.node(880, 190, 190, 76, "X e Y não decidem", "com < 1/3 de traidores", "ok", "segurança")
    d.text(40, 380, "No código: poder × 3 > total × 2 (is_quorum). A rede tolera até 1/3 do poder falho ou malicioso.", 11, MUTED)
    d.legend([(MUTED, "raciocínio", False), (GREEN, "conclusão", False)])
    return d


def poa_vs_bft():
    d = D("poa-vs-bft", 1100, 570, "Dois consensos no Teko", "PoA (votos, hoje) e BFT estilo Tendermint (economia)", "CONSENSO")
    d.region(30, 110, 480, 340, "PoA M-de-N · livro de votos", "optional")
    d.region(590, 110, 480, 340, "BFT por stake · livro de economia", "optional")
    for x, nomes, kinds in ((60, [("Lista fixa de validadores", "definida na rede (N chaves)", "plain"), ("Rodízio de proponente", "altura mod N", "plain"), ("M-de-N assinaturas", "bloco válido com M de N", "focal")], None),
                            (620, [("Validadores por stake", "poder ∝ moedas travadas", "plain"), ("Proposta · prevote · precommit", "rodadas com timeouts", "plain"), ("Certificado > 2/3 do poder", "commit final e verificável", "ok")], None)):
        for i, (n, s, k) in enumerate(nomes):
            y = 145 + i * 100
            d.node(x, y, 420, 64, n, s, k)
            if i < 2:
                d.arrow([(x + 210, y + 64), (x + 210, y + 100)], "default")
    d.text(30, 486, "PoA: simples e rápido, mas a lista de validadores é fixa. BFT: o conjunto muda com o stake e tolera < 1/3 falho.", 11, MUTED)
    d.legend([(MUTED, "etapas", False)])
    return d


def halving_curve():
    d = D("halving-curve", 1100, 520, "Halving: a recompensa cai pela metade", "Teko: 50 TEKO por bloco, halving a cada 210.000 blocos, teto de 21.000.000 TEKO", "MOEDA")
    base, esc_ = 410, 3.8
    vals = [50, 25, 12.5, 6.25, 3.125]
    acum = ["10,5 M", "15,75 M", "18,375 M", "19,69 M", "20,34 M"]
    for i, v in enumerate(vals):
        x = 80 + i * 190
        h = v * esc_
        d.under.append(f'<rect x="{x}" y="{base-h:.0f}" width="120" height="{h:.0f}" rx="4" fill="{GOLD_TINT if i else GOLD}" stroke="{GOLD}" stroke-width="1"/>')
        d.text(x + 60, base - h - 8, f"{str(v).replace('.', ',')} TEKO", 12, INK, "middle", weight=600)
        d.text(x + 60, base + 20, f"época {i}", 11, INK, "middle", weight=600)
        d.text(x + 60, base + 36, f"blocos {i*210000:,}".replace(",", "."), 9, MUTED, "middle", mono=True)
        d.text(x + 60, base + 52, f"acumulado {acum[i]}", 9, MUTED, "middle", mono=True)
    d.under.append(f'<line x1="60" y1="{base}" x2="1040" y2="{base}" stroke="{MUTED}" stroke-width="1.2"/>')
    d.text(60, 130, "No código: recompensa = initial_reward >> época (deslocamento de bits divide por 2).", 11, MUTED)
    d.text(60, 150, "A soma de todas as épocas converge para o teto: 50 × 210.000 × 2 = 21.000.000.", 11, MUTED)
    d.legend([(GOLD, "recompensa por bloco", False)])
    return d


def elgamal_sum():
    d = D("elgamal-sum", 1100, 490, "Somar votos sem abrir nenhum", "ElGamal exponencial: soma cifrada, decifração por limiar", "VOTAÇÃO")
    d.arrow([(240, 158), (300, 158), (300, 208), (340, 208)], "default")
    d.arrow([(240, 238), (340, 238)], "default")
    d.arrow([(240, 318), (300, 318), (300, 258), (340, 258)], "default")
    d.arrow([(560, 228), (640, 228)], "default", label="M partes", at=(600, 214))
    d.arrow([(840, 228), (900, 228)], "ok")
    d.node(40, 130, 200, 56, "Cédula cifrada A", "voto 1 (escondido)", "onca")
    d.node(40, 210, 200, 56, "Cédula cifrada B", "voto 0 (escondido)", "onca")
    d.node(40, 290, 200, 56, "Cédula cifrada C", "voto 1 (escondido)", "onca")
    d.node(340, 190, 220, 76, "Soma das cifras", "multiplica os pontos", "focal", "sem decifrar")
    d.node(640, 190, 200, 76, "≥ M guardiões", "cada um dá uma parte", "boto", "limiar")
    d.node(900, 190, 170, 76, "Total: 2", "só a soma aparece", "ok")
    d.text(40, 392, "Ninguém vê o voto de ninguém: nem os guardiões, que decifram apenas o total. Provas 0/1 impedem cédula trapaceira.", 11, MUTED)
    d.legend([(MUTED, "fluxo", False), (GREEN, "resultado", False)])
    return d
