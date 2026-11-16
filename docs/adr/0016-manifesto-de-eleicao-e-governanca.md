# ADR-0016 — Manifesto de eleição, ciclo de vida e governança

> **Atualização (ADR-0040):** o manifesto passou ao **formato 3**: cada opção pode levar o `credential_id` da candidatura na chain, o pleito pode admitir **2º turno** (`runoff`) e um pleito de 2º turno aponta o de origem (`parent`). Revisões continuam só **acrescentando** pleitos (é como o 2º turno entra).

**Status:** Proposto · **Decisão: PENDENTE (dono do produto)** · Fase 2 (§7)

## Contexto
O dispositivo precisa saber, de forma verificável, qual eleição é, quem são os candidatos e em que fase está, sem confiar no host.

## Proposta (atualizada: vários pleitos, ver ADR-0017)
- **Manifesto** assinado **M-de-N** (reusa `teko-attestation::multisig`): id da eleição, título e **lista de pleitos**, cada um com cargo, jurisdição, opções **ordenadas** e **calendário próprio**. O dispositivo verifica e exibe nomes **do manifesto**.
- **Ciclo de vida por pleito**, sem volta: `Rascunho → CandidatosPublicados → RegistroAberto → VotacaoAberta → VotacaoEncerrada → Apurada → Auditada` (+ `Cancelada`, só até a apuração, com motivo). Cada transição é assinada M-de-N, tem número de sequência global e é amarrada ao **hash do pleito**.
- **Revisões** do manifesto só **acrescentam** pleitos (ex.: 2º turno); alterar ou remover um pleito existente é recusado, então o histórico anterior continua válido.
- **Implementado** em `crates/teko-election` (21 testes).

## Perguntas em aberto (produto)
Quem são as N instituições e quanto vale M? Rotação/revogação de chaves? Como contestar uma eleição? O dispositivo guarda quantas eleições ao mesmo tempo? *Recomendação:* reusar o 2-de-3 de teste; limite pequeno de eleições no dispositivo.
