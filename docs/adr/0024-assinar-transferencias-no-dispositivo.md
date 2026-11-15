# ADR-0024 — Assinar transferências de moeda no dispositivo (`SignTransfer`)

**Status:** Aceito provisoriamente (2026-11-14, continuação do M5, dono do produto). **Implementado** (M5b) para a **moeda**; bens e saldo na rede ficam para depois.

## Decisão
- **O dispositivo monta o que assina.** A Suite envia só campos (`chain_id`, endereço de destino, valor, taxa, `nonce`). O dispositivo valida, **monta a transação do livro-razão ele mesmo**, **mostra** destino, valor, taxa, total e rede na tela e assina **somente após a confirmação física**. Os bytes assinados vêm dos mesmos valores exibidos (princípio 1).
- **Formato compartilhado.** Os bytes de assinatura da transação e o payload da transferência de moeda vivem em `teko-crypto::ledger` (`no_std`) e são usados pelo dispositivo **e** pelo `teko-ledger`/`teko-coin`, para não haver duas versões do formato.
- **Validações antes de qualquer tela:** carteira pronta e dispositivo desbloqueado; valor positivo; valor + taxa sem estouro; destino bech32m válido e **diferente da própria carteira**; rede sem caracteres de controle. Pedido inválido não mostra nada nem pede confirmação.
- **Quem entrega.** O cliente remonta a `Tx` e **confere a assinatura** antes de usá-la. A Suite escolhe o `nonce` e calcula a taxa mínima (preço por byte do ADR-0021) a partir do livro e confere o saldo **antes** de incomodar o dispositivo; o livro recusa de novo por conta própria.
- **Livro local de teste (só desenvolvimento).** Enquanto o livro de economia não está na rede (M6), a devnet local (`dev/economy.txt` com alocações de TESTE e `dev/economy.txs` com as transações aceitas) é reproduzida pelo `Ledger` e pelo módulo de moeda reais (`teko_coin::devnet::Economy`). O build de produção da Suite não tem envio nem saldo.

## O que o usuário vê
Na tela do dispositivo: "ENVIAR MOEDAS", `Para:` (endereço quebrado em linhas), `Valor`, `Taxa`, `Total`, `Rede`, e os botões confirmar/rejeitar. A Suite avisa para **conferir o endereço de destino no dispositivo**.

## Riscos e limites
- **Troca de endereço pelo computador (clipboard/malware):** é o ataque clássico; a defesa é o usuário **conferir o destino na tela do dispositivo**, que é a única fonte confiável. A Suite não consegue alterar o que o dispositivo mostra nem assinar sem o botão.
- O dispositivo **não conhece o saldo** nem verifica o `nonce`: quem confere são a Suite e o livro. Um `nonce` errado só gera transação recusada.
- Só transferência de **moeda**. Assinar bens (consentimentos com intenção, ADR-0022), taxas de outra natureza e envio para a rede de validadores ficam para os próximos marcos (M5c/M6).
- Sem auditoria; sem valor real.
