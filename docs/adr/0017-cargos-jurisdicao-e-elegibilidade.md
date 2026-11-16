# ADR-0017 — Cargos, jurisdição e elegibilidade (vários pleitos por eleição)

> **Atualização (ADR-0040):** os candidatos de Presidente, Governador e Prefeito vêm de **credenciais `candidato`**; só esses cargos (vaga única) geram resultado e eleito automáticos. Senador, deputados e vereadores seguem PENDENTES.

**Status:** Proposto · **Decisão: PENDENTE (dono do produto)** · Fase 2 (`PHASE-2-VOTING.md` §3.1) · **modelo de dados já implementado** em `teko-election`

## Contexto
Uma eleição tem vários cargos (Presidente, Governador, Prefeito...), cada um com **jurisdição, candidatos e prazos próprios**. O cidadão vota só nos pleitos do seu domicílio eleitoral; o Presidente é de todos. Isso afeta elegibilidade, privacidade e chaves.

## Decisões de modelagem (implementadas, revisáveis)
- **Eleição = contêiner de pleitos**; **cada pleito** tem cargo, escopo (nacional / UF / município IBGE), opções e calendário próprios.
- O **cargo deve combinar com o escopo** (Governador ⇒ estadual etc.); `Other` (plebiscito) aceita qualquer nível.
- Ciclo de vida e transições **por pleito**, amarradas ao **hash do pleito**; revisões do manifesto só **acrescentam** pleitos (2º turno).
- `is_eligible(domicílio, escopo)` é uma regra pura e testada. **Quem a aplica de verdade é a autoridade** ao conceder o direito de registro; a checagem no dispositivo é só cortesia (um dispositivo clonado não pode ampliar os próprios direitos).

## Alternativas
**A. Um manifesto por eleição com vários pleitos** (implementado): uma assinatura M-de-N cobre tudo; revisões acrescentam pleitos. *Contra:* manifesto cresce (eleição geral tem 27 UFs × cargos).
**B. Um manifesto por pleito:** pequenos e independentes. *Contra:* mais assinaturas M-de-N, sem visão única da eleição, e é mais difícil garantir consistência (ex.: mesmo `election_id`).
*Recomendação:* **A**, com manifestos regionais derivados só para o dispositivo exibir (mesmo conteúdo, hash por pleito).

## Onde mora o domicílio eleitoral?
1. **Claims na credencial de identidade** (mesma autoridade emite `electoral_state` e `electoral_municipality`). Simples; mistura dois papéis.
2. **Credencial de elegibilidade separada**, emitida pelo cadastro eleitoral (no Brasil, a Justiça Eleitoral), com claims de domicílio. Mais fiel à realidade e separa responsabilidades.
*Recomendação:* **2** como arquitetura-alvo; no estudo o `issuer-cli` simula os dois papéis (com claims próprias do cadastro eleitoral).

## Privacidade (novas ameaças V11–V15 no THREAT-MODEL)
- **Anonimato = tamanho do eleitorado do pleito.** Município pequeno ⇒ conjunto pequeno.
- **Chave de eleitor por pleito** (`election_id ‖ contest_id`), senão o voto nacional herda o conjunto de anonimato do municipal.
- **Registros em momentos e canais separados** por pleito.
- O registro vê o domicílio (necessário), nunca a identidade.

## Perguntas em aberto
Data de corte do cadastro e mudança de domicílio; voto em branco/nulo; cargos proporcionais e multi-vaga; eleitor no exterior; encerramento antecipado; mínimo de eleitores por pleito para permitir votação anônima (limiar de anonimato).
