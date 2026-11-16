# ADR-0041 — O eleitor confere o próprio voto: desafio de Benaloh e conferência no dispositivo

**Status:** Aceito provisoriamente (2026-11-16, dono do produto) e **implementado (etapa C)** · Substitui o item "a aleatoriedade é apagada, sem como conferir" do [ADR-0019](0019-cedulas-cifradas-elgamal-de-limiar.md); complementa [ADR-0034](0034-votos-em-bft-institucional-e-sem-blocos-vazios.md).

## Contexto
O dispositivo cifrava o voto, assinava e apagava o aleatório; a Suite mostrava "Voto em X", um recibo do computador sem valor criptográfico, e conferia inclusão contra uma cabeça **sem assinatura**. O dono do produto quer "não precisa confiar, você pode conferir", **sem** criar um recibo que sirva para coação ou compra de voto.

## Decisão
Três garantias, cada uma conferida por quem pode conferi-la:

1. **Cifrou o que escolhi (desafio de Benaloh, *cast-or-audit*).** O dispositivo cifra e **se compromete** com a cédula (`BallotCommitment` chega ao computador) **antes** de o eleitor decidir. Só então, **no dispositivo**, o eleitor escolhe: **votar** (assina a mesma cédula) ou **auditar** (`BallotAudited`: opção e aleatório daquela cédula, que é **descartada**, nunca assinada). O computador abre a cédula (`open_ballot`) e exige que contenha exatamente a opção pedida; senão o dispositivo mentiu. Como o dispositivo não sabe qual cédula será auditada, um dispositivo desonesto é pego com probabilidade ½ por auditoria. Auditar não consome o voto.
2. **Está registrada e é final.** Ao votar o dispositivo guarda só `VoteRecord{contest_hash, leaf_hash}` (a folha Merkle da entrada do quadro: **pública**, a cifra já está lá). `CheckMyBallot` leva ao dispositivo um bloco da chain de votos, o certificado de commit e a prova Merkle; **ele confere sozinho**: certificado com **mais de 2/3** das instituições do `trust.txt` (`vote_chain_id`, `vote_validator=`), `block_id` do bloco, rede e altura, e a inclusão da **sua** folha no `log_root`. Mostra bloco, "k de N" e o **rastreador** (`TK-XXXX-XXXX`, base32 da folha) só na tela; responde ao computador apenas dados públicos.
3. **Entrou na soma.** A Suite refaz a apuração do quadro verificado e mostra o **veredito da própria cédula** (`BallotVerdict`: contada ou recusada e por quê). O dispositivo não refaz a soma.

**Contra coação e compra de voto: sem recibo.** Depois de votar não resta no dispositivo nem a opção nem o aleatório (teste inspeciona o armazenamento), e a Suite **deixou de mostrar "Voto em X"**: só o rastreador. Quem tiver o aparelho e o PIN não descobre em quem se votou.

Para o dispositivo verificar sem `std`: `merkle` foi para `teko-crypto` (reexportado por `teko-bulletin`), `teko-bft` é `no_std`+`alloc`, e o bloco é lido por uma visão mínima do cabeçalho (o `block_id` cobre o payload inteiro).

## Consequências e limites honestos
- Garantia **estatística**: audite o bastante. Pressupõe dispositivo e firmware (assinado M-de-N) honestos na hora de votar.
- **Não dá para "ver depois em quem votei"**: é o preço de não existir recibo.
- A contagem é conferida pela Suite (refazendo a apuração), não pelo dispositivo.
- As instituições do `trust.txt` são estáticas (rotação é PENDENTE, como no ADR-0040).
- Coação sob observação direta e o limiar de instituições decifrando uma cédula específica seguem como no ADR-0019.
