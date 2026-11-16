# ADR-0040 — Candidatos por credencial, resultado certificado, 2º turno e a credencial `eleito`

**Status:** Aceito provisoriamente (2026-11-16, dono do produto) e **implementado (etapa B)** · Completa o [ADR-0037](0037-credenciais-na-chain-emitidas-por-orgaos.md); ajusta [ADR-0016](0016-manifesto-de-eleicao-e-governanca.md), [ADR-0017](0017-cargos-jurisdicao-e-elegibilidade.md) e [ADR-0034](0034-votos-em-bft-institucional-e-sem-blocos-vazios.md).

## Contexto
Candidatos eram linhas de texto no manifesto, a chain de votos não registrava quem venceu e não havia credencial de "eleito". O dono do produto quer: candidatos que são **credenciais** emitidas por órgãos, temporárias (expiram N dias depois da eleição); o **eleito** registrado e uma credencial **vitalícia** que o próximo eleito do mesmo cargo e jurisdição **sobrepõe**; e **2º turno**.

## Decisão
1. **Candidatura = credencial pública `candidato`** com os atributos `cargo, uf, municipio, numero, nome, eleicao` em claro (formato canônico, validado em `teko-credentials::candidate`); emitida por um órgão (quórum), com janela de contestação e validade (≤ 365 dias).
2. **O pleito é montado a partir da chain** (`election-admin build-election`): entram as candidaturas **ativas** na hora de referência da economia, da eleição, do cargo e da jurisdição, com validade além do fechamento da votação mais uma margem de apuração. **O pleito é recusado** (em vez de a ferramenta escolher) se houver número ou titular repetido, ou menos de 2 candidatos. Cada opção leva o `credential_id` (campo novo do manifesto, **formato 3**: firmware antigo recusa a assinatura, falha fechada). A governança assina o manifesto, então a ligação opção ↔ credencial também é assinada; um pleito assinado **não muda**: uma candidatura revogada depois aparece como alerta na Suite.
3. **Resultado por regra pura** (`teko-election::outcome::decide`), só para cargos de **vaga única** (Presidente, Governador, Prefeito): no 1º turno de um pleito com `runoff`, **mais de 50% dos votos apurados elege**; sem isso, os **dois mais votados** vão ao **2º turno**; empate pela 2ª vaga de finalista (ou entre só dois) = **TIE** (sem eleito, PENDENTE); pleito sem `runoff` e o 2º turno: **mais votos vence**, empate no topo = TIE. O **Senador** (1 ou 2 vagas) é decidido pelo ADR-0044; deputados e vereadores (proporcional) **não geram resultado** (PENDENTE).
4. **O resultado é certificado pelas instituições**: depois da decifração, **cada instituição** da chain de votos calcula o resultado do **seu** apurador verificado, assina com a chave de consenso e publica no quadro (`EntryKind::OUTCOME = 7`; a apuração ignora). A chain de votos continua **só ordenando**; o que vale é **mais de 2/3 assinarem os mesmos bytes**.
5. **`eleito` na economia:** tipo de **sistema** (sem órgão), público e vitalício. `ClaimElected{resultado, assinaturas}`, enviado por **qualquer um**, é aceito se: o resultado é da chain de votos da gênese; **mais de 2/3** das instituições (chaves na gênese, `vote_validators=`) assinaram; é `ELECTED`; cargo e jurisdição combinam; **não foi reivindicado antes** (marcador por eleição+pleito+hash do pleito); a candidatura vencedora existe na economia, não foi vetada nem revogada e é deste pleito. Cria a credencial `eleito` **ativa na hora** para a chave da candidatura e **sucede** o anterior do mesmo `(cargo, nível, jurisdição)` (estado **Sucedida**; a história fica). Reeleito = novo registro que sucede o antigo; outro cargo não conflita.
6. **2º turno** (`election-admin runoff`): a partir do resultado `RUNOFF` certificado, a governança assina uma **revisão do manifesto** que **acrescenta** o pleito `<id>-2t` (campo `parent`, exatamente 2 opções = os finalistas, com seu calendário e sua chave de registro); os eleitores se registram de novo (assinatura cega) e votam; o resultado é por maioria simples e vira `eleito` pelo mesmo `ClaimElected`.
7. **Ferramentas de teste:** `make candidates` (órgão simulado emite as candidaturas de exemplo), `make election` (monta da chain, com a lista de exemplo para pleitos sem candidaturas), `make claim` (leva os resultados certificados à economia) e `make runoff CONTEST=…`. O Emissor ganhou a aba **Candidaturas** (formulário e lote). A Suite mostra `número – nome` e o selo **"candidatura conferida na chain"** por opção.

> **Atualização (ADR-0044):** o Senador (1 ou 2 vagas) é majoritário de várias vagas: os N mais votados, sem 2º turno, N credenciais `eleito` numa reivindicação.

> **Atualização (ADR-0043):** o empate que a regra não desfaz é decidido por **sorteio verificável** (só fica sem eleito se ninguém votou em candidato).

> **Atualização (ADR-0042):** a maioria absoluta passa a contar só votos em **candidatos**; branco e nulo são opções explícitas fora da base. O 2º turno tem 2 candidatos (mais branco e nulo).

## Consequências e limites honestos
- **Confiança:** a economia acredita em **mais de 2/3 das instituições** da chain de votos (mesma confiança do BFT); elas poderiam, juntas, assinar um resultado falso, e a economia não consegue recontar (não tem as cédulas). Quem quiser conferir de verdade recalcula a apuração do quadro (a Suite e o explorador fazem).
- **Conjunto de instituições:** copiado na gênese (época 0) e trocável por rotação aprovada por mais de 2/3 ([ADR-0045](0045-rotacao-das-instituicoes.md)); a economia guarda o conjunto em vigor. Com 3 instituições, "mais de 2/3" exige as 3.
- **Vaga única apenas.** Proporcional e várias vagas, o 2º turno só entre os dois primeiros são decisões PENDENTES.
- Uma candidatura revogada depois do pleito assinado continua na cédula (o pleito não muda); o selo avisa, e o remédio é cancelar o pleito com motivo e abrir outro.
- Sem validade jurídica; sem auditoria; só devnet.

## Testes
`teko-credentials` (`candidate`, `tests/elected.rs`), `teko-election` (`outcome`, manifesto v3 e 2º turno), `issuer-cli` (eleição com candidatos da chain e revisão do 2º turno), `teko-economy` (gênese), e o **ponta a ponta em nós reais** `devnet-sim/tests/elected.rs`. Ver `docs/TEST-MATRIX.md`.

## Decisão
Aceita provisoriamente com: candidaturas por credencial; resultado assinado por mais de 2/3 das instituições; cargos de vaga única com maioria absoluta e 2º turno; empate sem eleito (PENDENTE); `eleito` vitalícia que sucede a anterior.
