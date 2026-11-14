# ADR-0016 — Manifesto de eleição, ciclo de vida e governança

**Status:** Proposto · **Decisão: PENDENTE (dono do produto)** · Fase 2 (§7)

## Contexto
O dispositivo precisa saber, de forma verificável, qual eleição é, quem são os candidatos e em que fase está, sem confiar no host.

## Proposta
- **Manifesto** assinado **M-de-N** (reusa a lógica de `teko-attestation`): id, título, datas, lista **ordenada** de opções, regras, raízes de confiança (Autoridade, Registro, curadores), parâmetros de prova. O dispositivo verifica e exibe nomes **do manifesto**.
- **Ciclo de vida** em máquina de estados sem volta: `Rascunho → CandidatosPublicados → RegistroAberto → VotacaoAberta → VotacaoEncerrada → Apurada → Auditada` (+ `Cancelada`). Cada transição é uma mensagem assinada M-de-N no quadro. Mudança = novo manifesto versionado.

## Perguntas em aberto (produto)
Quem são as N instituições e quanto vale M? Rotação/revogação de chaves? Como contestar uma eleição? O dispositivo guarda quantas eleições ao mesmo tempo? *Recomendação:* reusar o 2-de-3 de teste; limite pequeno de eleições no dispositivo.
