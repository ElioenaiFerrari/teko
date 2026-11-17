# ADR-0044 — Cargos de várias vagas, etapa 1: o Senador

**Status:** Aceito provisoriamente (2026-11-16, dono do produto) e **implementado** · Fecha o pendente "várias vagas" do [ADR-0040](0040-candidatos-por-credencial-resultado-certificado-e-eleito.md) **só para o Senador**; usa o branco e nulo do [ADR-0042](0042-voto-em-branco-e-nulo.md) e o sorteio do [ADR-0043](0043-desempate-por-sorteio-verificavel.md).

## Contexto
O sistema só elegia **uma pessoa por pleito** (`decide()` devolvia um eleito ou 2º turno; a cédula provava "soma = 1"). O Senador tem **1 ou 2 vagas por estado** (alterna a cada eleição) e é majoritário: os N mais votados vencem, sem 2º turno. O dono do produto escolheu fazer **só o Senador agora** e dar ao eleitor **N votos em candidatos distintos**. O proporcional (deputados e vereadores) exige partidos e fica para outra rodada.

## Decisão
1. **Manifesto (formato 5).** `Contest.seats` (tag 10): **só o Senador** o define, **1 ou 2** explícitos; os outros cargos deixam 0 (uma vaga: mesmo encode e mesmo hash de antes). Regras: pelo menos **vagas + 1** candidatos; sem 2º turno; **2 vagas exigem eleição cifrada** (a prova nova e a decifração de limiar vivem aí). `Office::Senator` passa a ser majoritário (`is_majoritarian`); só Presidente, Governador e Prefeito têm 2º turno.
2. **A cédula.** O eleitor marca **1 a N candidatos distintos, ou só Branco, ou só Nulo** (nunca candidato com branco/nulo). Cada opção continua provada 0 ou 1 (por isso distintos); a cédula prova ainda que o **total ponderado** `T' = Σcandidatos + N·(branco + nulo)` **está em 1..=N** (OU de provas de Chaum-Pedersen, uma por valor, sem dizer qual). Candidato + branco dá `T' ≥ N+1` e é recusado. Com **uma vaga** os bytes são exatamente os de antes (prova de soma 1); com 2, a prova de faixa ocupa 128 bytes em vez de 64.
3. **Dispositivo.** `SignBallot.option_indices` (repetido; vazio = o `option_index` de antes) e `BallotAudited.option_positions`. A tela mostra **"SEUS VOTOS (k de N vagas)"** com cada candidato; o dispositivo valida a regra antes de cifrar; a auditoria de Benaloh (ADR-0041) abre a cédula e o cliente exige o **conjunto exato** que o eleitor pediu.
4. **Resultado.** Só candidatos **com voto** disputam; os **N mais votados** vencem. Se o corte empata (o N-ésimo com o seguinte), o **sorteio verificável** do ADR-0043 preenche as vagas abertas (`DrawGoal::Seats`); quem está à frente do grupo empatado mantém o lugar. **Menos de N candidatos com voto deixam vagas em aberto** (ninguém é sorteado entre zero votos); ninguém votado = sem eleito. `ElectionOutcome` ganhou `seats` (20), `elected` (21) e `elected_credentials` (22); os outros cargos mantêm os bytes de antes. Os totais decifrados só liberam se couberem nas cédulas contadas (`aceitas ≤ Σcandidatos + N·(branco+nulo) ≤ N·aceitas`).
5. **Economia.** `Cargo::Senador`. Um único `ClaimElected` emite **as N credenciais `eleito` de uma vez** (tudo ou nada). A sucessão tem uma lista por **classe**: o espaço é cargo + jurisdição + **número de vagas**, então a classe de 2 vagas (eleição de 2026) e a de 1 vaga (2030) não se substituem; a nova eleição de cada classe substitui todos os titulares da própria.
6. **Ferramentas de teste.** `make election` cria `senador-sp` (2 vagas) e `senador-rj` (1 vaga) quando a eleição é cifrada; `make candidates` emite candidaturas de senador; `make simulate` sorteia 1 ou 2 candidatos distintos por eleitor (ou só branco/nulo); `make tie` planta votos exatos. A Suite mostra caixas de marcar limitadas a N; o Emissor ganhou o cargo "senador".

## Consequências e limites honestos
- Mudança de formato (v5): eleições antigas precisam ser refeitas.
- Em 2 vagas o voto é cifrado: o total por candidato é público depois do fechamento, como nas outras eleições.
- A classe do Senado (1 ou 2 vagas) é escolhida pelo organizador do pleito; o sistema não calcula o calendário real.
- Quem lê o quadro vê que a cédula é de 2 vagas (tamanho da prova), mas não quais candidatos foram marcados.
- O proporcional (deputados e vereadores) foi resolvido no [ADR-0046](0046-cargos-proporcionais.md) e a rotação das instituições no ADR-0045.
