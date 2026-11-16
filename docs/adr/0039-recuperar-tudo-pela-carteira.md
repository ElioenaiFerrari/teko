# ADR-0039 — Recuperar pela carteira: chaves de credencial da seed e atributos cifrados na chain

**Status:** Aceito provisoriamente (2026-11-16, dono do produto) e **implementado (etapa E)** · **Emenda o [ADR-0037](0037-credenciais-na-chain-emitidas-por-orgaos.md) (item 5: de onde vem a chave de credencial) e o [ADR-0023](0023-carteira-com-seed-bip39.md) (o que a frase recupera)**.

## Contexto
O dono do produto quer que **recuperar a carteira pela frase** devolva **tudo o que é do cidadão**, não só o saldo: bens, histórico e credenciais. Dinheiro e bens já viviam na chain, ligados ao endereço. As credenciais não: o titular era uma chave de credencial derivada da raiz de **identidade** (que não tem seed) e os atributos privados ficavam só num pacote com o titular.

## Decisão
1. **A chave de credencial vem da carteira:** `m/7240'/1'/t'` (hardened), com `t` = 31 bits de `H(CRED_KEY, tipo)`. A mesma frase (e passphrase) refaz todas as chaves em qualquer dispositivo. Os ramos são independentes: as chaves públicas não se ligam entre si nem aos endereços `m/7240'/0'/0'/0'/i'`.
2. **Os atributos privados vão para a chain cifrados ao titular:** no `Propose`, o **dispositivo do membro** sela `sal ‖ atributos` à chave de credencial do titular (ECDH P-256 efêmero + KDF SHA3 + ChaCha20-Poly1305, contexto `tipo ‖ titular` autenticado) e põe o resultado em `sealed` (campo do `Propose`, guardado no `CredRec`). Tudo dentro da transação que o dispositivo assina (princípio 1); o cliente refaz a mesma transação com o `sealed` que o dispositivo devolve.
3. **Abrir só no dispositivo:** `OpenCredentialBundle` decifra com a chave de credencial (que nunca sai), mostra os atributos e só os entrega à Suite depois do botão; a Suite confere o resultado contra o **compromisso** da chain.
4. **"Recuperar tudo" na Suite** (tela Credenciais): lê a chain verificada, pede **todas as chaves de uma vez** (`GetCredentialKeys`, uma confirmação), varre `credentials_of` e mostra saldo, stake, bens e credenciais; os atributos privados são abertos no dispositivo (um botão por credencial). As chaves **públicas** de credencial são guardadas em disco (`dev/cred-keys.json`).
5. **O que continua fora da seed:** a **identidade** (chave de identidade, ID Cívico e a credencial `civic`) e o **registro de voto** (segredo de eleitor): reemissão presencial, como no ADR-0003 e no ADR-0038. O tipo `civic` não usa a chave da carteira e **não** é selado (a chave de identidade não é recuperável). O dispositivo recusa `GetCredentialKey("civic")`.
6. **Explorador:** o bem registrado para alguém e a oferta feita a alguém passam a aparecer também no histórico **dessa** pessoa (antes só no de quem assinou).

## Consequências e limites honestos
- **Quem tem a frase tem o dinheiro, os bens e as credenciais** (CNH, candidatura…). O ADR-0023 já dizia que a frase vale dinheiro; agora vale também credenciais. A frase não dá a identidade nem o voto.
- A passphrase também separa as credenciais: outra passphrase é outra carteira e **não encontra nada** (consistente com a carteira-isca).
- **Os cifrados ficam na chain para sempre.** Se a frase vazar um dia, os atributos privados já emitidos podem ser lidos retroativamente (colheita agora, decifra depois).
- **Chave por tipo estável:** duas credenciais do mesmo tipo da mesma carteira são ligáveis entre si.
- Os atributos têm limite de tamanho (~2 KB); credenciais emitidas **sem** `sealed` (o emissor antigo ou o `civic`) não são recuperáveis pelos atributos, só o fato de existirem.
- O dispositivo do **membro do órgão** precisa selar (sempre faz para tipos privados que não sejam o `civic`); um emissor malicioso poderia selar lixo: o titular detecta ao abrir (o compromisso não confere).
- Sem auditoria; cifra feita com `chacha20poly1305` e `p256` (RustCrypto), mas a **construção** é nossa.

## Testes
`teko-wallet` (chaves por tipo, determinismo, passphrase), `teko-crypto/src/seal.rs` (só o destinatário abre, qualquer alteração é recusada, contexto autenticado, proptest), `teko-device-core` (restaurar numa carteira nova dá as mesmas chaves, abrir o selado, recusas), `teko-explorer` (histórico de quem recebe) e **`devnet-sim/tests/e2e.rs` em nós reais**: emitir, "perder" o dispositivo, restaurar pelas 24 palavras num novo, reencontrar a credencial na chain e abrir os atributos. Ver `docs/TEST-MATRIX.md`.

## Decisão
Aceita provisoriamente com: chaves de credencial da carteira; atributos privados cifrados na chain ao titular; identidade e voto fora da seed.
