# ADR-0030 — Penalidade automática e bloqueio (*lock*) do BFT persistido

**Status:** Aceito (2026-11-14, escolha do assistente a pedido do dono do produto: "o que você preferir"). **Implementado** em `teko-bft`, `teko-economy` e `teko-node`.

## Contexto
O ADR-0028 deixou dois limites: o nó só **contava** a evidência de equivocação (ninguém enviava o `Slash`), e o motor BFT **não gravava o *lock*** em disco. Sem o lock, um validador que reinicia depois de dar *precommit* num bloco pode dar *prevote* noutro, quebrando a garantia de segurança do Tendermint se isso ocorrer com mais de 1/3 do poder.

## Decisão
- **Penalidade automática.** Ao detectar duas mensagens assinadas em conflito (mesma altura, rodada e tipo), o nó monta a transação `Slash{evidência}`, **assina com a chave do próprio nó**, põe no seu mempool e a divulga aos pares (`ECON_TX_FWD`), **uma vez por ofensa** (validador, altura, rodada, tipo). O livro continua recusando repetição e evidência fora da janela de penalidade.
- **Lock persistido.** O `economy.wal` passa a guardar `altura rodada [rodada_do_lock id_do_bloco]`, gravado e sincronizado **antes** de cada voto sair. No reinício, na mesma altura, o motor volta na rodada seguinte **com o lock restaurado** (`Engine::start_height_locked`). Um lock de rodada igual ou posterior à de retomada é ignorado; uma altura nova nunca herda lock. Linha truncada ou corrompida no fim do WAL é ignorada, nunca inventada.

## Limites honestos
- A conta do nó não tem moedas: o `Slash` só entra enquanto `fee_per_byte` for 0 (devnet local). Com taxa ligada, alguém com saldo precisa enviá-lo.
- O bloco travado não é guardado, só o seu id: após o reinício o validador não consegue repropor o valor travado se for o proponente da rodada; ele segue seguro (não vota em outro bloco), mas pode precisar de mais uma rodada.
- Sem auditoria; vale só para a devnet/testnet.

## Testes
`teko-bft`: `a_restored_lock_survives_a_restart_and_is_dropped_on_a_fresh_height`. `teko-node`: `the_wal_remembers_the_round_and_the_lock_across_a_restart`, `an_equivocation_becomes_one_slash_transaction_per_offence`. A execução do `Slash` no livro já era coberta em `teko-economy` (`equivocation_evidence_becomes_a_slash_of_the_owner`).
