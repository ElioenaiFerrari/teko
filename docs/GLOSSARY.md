# Glossário

Consulta rápida. Para **entender** cada conceito, com diagrama e código Rust, veja o [guia de conceitos](concepts/README.md).

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
## Conceitos explicados no guia

| Termo | Definição | Capítulo |
|---|---|---|
| **Hash** | Impressão digital de tamanho fixo (SHA3-256, 32 bytes); mudar um bit muda tudo. | [1](concepts/01-hash.md) |
| **Hash com tag** | Hash em que um rótulo versionado (`teko/v1/…`) e os tamanhos entram no cálculo; evita um uso valer em outro. | [1](concepts/01-hash.md) |
| **Separação de domínio** | Princípio por trás da tag: cada finalidade tem o seu rótulo. | [1](concepts/01-hash.md) |
| **ECDSA P-256** | Algoritmo de assinatura do Teko: chave privada assina, pública verifica; 64 bytes. | [2](concepts/02-signatures.md) |
| **bech32m** | Formato de texto de endereços, minúsculo, com checksum que detecta erro de digitação. | [3](concepts/03-addresses-bech32m.md) |
| **HRP** | *Human-readable part*: prefixo legível que diz para que serve o endereço (`teko`, `tekow`, `tekop`). | [3](concepts/03-addresses-bech32m.md) |
| **BIP-39** | Padrão de frase de recuperação (24 palavras = 256 bits + checksum). | [4](concepts/04-wallet.md) |
| **SLIP-10** | Derivação hierárquica de chaves para a curva P-256; caminho `m/7240'/0'/0'/0'/i'`. | [4](concepts/04-wallet.md) |
| **Derivação endurecida** | Índice com apóstrofo: não permite deduzir chaves irmãs. | [4](concepts/04-wallet.md) |
| **TEKO / ypy** | A moeda e sua menor unidade; 1 TEKO = 1.000.000 ypy. | [4](concepts/04-wallet.md) |
| **Limiar / threshold** | M de N para agir. Para assinaturas separadas (M-de-N) ou para juntar partes de uma chave (votação). | [5](concepts/05-m-of-n.md), [9](concepts/09-voting-crypto.md) |
| **Consenso** | Como nós que não confiam uns nos outros concordam sobre os mesmos blocos. | [6](concepts/06-consensus.md) |
| **PoA** | *Proof of authority*: validadores de uma lista fixa em rodízio; bloco vale com M de N assinaturas (livro de votos). | [6](concepts/06-consensus.md) |
| **BFT** | *Byzantine Fault Tolerance*: tolera validadores que mentem, até 1/3 do poder (livro de economia). | [6](concepts/06-consensus.md) |
| **Tendermint** | Estilo de BFT com proposta, prevote, precommit e commit por rodadas. | [6](concepts/06-consensus.md) |
| **Quórum** | Mais de 2/3 do poder de voto; garante que dois blocos não são decididos. | [6](concepts/06-consensus.md) |
| **Lock** | Trava no bloco em que se deu precommit; gravada em disco antes do voto sair. | [6](concepts/06-consensus.md) |
| **Certificado de commit** | Precommits assinados (> 2/3) que provam a decisão; qualquer um verifica. | [6](concepts/06-consensus.md) |
| **Equivocação / slash** | Dois votos conflitantes do mesmo validador; vira evidência e queima parte do stake. | [6](concepts/06-consensus.md) |
| **WAL** | *Write-ahead log*: o que o nó grava antes de votar para não repetir ou esquecer após reiniciar. | [6](concepts/06-consensus.md) |
| **Nonce (de transação)** | Contador por endereço; só a transação com o próximo número vale (impede replay). | [7](concepts/07-ledger.md) |
| **Raiz do estado** | Hash que resume todo o estado após um bloco; nós honestos chegam à mesma. | [7](concepts/07-ledger.md) |
| **Árvore de Merkle** | Resume uma lista em um hash e permite provar que um item está nela. | [7](concepts/07-ledger.md) |
| **Halving** | Recompensa de bloco cortada pela metade a cada intervalo (210.000 blocos); garante o teto. | [8](concepts/08-coin.md) |
| **Stake** | Moedas travadas como garantia para validar; dá poder de voto e pode ser penalizado. | [8](concepts/08-coin.md) |
| **Fase de fundador** | No começo só o fundador valida e recebe as recompensas; termina quando o stake total chega a N. | [8](concepts/08-coin.md) |
| **ElGamal exponencial** | Cifra em que multiplicar cifras soma os votos sem decifrar. | [9](concepts/09-voting-crypto.md) |
| **Shamir** | Divide um segredo em partes; M delas o reconstroem. | [9](concepts/09-voting-crypto.md) |
| **DKG (Feldman)** | Geração distribuída de chave: ninguém a conhece inteira; as partes são verificáveis. | [9](concepts/09-voting-crypto.md) |
| **Prova de conhecimento zero** | Prova que algo é verdade (voto é 0 ou 1) sem revelar o quê. | [9](concepts/09-voting-crypto.md) |
| **Assinatura cega (RFC 9474)** | O órgão assina sem ver a mensagem; base do registro anônimo. | [9](concepts/09-voting-crypto.md) |
| **Tempo constante** | Comparação cujo tempo não depende de onde erra (PIN). | [10](concepts/10-device-security.md) |
| **zeroize** | Zerar a memória de um segredo ao descartá-lo. | [10](concepts/10-device-security.md) |
| **Explorador** | Página e API somente leitura para ver blocos, transações e carteiras; verifica a cadeia sozinho, sem confiar nos nós. | [fluxo 15](flows/15-explorer.md) |
| **Recibo / evento** | O que uma transação fez ao executar (por exemplo `slashed`); vem do `BlockOutcome` do livro. | [7](concepts/07-ledger.md) |
| **Classe de bem** | Tipo de bem (`imovel`, `veiculo`) com o seu órgão registrador M-de-N e o seu esquema de metadados. | [fluxo 12](flows/12-ledger-coin-and-assets.md) |
| **Metadados do bem** | Dados do bem (placa, RENAVAM, endereço) guardados fora da chain; só o hash vai à chain e entra no consentimento de venda. | [fluxo 12](flows/12-ledger-coin-and-assets.md) |
| **Chassi / RENAVAM** | Chassi: identificador único do veículo (é o `id` do bem). RENAVAM: registro nacional, 11 dígitos com dígito verificador módulo 11. | [fluxo 12](flows/12-ledger-coin-and-assets.md) |
