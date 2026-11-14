# ADR-0018 — Resultados em tempo real e política de visibilidade

**Status:** Proposto · **Decisão: PENDENTE (dono do produto)** · **Criticidade: alta** · Fase 2 (`PHASE-2-VOTING.md` §5.1) · **implementação de demonstração em andamento**

## Contexto
Requisito: gráfico por cargo e por estado, top 10, nome e votos, em tempo real. A apuração é uma função pura do log do quadro de avisos, então a parte técnica é direta. O problema é de **segurança e de produto**: resultado parcial divulgado durante a votação.

## Riscos de mostrar a distribuição ao vivo
- **Influência** sobre quem ainda não votou (efeito manada / voto útil).
- **Compra e monitoramento de voto:** o comprador vê o efeito do pagamento imediatamente (agrava ADR-0015).
- **Eleitorado pequeno:** em município pequeno a parcial quase identifica votos individuais.
- **Falsa sensação de confiabilidade:** sem registro anônimo e sem prova de elegibilidade (decisões #2 e #4), a contagem do protótipo **não é confiável**; mostrá-la ao vivo parece oficial.

## Alternativas
| | A. Ao vivo (`live`) | B. Só comparecimento (`turnout_only`) | C. Só após o fechamento (`after_close`) |
|---|---|---|---|
| Transparência em tempo real | Máxima | Parcial (quantas cédulas) | Nenhuma até fechar |
| Influência/compra de voto | Pior | Menor | Menor |
| Dá para **impor** com cédula pública? | **Não** (log em claro) | **Não** | **Não** |
| Dá para impor com cédula cifrada? | Não se aplica | Sim | Sim |

## Recomendação
Em **eleição real**: B ou C. Em **demonstração**: A, com **"DADOS SIMULADOS"** inconfundível. A política vive no **manifesto assinado** (`results_visibility`), então muda por decisão de governança, não por configuração local. **Enquanto a cédula for pública, dizer na interface que a política é convenção, não garantia.**

## Resultado por jurisdição
- Pleito estadual/municipal: o gráfico é por pleito (já é por UF/município).
- **Pleito nacional por UF** exigiria a UF na cédula (anonimato cai ao estado): **fora do primeiro corte** (decisão #15).

## Implementação de referência (demonstração)
`teko-election::tally` (função pura sobre o log; regras de aceitação verificáveis), `election-admin-cli` (publicar, simular, apurar) e a tela de resultados da Suite. A simulação grava um marcador para a UI exibir "DADOS SIMULADOS".

## Perguntas em aberto
Quem decide a política por eleição? Resultado parcial pode ser mostrado só aos auditores? Como comunicar ao eleitor que a parcial é simulada/não oficial? Limiar mínimo de cédulas antes de mostrar distribuição (privacidade em eleitorados pequenos)?
