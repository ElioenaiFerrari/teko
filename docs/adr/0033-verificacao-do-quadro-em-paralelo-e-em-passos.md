# ADR-0033 — Verificação do quadro em paralelo, em passos e com estado compartilhado

**Status:** Aceito (2026-11-15). **Implementado:** `teko-election` (`precheck`), `election-admin-cli` (`feed_step`), Suite, `teko-node` (`NodeWorld`).

## Contexto
As telas Resultados ao vivo e Exploração pública demoravam dezenas de segundos. Medido numa devnet com 274 blocos e 6.640 entradas (5 MB):
- baixar e verificar a cadeia de votos: **0,49 s** (não é o download);
- apurar o quadro (assinatura de cada cédula, provas ElGamal, tokens de registro): **21,8 s** no release e 28 s no build de dev, em uma única thread, a **3,3 ms por entrada**; a contagem em si leva 38 ms.

E o trabalho se repetia: a Exploração refazia a apuração inteira a cada abertura; reiniciar a Suite perdia o estado incremental; e cada comando da Suite que lê a economia rebaixava e reverificava a cadeia de economia inteira.

## Decisão
1. **Conferência em paralelo, contagem em ordem.** `teko_election::precheck` e `precheck_registration` fazem, sem estado, as partes caras (assinatura e provas da cédula; token RSA do registro). O `election-admin-cli` roda isso em lotes de 256 entradas, em várias threads (núcleos da máquina, até 16; `TEKO_VERIFY_THREADS=1` força o sequencial), e depois aplica as entradas **uma a uma, na ordem do quadro** (`Tallier::feed_prechecked`). O veredito só vale para o **mesmo conteúdo** (hash do payload) e a **mesma chave e número de opções** da eleição em vigor; senão a conferência é refeita. O resultado é **idêntico** ao sequencial (testado, inclusive com cédulas adulteradas, duplicadas, com prova ou assinatura inválida e com veredito de outra chave).
2. **Passos e progresso.** `feed_step(max)` avança no máximo `max` entradas e devolve quantas foram consumidas. A Suite responde com o que já foi verificado e o progresso (`verified` e `total`), e a tela mostra "Verificando o quadro: X de Y".
3. **Um só estado, aquecido no início.** Resultados e Exploração usam o **mesmo** apurador (`Suite.board`), e uma thread de fundo o mantém em dia desde que a Suite abre.
4. **Economia em cache.** `NodeWorld::open` guarda a última cadeia verificada no processo e, na abertura seguinte, baixa e verifica **só os blocos novos**; se a rede foi recriada (os nós estão mais curtos) ou a cadeia em cache não bate, refaz do zero.
5. **Visibilidade.** Se um passo, ou a abertura da economia, passar de 300 ms, o terminal do `make suite` mostra uma linha de tempo; `TEKO_VERIFY_TRACE=1` mostra o tempo de verificação e de contagem na apuração (`make tally-time`). O motivo de a economia de rede não abrir (cadeia antiga, nós fora do ar) agora é impresso, em vez de cair em silêncio no arquivo local.
6. **Sem cache em disco.** Um arquivo de "entradas já verificadas" faria a Suite pular verificações com base em algo gravado no disco: exige proteção de integridade e fica para quando o quadro for grande demais para revalidar a cada abertura.

## Resultado medido (6.640 entradas, 11 núcleos)
Apuração: 21,8 s com 1 thread, 6,5 s com 4, 4,5 s com 8, **3,7 s com 11**. A Suite abre, aquece em segundo plano e a tela mostra progresso em vez de ficar em branco.

## Limites
- Reiniciar a Suite ainda revalida o quadro todo (agora em paralelo e em segundo plano). Com centenas de milhares de cédulas volta a ser lento; aí vale um ponto de verificação em disco com integridade, ou verificação em lote das provas (matemática mais complexa).
- O ganho depende dos núcleos (núcleos de eficiência pesam menos).

## Testes
`teko-election`: `prechecked_feeding_gives_exactly_the_sequential_result`; `election-admin-cli`: `checking_ballots_on_many_threads_gives_the_sequential_result`, `feeding_in_steps_reports_progress_and_ends_at_the_same_state`; `teko-node`: `reopening_the_world_downloads_only_the_new_blocks`.
