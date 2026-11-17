# ADR-0046 — Cargos proporcionais (deputados e vereadores)

**Status:** Aceito provisoriamente (2026-11-16, dono do produto) e **implementado** · Fecha o pendente "proporcional" dos [ADR-0040](0040-candidatos-por-credencial-resultado-certificado-e-eleito.md) e [ADR-0044](0044-cargos-de-varias-vagas-senador.md); usa o sorteio do [ADR-0043](0043-desempate-por-sorteio-verificavel.md) e eleva o limite de opções do [ADR-0019](0019-cedulas-cifradas-elgamal-de-limiar.md).

## Contexto
Só os cargos majoritários geravam resultado. Deputado federal, deputado estadual e vereador dividem as cadeiras entre **partidos** e os candidatos ocupam as vagas do partido.

## Decisões do dono
- **Voto:** lista aberta, **um** voto por eleitor: em um **candidato** (conta para ele e para o partido) ou em uma **legenda** (só o partido); branco e nulo seguem como opções explícitas, fora dos válidos (ADR-0042).
- **Cadeiras:** quociente eleitoral (válidos ÷ cadeiras; fração acima de 0,5 sobe, até 0,5 é desprezada) e sobras por **D'Hondt**; dentro do partido ficam os mais votados.
- **Barreiras (pontos-base do quociente, configuráveis, 0 = sem):** candidato precisa de **10%** para ocupar vaga do quociente partidário; para as sobras, partido **80%** e candidato **20%**.
- **Vaga que ninguém apto ocupa:** **3ª rodada** por D'Hondt entre todos os partidos com candidato disponível, sem barreira.
- **Empates:** sorteio verificável (ADR-0043).
- **Limite:** **128 opções** por pleito; na Suite se vota **digitando o número** (2 dígitos = legenda, 4 ou 5 = candidato).

## Decisão técnica
1. **Manifesto (versão 6).** `OptionKind::Party` (nome do partido, número de 2 dígitos, sem credencial); `ElectionOption.party` no candidato (obrigatório e coerente num pleito proporcional, proibido fora dele; o número do candidato começa pelo do partido); `Contest.proportional {seats, first_candidate_bp, party_bp, candidate_bp}`, só para os três cargos, sem 2º turno, com ao menos 2 partidos. `Contest.seats` continua 0: a **cédula é um voto só** (a criptografia não muda: uma opção, soma 1). `MAX_OPTIONS` 64/16 → **128** (cédula de ~25 KB).
2. **Regra pura** (`teko-election::proportional`): quociente → cadeiras do quociente (candidatos com ≥ 10%) → sobras D'Hondt com barreiras → 3ª rodada sem barreira. Só candidato **com voto** ocupa cadeira (a câmara pode ficar incompleta). Se nenhum partido atinge o quociente, os candidatos mais votados ocupam as vagas. Cálculo em inteiros (`u128`).
3. **Empate.** Compara-se o resultado com a ordem de desempate normal e com a inversa; se o conjunto de eleitos muda, há empate que importa e o **sorteio do pleito inteiro** (`DrawGoal::Ranking`, sobre todos os candidatos e legendas) dá a ordem global de desempate. Um sorteio só, a mesma entrada do quadro do ADR-0043. Limite: um empate que mude só a *ordem* dos eleitos, e não o conjunto, não dispara sorteio (a ordem de chegada é a do voto).
4. **Resultado.** `ElectionOutcome`: `seats` = cadeiras, `elected`/`elected_credentials` = a câmara inteira, `quotient`, `parties [{option, votes, seats, number}]` e as barreiras usadas. Os majoritários não preenchem esses campos: bytes inalterados.
5. **Partido e candidatura.** O partido não é uma credencial à parte (simplificação em relação ao plano): a candidatura (`candidato`) ganha `partido` e `legenda`, obrigatórios nos cargos proporcionais e proibidos nos demais; a montagem da eleição recusa um partido com dois nomes. Cargos novos: `deputado-federal` (4 dígitos), `deputado-estadual` e `vereador` (5).
6. **Economia.** `ClaimElected` aceita a câmara (até 100 cadeiras, sem repetido, cada candidatura do cargo, da jurisdição e do partido do resultado; as cadeiras de cada partido conferem); uma `eleito` por vencedor, com `partido`; a câmara seguinte do mesmo cargo e jurisdição sucede a anterior inteira; teto do resultado 4 → 16 KiB.
7. **Dispositivo e Suite.** `ContestInfo` ganha `option_parties` e `chamber_seats`; a tela do dispositivo mostra "LEGENDA 11 - Nome (conta só para o partido)" ou "11011 - Nome (partido 11)". A Suite tem o campo de número, o filtro por partido e a tabela de votos por partido nos resultados; o Emissor, os cargos e as colunas de partido.

## Limites honestos
- 128 opções por pleito: uma câmara real tem milhares de candidatos, e um texto cifrado por opção não escala até lá (exigiria outra codificação da cédula).
- Sem federações, coligações, suplentes nem candidatura avulsa; só os eleitos recebem `eleito`.
- Empate decidido por sorteio, não pelo mais idoso (diferente da lei).
- O empate que só reordena os eleitos não é sorteado (ponto 3).
- Cadeira sem candidato com voto fica vaga.
