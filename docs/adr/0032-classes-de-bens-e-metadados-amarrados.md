# ADR-0032 — Classes de bens (imóvel e veículo) e metadados amarrados ao consentimento

**Status:** Aceito (2026-11-15, dono do produto, com as recomendações do assistente). **Implementado:** `teko-crypto::asset_meta`, `teko-assets`, `teko-node`, dispositivo, Suite e explorador.

## Contexto
O modelo de bens ([ADR-0022](0022-bens-registro-com-orgao-e-troca-atomica.md)) já era genérico (classes com órgão M-de-N), mas só existia a classe `imovel`, e o bem guardava um `metadata_hash` que na prática era um texto de teste. Um veículo precisa de dados (placa, RENAVAM, modelo) que não devem ficar públicos, e a pessoa precisa ver **qual** veículo está vendendo.

## Decisão
- **Classes:** a devnet cria `imovel` e `veiculo`, cada uma com **o seu órgão** 2-de-3 (chaves de teste `registrar-<i>.key` e `registrar-veiculo-<i>.key`). O órgão de uma classe não registra a outra.
- **Metadados fora da chain, hash na chain.** O registro é um texto canônico (`campo=valor`, ordem fixa, normalizado, até 512 bytes) definido por um **esquema por classe** em `teko_crypto::asset_meta`, usado por todos (dispositivo, Suite, livro, explorador). A chain guarda só `metadata_hash = H(tag "teko/v2/asset-metadata", classe, registro)`, de **32 bytes** (`Register` recusa outro tamanho). Isso protege os dados do dono e mantém a chain pequena.
  - `veiculo`: chassi (17 caracteres, sem I, O, Q), placa (Mercosul ou antiga), RENAVAM (11 dígitos com dígito verificador), marca, modelo, anos de fabricação e do modelo, cor.
  - `imovel`: matrícula, cartório, endereço, área em m².
- **O `id` do bem na chain** é o chassi (veículo) ou a matrícula (imóvel): único e imutável; a chain impede registrar o mesmo bem duas vezes.
- **O consentimento amarra os metadados.** `Intent::transfer` e `Intent::swap` passam a incluir o `metadata_hash` do bem (campo `extra`, antes vazio), e o livro usa o hash guardado. `SignAssetConsent` ganha o campo `metadata` (o registro): o dispositivo valida o registro, confere que ele nomeia **este** bem, **mostra** o resumo (placa, modelo, ano, cor; matrícula, endereço, área) e assina o hash. Uma Suite que mostre um veículo e troque o registro faz o livro recusar a assinatura.
- **Armazém off-chain** (`MetadataStore`): arquivos `<pasta>/<classe>/<hex do id>.rec`. A venda é bloqueada se o registro não estiver no computador ou não conferir com a chain. O explorador só mostra os metadados em **devnet local** (ou com `--metadata-dir`); um explorador público mostra apenas o hash.
- **Regras só em Rust:** a Suite chama `asset_form` e `asset_register_test`; os erros de campo vêm do esquema compartilhado, sem duplicar regra em JS.

## Consequências
- **Quebra de formato:** hash de 32 bytes e `extra` nas intenções. Uma devnet antiga precisa ser recriada, e o `devnet-reset` **não basta** (ele mantém o `economy.txt` antigo, sem a classe `veiculo`): rode `make devnet-stop`, `make devnet-founder FOUNDER=<seu tekow1…>` (regenera a economia com as duas classes e apaga só a cadeia de economia) e `make devnet`. Quem tentar abrir a cadeia antiga recebe uma mensagem com esses passos.
- O `Consent` do mundo de teste passou a receber um `ConsentRequest` (inclui o registro).

## Limites
- O dispositivo não vê a chain: ele assina o hash do registro que a Suite entrega, e **o livro** confere. A garantia é "ou a assinatura vale para o bem certo, ou é recusada".
- O armazém de dev é local e sem controle de acesso; um armazém de produção (integridade, acesso, retenção) é decisão futura.
- Sem gravames próprios (alienação fiduciária, bloqueio judicial), multas, IPVA ou validação do dígito do chassi. Sem validade jurídica nem auditoria.

## Testes
`teko-crypto` (esquemas, RENAVAM, hash, canônico estrito, proptest), `teko-assets` (hash de 32 bytes, consentimento sobre outro hash recusado, órgão por classe, venda bloqueada sem registro), `teko-device-core` (tela com resumo; registro que não confere é recusado antes de mostrar), `teko-client` (venda de imóvel e de veículo com o dispositivo), `teko-node` (duas classes e registro por 3 nós), `explorer-server` (metadados verificados).
