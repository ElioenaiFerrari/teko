# Glossário

| Termo | Definição |
|---|---|
| **Teko** | Nome do projeto (guarani: "modo de ser", identidade). |
| **Dispositivo** | Cartão/USB que guarda chaves e credenciais. Nesta fase: emulador. |
| **Suite** | App desktop (Tauri v2) que conversa com o dispositivo. Ambiente não confiável. |
| **Emulador** | Processo separado que simula tela e botões do dispositivo. É onde PIN e confirmações acontecem. |
| **Autoridade / Issuer** | Entidade que verifica a pessoa fora do sistema e emite credenciais. Simulada pelo `issuer-cli`. |
| **Chave de identidade** | Par de chaves de assinatura identificável, vinculado à pessoa pela credencial. |
| **ID Cívico** | Identificador público derivado da chave de identidade (`teko1…`, bech32m). Não é RG nem CPF. |
| **ID pairwise** | Identificador por serviço, `H(segredo, id_do_serviço)`; impede correlação entre serviços. |
| **Segredo de eleitor (`s`)** | Segredo isolado das demais chaves; gera commitment `C` e nullifier `N`. |
| **Commitment (`C`)** | `Poseidon(s)`; registrado por canal separado, sem vínculo com identidade. |
| **Nullifier (`N`)** | Valor determinístico por eleição que impede voto duplo sem revelar o eleitor. |
| **Credencial de atributos** | Dados pessoais assinados pela autoridade com divulgação seletiva. |
| **Divulgação seletiva** | Provar um predicado ("maior de 18") sem revelar o dado (data de nascimento). |
| **Atestação** | Prova de que o dispositivo é genuíno, via desafio-resposta e cadeia de certificados até a raiz da autoridade. |
| **Chave de lote** | Chave de atestação compartilhada por um lote grande de dispositivos (estilo FIDO batch attestation), para evitar rastreamento. |
| **Manifesto de firmware** | Metadados assinados: versão, hash, assinaturas M-de-N. |
| **M-de-N** | Exigência de M assinaturas válidas de N chaves de instituições independentes. |
| **Bootloader** | Código imutável que verifica o manifesto e impede downgrade antes de executar o firmware. |
| **Pairwise subject** | Conceito do OIDC/WebAuthn: identificador distinto por parte confiável. |
| **PLUME / ERC-7524** | Esquemas de nullifier/assinatura determinística, estudados para o segredo de eleitor. |
| **SD-JWT / BBS+** | Formatos de credencial com divulgação seletiva (ver ADR-0005). |
| **HAL** | Camada de abstração do hardware (traits `Display`, `Buttons`, `SecureStorage`, `Rng`, `Clock`). |
| **Transport** | Trait que abstrai o canal Suite↔dispositivo (TCP/socket agora, USB HID depois). |
| **ButtonRequest** | Mensagem do dispositivo informando que aguarda ação física do usuário. |
| **Wipe** | Apagamento seguro de todo o estado do dispositivo. |
| **ADR** | Architecture Decision Record, em `docs/adr/`. |
| **TEKO** | A moeda. 1 TEKO = 1.000.000 ypy; mostrada em pt-BR com até 6 casas (`1.234,5 TEKO`). ADR-0029. |
| **ypy** | Menor unidade da moeda (tupi: começo, origem). O livro guarda inteiros em ypy; 1 ypy = 0,000001 TEKO. |
