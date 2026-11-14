# ADR-0015 — Sigilo do voto, cédula aberta ou cifrada e resistência à coerção

**Status:** Proposto · **Decisão: PENDENTE (dono do produto)** · **Criticidade: máxima** · Fase 2 (§6)

## Contexto
Anonimato por não-ligação (identidade ↔ cédula) **não** impede compra de voto: se o eleitor consegue provar como votou (recibo), alguém pode pagar por isso. Resistir a isso muda a arquitetura inteira.

## Alternativas
1. **Cédula pública + nullifier.** Simples. Recibo trivial. Sem resistência à compra.
2. **Cédula cifrada para curadores (limiar), apuração após o fechamento.** Sigilo até o fim; curadores viram ponto de confiança; ainda pode haver recibo.
3. **Re-voto (vale o último) / estilo MACI.** O coator não sabe qual voto valeu. Exige coordenador de confiança e nullifier que permita substituição.
4. **Receipt-free de verdade.** Pesquisa ativa, complexidade muito alta.

## Recomendação
Primeira versão com **1**, **declarando explicitamente** na UI e nos docs que **não resiste à compra de voto**, e com a interface de cédula e quadro desenhada para permitir **3** depois. Qualquer afirmação de "voto seguro contra coerção" **não** deve ser feita enquanto isso não mudar.

## Consequências
O THREAT-MODEL mantém C2–C5 como risco residual total até esta decisão evoluir.
