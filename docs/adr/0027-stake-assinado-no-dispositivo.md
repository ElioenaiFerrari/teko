# ADR-0027 — Stake assinado no dispositivo e tela Validadores

**Status:** Aceito provisoriamente (2026-11-14, continuação do M5, dono do produto). **Implementado** (`SignStake`, tela **Validadores** da Suite) contra o livro local de teste. **Decisão: PENDENTE** continua a do [ADR-0021](0021-moeda-teto-fixo-stake-e-taxas.md) (quem valida no início da rede).

## Decisão
- **Ações de stake** da moeda (travar, sair, resgatar) são assinadas **no dispositivo** com `SignStake{chain_id, action, amount, fee, nonce}`. O dispositivo valida, **monta a transação**, mostra ação, valor, taxa, rede e o aviso de penalidade, e assina após o botão (mesmo princípio do ADR-0024). Ações: 1 travar, 2 sair (inicia a espera), 3 resgatar (sem valor).
- **Formato compartilhado:** `teko-crypto::ledger::coin_stake_payload` é usado pelo dispositivo e pela moeda.
- **Tela Validadores** (Suite): saldo livre, meu stake, saídas pendentes (com a contagem de blocos), penalidade, e a lista de validadores por stake com a **parcela do poder de voto**. O texto lembra a regra do consenso (mais de 2/3 do stake para fechar um bloco).
- **Mundo de teste único.** O saldo, o envio, os bens e o stake usam **o mesmo livro local** (`teko_assets::devnet::open`, moeda + bens). Isso corrige um problema: o caminho antigo só com moeda não conseguia reproduzir um arquivo que já tivesse transações de bens. No livro de teste a saída do stake espera 3 blocos (cada transação é um bloco).

## Limites
- **Sem rede de validadores real:** a lista é do livro local. Ligar o stake ao conjunto de validadores do consenso (BFT) é o M6, e depende da decisão do início da rede (ADR-0021).
- A penalidade (equivocação) existe no módulo de moeda e é testada com evidência real do BFT, mas **não há botão** que a provoque na Suite.
- Só **uma** carteira por dispositivo; sem delegação de stake (ninguém trava stake por outra pessoa).
- Sem auditoria; sem valor real.
