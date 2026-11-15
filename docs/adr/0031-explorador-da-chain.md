# ADR-0031 — Explorador da chain (somente leitura, verificado no cliente)

**Status:** Aceito (2026-11-15, dono do produto). **Implementado:** `teko-explorer`, `explorer-server`. Controles de desenvolvimento: **PENDENTE** (decisão abaixo).

## Contexto
Só havia comandos de terminal para ver a chain. O dono do produto quer uma ferramenta de desenvolvimento que depois sirva como explorador público, no estilo Etherscan.

## Decisão
- **Servidor web local + API JSON somente leitura** (`GET /api/v1/…`), com página HTML/CSS/JS pura servida pelo mesmo binário (`teko-explorer`, `make explorer`). Escuta só em loopback; fora dele exige `--public` e um proxy com HTTPS.
- **O explorador não confia nos nós.** A biblioteca `teko-explorer` (`Index`, sem I/O) baixa os blocos e verifica certificado, proponente, execução e raiz do estado com o mesmo código dos nós (`EconomyChain::apply_decided_outcome`, que passou a devolver recibos e eventos). Bloco inválido é recusado e o índice mantém a cadeia que já verificou.
- **Valores** em ypy (inteiro) e em TEKO exato ([ADR-0029](0029-unidade-teko-e-ypy.md)).
- **Livro de votos (PoA):** só cabeçalhos, contagens por tipo e assinaturas M-de-N; o conteúdo das entradas não é exposto.
- **Segurança da página:** CSP sem inline, `X-Content-Type-Options`, `X-Frame-Options`, `no-referrer`; todo texto da API é escapado; a API não tem segredo nem escreve nada (405 para outros métodos).
- **Dependências novas:** `tiny_http` (servidor síncrono pequeno, como o resto dos nós), `serde` e `serde_json`.

## Pendente: controles de desenvolvimento (estilo Ganache)
Faucet para outro endereço, reiniciar a rede e ritmo dos blocos **não foram feitos**. O motivo é de produto: a operação `Faucet` do `teko-coin` credita quem assina (a carteira, no dispositivo), então dar moedas a outro endereço exige uma **conta de dev financiada na gênese**, o que contraria o "sem pré-mineração" da fase de fundador (ADR-0028). Alternativas e recomendação serão apresentadas ao dono do produto.

## Limites
Índice em memória; sem WebSocket, autenticação ou HTTPS próprios; sem auditoria.

## Testes
`crates/teko-explorer/tests/index.rs`, `tools/explorer-server/tests/api.rs` e `tests/e2e.rs` (3 nós reais).
