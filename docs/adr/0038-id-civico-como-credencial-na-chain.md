# ADR-0038 — ID cívico como credencial na chain, emitida pelo órgão

**Status:** Aceito provisoriamente (2026-11-16, dono do produto) e **implementado (etapa D)** · **Supera em parte o [ADR-0005](0005-credenciais-sd-jwt-vs-bbs.md)** (formato da credencial de identidade e lista de revogação fora da chain) e ajusta [ADR-0003](0003-recuperacao-seed-vs-reemissao.md), [ADR-0008](0008-registro-de-commitment-sem-vinculo.md) e [ADR-0037](0037-credenciais-na-chain-emitidas-por-orgaos.md).

## Contexto
A credencial de identidade era um blob SD-JWT **fora da chain**, assinado por **uma só chave de autoridade** (`issuer-cli`) e instalada pela Suite do cidadão. O dono do produto quer a emissão **fora da Suite**, no app do órgão, e a credencial tratada como **mais uma credencial** do [ADR-0037](0037-credenciais-na-chain-emitidas-por-orgaos.md).

## Decisão
1. **Tipo `civic`** (privado, 2-de-3 na devnet): o ID cívico é emitido pelo **quórum de um órgão**, no app **Teko Emissor** (aba *ID cívico*), com a janela de contestação e a validade do ADR-0037. O `issuer-cli` deixa de emitir, reemitir e revogar.
2. **O titular é a chave de identidade** (exceção consciente à regra de "uma chave por tipo, sem ligação"): é isso que amarra a pessoa ao ID Cívico, às assinaturas de documentos, às provas de atributo e ao registro anônimo de voto. Segue **sem seed** (reemissão presencial).
3. **O dispositivo confere o pacote sozinho** (ele não tem relógio confiável nem acesso à chain): o titular recebe um **`CivicBundle`** (atributos, sal e as transações `Propose` e `Approve` assinadas pelos membros, **tiradas da chain**). O dispositivo recalcula compromisso e id, confere cada assinatura e exige **M membros distintos** do órgão que o `trust.txt` lista (`civic_chain_id`, `civic_threshold`, `civic_member`, escritos pelo `teko-node init`). Veto, janela, expiração e revogação são conferidos por **quem usa a credencial, lendo a chain** (a Suite, o registro de voto), como já era a lista de revogação.
4. **Divulgação seletiva mantida:** o compromisso passa a ser um hash dos **resumos salgados de cada atributo** (sal por atributo derivado do sal da credencial). A prova de idade revela só `age_over_18` e os resumos dos demais; o verificador reconstrói o compromisso e o compara com o **registro na chain**. A apresentação é assinada pela chave de identidade sobre nonce e verificador.
5. **Pedido do cidadão:** a Suite gera um **pedido** (`GetIdentityRequest`: prova de posse da chave de identidade + atestação do lote do dispositivo) e o salva em `dev/requests/`. O Emissor o **verifica** antes de emitir (mantém a garantia de dispositivo genuíno que uma chave colada em hex perderia).
6. **Revogação e reemissão** passam a ser **na chain**: `Revoke` por quórum (motivo público) e nova emissão com o **mesmo `registry_id`**, que é como o registro de voto reconhece o mesmo cidadão (substitui a `citizens.log`). A lista de revogação assinada por chave única deixa de existir.
7. **Registro anônimo de voto** confere o pacote **e** que a credencial está **ativa na chain**; o cidadão é o `registry_id`. A Suite tem uma fase em que carteira, bens, credenciais e stake funcionam **antes** do ID cívico (antes isso era circular).

## Consequências e limites honestos
- A confiança sai de **uma chave única** para o **quórum do órgão**, e tudo fica auditável na chain; o oráculo continua (o livro prova quem emitiu, não a verdade).
- O dispositivo não vê veto nem revogação: uma credencial vetada ou revogada **continua instalada** nele, mas **não é aceita** por quem lê a chain.
- **Dedupe de pessoa:** a chain não impede duas identidades (chaves de dispositivo diferentes); o órgão deduplica por `registry_id`.
- Mudar o órgão ou a rede exige atualizar o `trust.txt` e **reiniciar o emulador** (o `teko-node init` escreve; `issuer-cli dev-setup` apaga e precisa vir antes).
- Sem validade jurídica; sem auditoria; só devnet.

## Testes
`teko-crypto/src/civic.rs` (pacote, quórum, compromisso por atributo, apresentação), `teko-device-core` (instalar, recusar, provar), `teko-client/tests/e2e.rs` (emissão com o emulador, prova contra a chain, registro, reemissão), `teko-attestation` (política), `teko-credentials/src/civic.rs` (atributos). Ver `docs/TEST-MATRIX.md`.

## Decisão
Aceita provisoriamente com: tipo `civic` por quórum; titular = chave de identidade; pacote conferido no dispositivo pelo quórum do `trust.txt`; compromisso por atributo; revogação e reemissão na chain.
