# ADR-0007 — Atestação em lote (FIDO-style) e tamanho do lote

**Status:** Proposto · **Decisão: PENDENTE (dono do produto)**

## Contexto
A atestação prova que o dispositivo é genuíno sem permitir rastrear um dispositivo (princípio 6). Uma chave única por dispositivo identificaria cada um; uma chave por lote grande só identifica o lote.

## Alternativas
| | Chave por dispositivo | Chave de lote | Atestação anônima (DAA/ZK) |
|---|---|---|---|
| Rastreabilidade | **Total** | Só o lote | Nenhuma |
| Revogação individual | Fácil | **Difícil** (revogar o lote afeta todos) | Possível (listas) |
| Complexidade | Baixa | Baixa | Alta |

## Recomendação
**Chave de lote** (cadeia: raiz da autoridade → certificado de lote → dispositivo) com **lotes ≥ 100 000** dispositivos (valor a validar), e a atestação **nunca** acompanhando um voto. A revogação de dispositivo individual ocorre no nível da credencial de identidade (ADR-0003), não da atestação.

## Perguntas
- Tamanho mínimo do lote vs. necessidade de recall de um lote comprometido.
- O certificado do dispositivo contém algum identificador único? **Não deve.**
- Atestação é feita só na ativação ou a cada sessão da Suite? (frequência × privacidade)

## Consequências
Vazamento de uma chave de lote permite falsificar dispositivos daquele lote (risco S4 no THREAT-MODEL).
