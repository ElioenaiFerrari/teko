# ADR-0042 — Voto em branco e nulo como opções explícitas, fora da base da maioria

**Status:** Aceito provisoriamente (2026-11-16, dono do produto) e **implementado** · Fecha o item "voto em branco/nulo" pendente nos [ADR-0017](0017-cargos-jurisdicao-e-elegibilidade.md) e [ADR-0040](0040-candidatos-por-credencial-resultado-certificado-e-eleito.md); ajusta o [ADR-0016](0016-manifesto-de-eleicao-e-governanca.md).

## Contexto
Todo voto entrava no denominador da maioria: um branco ajudava a impedir a maioria absoluta e, em tese, podia até virar finalista do 2º turno. O dono do produto escolheu o desenho abaixo.

## Decisão
1. **Duas opções especiais, explícitas.** Cada opção do manifesto tem um `kind`: candidato (padrão), **branco** ou **nulo** (`ElectionOption.kind`, tag 5; o padrão 0 mantém o encode antigo). **Formato do manifesto 4**: firmware e dispositivos antigos recusam a assinatura (falha fechada), como no formato 3.
2. **Regras do pleito** (`validate_contest`): no máximo **um branco e um nulo**; eles têm nome fixo (`Branco`, `Nulo`), sem número e sem credencial; **pelo menos 2 candidatos**; o **2º turno** tem **exatamente 2 candidatos** (mais os especiais que o 1º turno tinha). O limite de 16 opções do pleito cifrado vale para candidatos + especiais (até 14 candidatos). Como o `kind` está no pleito assinado pela governança, **o pleito fixa quais especiais existem** e não muda depois.
3. **São opções como as outras na cédula.** Cifradas e provadas como qualquer posição (sigilo igual: na rede ninguém distingue quem votou em branco). O dispositivo mostra **"VOTO EM BRANCO"** / **"VOTO NULO"** com as **próprias palavras** (não as do manifesto) e avisa que não conta para nenhum candidato. A auditoria de Benaloh (ADR-0041) vale para elas.
4. **Fora do denominador.** A **maioria absoluta (> 50%) e o ranking contam só votos em candidatos**: `valid = soma dos votos dos candidatos`. Branco e nulo são contados e **mostrados** (apuração, Suite, resultado assinado), mas nunca lideram, nunca são finalistas e nunca vencem. Se **ninguém votou em candidato** (`valid == 0`), o resultado é TIE (sem eleito, PENDENTE do desempate). `ElectionOutcome` ganhou `blank_votes` (tag 15) e `null_votes` (tag 16); `totals` e `accepted` seguem como eram. A economia (`ClaimElected`) já exige candidatura existente por `credential_id`, então um branco/nulo nunca recebe a credencial `eleito`.
5. **"Nulo" é uma escolha explícita no dispositivo**, não uma cédula inválida: cédula inválida (duas opções, prova ruim) continua recusada pelas provas e conta como rejeitada.
6. **Ferramentas de teste:** `make election` e o 2º turno anexam as duas opções; `make simulate` planta ~3% de brancos e ~2% de nulos sobre o que os candidatos têm; `make tally` mostra "Branco · Nulo" à parte, com percentuais sobre os candidatos; a Suite mostra os dois como opções separadas e as barras e o mapa usam `valid`.

## Consequências e limites honestos
- Mudança de formato (v4): eleições antigas não carregam em firmware novo e vice-versa.
- O **total** de brancos e nulos é público (como o dos candidatos); quem votou neles não.
- Quem quiser outra regra (branco como voto válido, só branco sem nulo) muda o manifesto e a regra pura `decide()`; o formato já suporta "só um dos dois".
- Continuam PENDENTES: desempate legal, cargos de várias vagas e rotação das instituições da chain de votos.
