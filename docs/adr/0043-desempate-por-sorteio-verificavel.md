# ADR-0043 — Desempate por sorteio verificável (VRF de limiar sobre a chave da cerimônia)

**Status:** Aceito provisoriamente (2026-11-16, dono do produto) e **implementado** · Fecha o pendente "desempate legal" dos [ADR-0040](0040-candidatos-por-credencial-resultado-certificado-e-eleito.md) e [ADR-0042](0042-voto-em-branco-e-nulo.md); usa a cerimônia de chave do [ADR-0019](0019-cedulas-cifradas-elgamal-de-limiar.md).

## Contexto
Um empate que a regra não desfaz virava `Tie` ("sem eleito"): o resultado era certificado, mas `ClaimElected` o recusava e nada acontecia. O dono do produto escolheu **sorteio verificável, que sempre decide** (sem regra de idade, sem nova votação). O sorteio não pode ser controlado por ninguém sozinho, nem por quem ordena o quadro, e qualquer um tem de poder refazê-lo.

## Decisão
1. **Quando há sorteio** (só entre candidatos com voto, `valid > 0`): (a) empate **no topo** de um pleito sem 2º turno ou do próprio 2º turno: o sorteio **elege**; (b) num pleito com `runoff` e **só dois candidatos** sem maioria (empate exato): o sorteio **elege** (um 2º turno entre os mesmos dois não mudaria nada); (c) empate **pela vaga de finalista** (o 2º lugar empatado com o 3º, ou três empatados disputando duas vagas): o sorteio preenche as vagas abertas, e quem está à frente do grupo empatado mantém o lugar. **Se ninguém votou em candidato (`valid == 0`) continua "sem eleito"**: não há empate entre candidatos com voto, e sortear alguém sem nenhum seria absurdo.
2. **VRF de limiar sobre a chave da cerimônia (DKG).** A entrada `H` é um ponto da curva derivado (tentativa e incremento, RFC 9381 TAI) só do pleito e do empate: eleição, pleito, `contest_hash`, as opções empatadas, quantas o sorteio escolhe, para quê e os votos do empate. Cada instituição publica no quadro (`EntryKind::DRAW = 8`) a sua **parte** `d_i · H` com prova de Chaum-Pedersen contra a sua chave de verificação (o mesmo mecanismo da decifração dos totais). Com `t` partes válidas, `P = d · H` é **único, sejam quais forem as `t` partes**: ninguém conhece `d`, ninguém escolhe o subconjunto nem enviesa a saída. A **semente** é `H(P, entrada)`; ela embaralha (Fisher-Yates por fluxo de hash) só as opções empatadas.
3. **O resultado carrega a evidência:** `ElectionOutcome` ganhou `drawn` (17), `draw_tied` (18, as opções empatadas na ordem sorteada) e `draw_seed` (19). Todas as instituições chegam aos **mesmos bytes**, então "mais de 2/3 assinam o mesmo resultado" segue valendo, e a economia (`ClaimElected`) **não mudou**: um resultado sorteado é `Elected` ou `Runoff` comum.
4. **Verificação:** o apurador de cada nó confere cada parte (só vale para o pleito encerrado, revelado e realmente empatado, para **este** empate, com prova válida, uma por validador); `election-admin outcomes` **refaz o sorteio do quadro** (`verify_draw`) e exige os mesmos bytes assinados.
5. **Só dev:** `make tie CONTEST=… VOTES=5,5` planta votos exatos para forçar empates (`simulate-exact`).

## Consequências e limites honestos
- Exige **eleição cifrada** (a chave do sorteio é a do DKG); numa eleição em claro do protótipo o empate segue sem eleito.
- Mesma confiança da decifração: **`t` instituições juntas** sabem a semente antes dos outros (e poderiam decifrar cédulas); menos de `t` não sabem nem enviesam. Com menos de `t` instituições vivas o empate fica pendente.
- Quem controlasse votos poderia escolher *quais* empates existem, nunca *quem ganha* o sorteio.
- A economia continua acreditando em > 2/3 das instituições; a conferência independente é refazer o sorteio do quadro.
- O sorteio não tem valor legal; a regra de desempate legal (idade) foi descartada pelo dono do produto.
