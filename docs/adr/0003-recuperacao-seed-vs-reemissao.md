# ADR-0003 — Recuperação: seed phrase vs reemissão presencial

**Status:** Aceito provisoriamente (2026-11-13: dono do produto mandou seguir; recomendação adotada: sem seed, reemissão presencial) · revisável

## Contexto
Se o dispositivo for perdido, quebrado ou apagado (N erros de PIN), como o cidadão recupera a identidade? A tendência do dono do produto é **sem seed**.

## Alternativas
| | Seed phrase | Reemissão presencial (sem seed) |
|---|---|---|
| Conveniência | Alta | Baixa (deslocamento à autoridade) |
| Venda/compra de voto | **Seed é ativo vendável**: quem a compra passa a ser o eleitor | Dispositivo físico é vendável, mas a venda é detectável por revogação e reemissão |
| Superfície de ataque | Seed em papel, fotos, phishing | Autoridade passa a ser ponto central |
| Segredo de eleitor | Precisa ser derivável da seed (quebra isolamento) ou fica sem backup | `s` novo na reemissão (commitment antigo revogado) |
| Revogação do antigo | Difícil (seed clonável) | Natural |
| Custo operacional | Baixo | Alto |

## Recomendação
**Sem seed**: reemissão presencial, revogando o dispositivo/credencial antigo e o commitment de eleitor antigo. Registrar o custo operacional e a dependência da autoridade (THREAT-MODEL §5, X1).

## Perguntas
- Como revogar o commitment sem vincular a identidade (interação com ADR-0008)?
- Prazo máximo de reemissão; o que acontece com um voto já emitido?
- A reemissão gera nova chave de identidade e novo ID Cívico ou preserva o ID? (KEY-HIERARCHY, pergunta 5)

## Consequências
Sem seed, a perda total do dispositivo exige processo presencial; elimina o vetor "venda de seed".
