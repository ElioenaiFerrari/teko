# ADR-0025 — Consentimentos de bens assinados no dispositivo (`SignAssetConsent`)

> **Substituído em parte pelo [ADR-0036](0036-bens-negociacao-direta-com-janela-de-veto.md):** o dispositivo não assina mais um *consentimento* (`SignAssetConsent`); assina a **transação inteira** do usuário (`SignAssetOffer`/`SignAssetAccept`), que sai da carteira dele.

**Status:** Aceito provisoriamente (2026-11-14, continuação do M5, dono do produto). **Implementado** no dispositivo, no protocolo e no cliente (M5c). **Ainda sem tela na Suite**: o fluxo de compra e venda de bens na interface é o próximo passo.

## Contexto
Transferir um bem exige três consentimentos (dono, comprador e quórum do órgão, [ADR-0022](0022-bens-registro-com-orgao-e-troca-atomica.md)). O consentimento do **dono** e o do **comprador** são assinaturas da carteira de cada um, então precisam ser feitas **no dispositivo**, vendo o que se assina. O consentimento do órgão vem das chaves da instituição e não passa pelo dispositivo do cidadão.

## Decisão
- **Pedido** `SignAssetConsent{chain_id, kind, class, asset, from_address, to_address, price, version}`, com `kind` = 2 (transferência) ou 3 (venda contra pagamento). A Suite envia só campos.
- **O dispositivo mostra e assina a mesma coisa.** A tela traz o papel ("Você é o VENDEDOR/COMPRADOR"), o bem (`classe/id`), vendedor, comprador, **preço** (venda) ou "sem pagamento" (transferência), a **versão do bem** e a rede. A assinatura é sobre o hash da intenção, calculado em `teko-crypto::ledger::asset_intent_hash` (o **mesmo código** que o `teko-assets` usa para verificar), com a tag `teko/v2/asset-approval`.
- **Só assina se a carteira for parte da operação:** o endereço da carteira precisa ser o vendedor ou o comprador, e vendedor ≠ comprador. Senão, recusa **antes de qualquer tela**.
- **Validações antes da tela:** tipo conhecido; classe e id do bem sem caracteres de controle e dentro dos limites; endereços bech32m válidos; venda com preço positivo; transferência simples sem preço.
- **Sem repetição, por construção:** o consentimento está amarrado a rede, ação, bem, partes, preço e **versão**; qualquer mudança do bem torna as aprovações antigas inúteis (ADR-0022).
- **O cliente confere a assinatura** antes de devolvê-la à Suite.

## O que não muda
O consentimento do órgão (quórum M-de-N) e o registro inicial do bem continuam fora do dispositivo do cidadão. Registro, gravames e troca do órgão não têm tela do cidadão.

## Riscos e limites
- O dispositivo **não conhece a versão atual do bem** nem se o bem existe: mostra o que a Suite informa. Se a versão estiver errada, o livro recusa. O risco de enganar o cidadão está em ele confirmar um negócio que não combinou; por isso a tela traz todos os campos e a Suite deve mostrar os mesmos.
- **Oráculo:** continua valendo o limite do ADR-0022 (o livro prova quem consentiu, não que o mundo concorde).
- Sem auditoria; sem validade jurídica.
