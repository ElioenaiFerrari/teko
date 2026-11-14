# ADR-0020 — Livro-razão genérico com módulos (`teko-ledger`)

**Status:** Aceito provisoriamente (2026-11-14, dono do produto aprovou o programa de livro-razão: dois livros, BFT primeiro). **Implementado** o núcleo (M2). Moeda (M3) e bens (M4) ainda não.

## Contexto
O quadro de votos é um log Merkle de entradas. Para moeda e bens precisamos de **estado** (saldos, donos) com regras, e de uma raiz de estado sobre a qual o consenso BFT concorde, sem misturar essas regras com o voto.

## Decisão
- **Máquina de estados replicada** (`Ledger`): executa **blocos** de transações assinadas sobre um estado chave-valor e expõe uma **raiz Merkle do estado** (folhas ordenadas por chave, então a raiz não depende da ordem de inserção) e **provas de inclusão** para clientes leves.
- **Envelope de transação (`Tx`)**: `chain_id`, `nonce`, `module`, `payload`, chave pública do remetente e assinatura P-256 com tag própria (`teko/v2/ledger-tx`). O endereço é o hash da chave pública. Codificação canônica (decodifica e recodifica; sobra de bytes é recusada). Payload ≤ 64 KiB; ≤ 10.000 transações por bloco.
- **Segurança de repetição**: o `nonce` deve ser igual ao número de transações já executadas pelo endereço; `chain_id` amarra a rede (uma transação da devnet não vale na testnet).
- **Módulos plugáveis** (`Module`): decidem o que o `payload` significa. Cada módulo **escreve só no próprio espaço de nomes** e **lê** os dos outros (a moeda lerá saldos para os bens, por exemplo). Devem ser **determinísticos**.
- **Blocos atômicos**: se uma transação falha, o bloco inteiro é recusado e nada muda. O proponente usa `select_valid` para montar blocos só com transações válidas.
- **Chamada entre módulos** (`Ctx::call_as`, M4): um módulo chama outro na mesma transação, em nome de um endereço que ele já autenticou (usado pela troca bem × moeda). Atômico; profundidade limitada a 3. Ver [ADR-0022](0022-bens-registro-com-orgao-e-troca-atomica.md).
- **Gênese** só antes do primeiro bloco (`genesis_put`).
- **Taxas (M3)**: a `Tx` tem o campo `fee` (assinado). O núcleo consulta o *módulo de taxas* (`with_fee_module`) para o mínimo (`min_fee`) e a cobrança (`charge_fee`), antes de executar o módulo da transação. Além disso, `begin_block` roda por bloco (recompensas), e o proponente do bloco é passado a `apply_block`/`select_valid`.
- **Fora do núcleo**: preço da taxa, stake, penalidades e emissão de moeda. Ficam nos módulos ([ADR-0021](0021-moeda-teto-fixo-stake-e-taxas.md)).

## Alternativas consideradas
- Reaproveitar o `teko-bulletin` (log): serve para registro, mas não dá saldo/estado nem raiz de estado.
- Máquina virtual (EVM/WASM): flexível, mas enorme e difícil de auditar; módulos em Rust bastam para um conjunto pequeno de regras conhecidas.
- Contas com saldo no núcleo: acopla a moeda; preferimos módulo.

## Consequências e limites
Sem auditoria. A raiz é recalculada por inteiro a cada consulta (O(n)): suficiente para o protótipo, não para milhões de contas (trabalho futuro: árvore esparsa/incremental). Sem um módulo de taxas configurado, o `mempool` não tem defesa contra spam além do limite de tamanho. Conferência de Merkle usa as funções RFC 9162 do `teko-bulletin`.
