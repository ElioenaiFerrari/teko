# ADR-0009 — Interface do emulador: terminal (TUI) vs janela

**Status:** Aceito provisoriamente (2026-11-13) · **Implementado: terminal** (tela e botões no terminal; janela egui fica para depois). Modo automático só sob a feature `test-buttons`.

## Contexto
O emulador precisa de "tela" e "botões" e é onde o PIN e as confirmações acontecem. Deve rodar como processo separado e permitir modo headless para testes.

## Alternativas
| | TUI (`ratatui`) | Janela (`egui`/`eframe`) |
|---|---|---|
| Fidelidade a tela+botões | Média | **Alta** (tela pixelada + botões clicáveis) |
| Dependências | Poucas | Mais (gráficas) |
| Roda em CI/SSH | Sim | Precisa de display (ou modo headless) |
| Esforço | Baixo | Médio |

## Recomendação
**Janela `egui`** como interface principal, com **modo headless** (`test-buttons`, só em feature de teste) para o e2e. TUI é aceitável como primeiro passo, se quiser reduzir escopo.

## Consequências
Em ambos os casos o core fica atrás da HAL (`Display`/`Buttons`); a UI é trocável. Compilar o modo headless somente sob feature de teste, nunca no build normal.
