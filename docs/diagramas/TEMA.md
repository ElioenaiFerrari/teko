# Tema dos diagramas do Teko

Todos os diagramas seguem a skill `diagram-design` (conectores ortogonais com cantos arredondados, rótulos com margem, sem sombras, no máximo 9 nós por diagrama) com a **identidade visual do Teko**: Brasil, Amazônia e fauna nativa. O mesmo tema vale para a Suite (`apps/suite/ui/styles.css`).

## Paleta

| Papel | Cor | Origem | Uso |
|---|---|---|---|
| `paper` | `#F5EFE3` | areia do rio | fundo |
| `paper-2` | `#EFE6D3` | areia úmida | regiões |
| `ink` | `#1A2B22` | jenipapo | texto e contornos |
| `muted` | `#4A5C52` | | texto secundário e setas |
| `accent` | `#F2B705` (tinta `#FFF4CC`) | ouro | foco: no máximo 1–2 por diagrama |
| `link` | `#1F4BB8` | arara-azul | emissão, API, Suite |
| verde | `#12663B` | mata | caminho feliz e verificação |
| guará | `#C62F24` | guará | **só** falha, ataque ou cancelamento |
| onça | `#9A5208` | onça-pintada | dispositivo |
| boto | `#B8365F` | boto-cor-de-rosa | livro, quadro e rede |

A faixa verde–ouro–azul no topo de cada imagem lembra a bandeira.

## Tipografia
Instrument Serif (título), Geist (nomes) e Geist Mono (termos técnicos: crates, funções, mensagens), com reservas `Avenir Next`, `Segoe UI` e `system-ui`.

## Regras de conteúdo
- Textos em **pt-BR**; nomes de crates, funções e mensagens ficam em inglês, em mono.
- Contraste mínimo 4,5:1; nunca usar só a cor para dar significado (há legenda e rótulos).
- Acima de 9 nós ou 12 setas, dividir em visão geral + detalhe. Diagrama de sequência: até 5 linhas de vida.
- Tracejado = retorno, leitura ou **planejado**.

## Como gerar
Os diagramas são código: `scripts/diagramas/` (`lib.py` com o tema e `d_*.py` com cada diagrama).

```bash
make diagramas                 # regenera todos (precisa do navegador Brave para o PNG)
python3 scripts/diagramas/gerar.py voto-cifrado   # só um
```

Cada diagrama produz `docs/diagramas/<nome>.svg` (vetorial), `.html` (fonte autônoma) e `.png` (2x, embutido nos markdowns). **Altere o gerador e rode de novo; não edite os arquivos gerados à mão.**
