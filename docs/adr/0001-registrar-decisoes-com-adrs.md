# ADR-0001 — Registrar decisões com ADRs

**Status:** Aceito (processo)

## Contexto
O projeto envolve decisões de segurança e produto com trade-offs não óbvios. É preciso rastreabilidade e um ponto de aprovação explícito.

## Decisão
Toda decisão relevante vira um ADR em `docs/adr/`. Decisões de segurança/produto só ficam `Aceito` após aprovação do dono do produto; o agente propõe alternativas e recomendação, **não decide**.

## Consequências
Mais escrita; menos retrabalho. ADRs substituídos permanecem no histórico com status `Substituído por NNNN`.
