# ADR-0003 — Recuperação: seed phrase vs reemissão presencial

> **Atualização (ADR-0038):** a reemissão presencial agora é **na chain**: o órgão **revoga** a credencial antiga (quórum, motivo público) e emite a nova com o **mesmo `registry_id`**; não há mais lista de revogação por chave única nem `issuer-cli reissue`.

> **Atualização (ADR-0023):** esta decisão continua valendo para a **identidade e o voto**. A **carteira** (moeda e bens) tem frase de recuperação própria, com entropia separada. Veja o [ADR-0023](0023-carteira-com-seed-bip39.md).

**Status:** **Aceito** (2026-11-14, dono do produto, após o debate de BIP-39): **sem seed/mnemônico**; recuperação = **reemissão presencial com revogação**. Implementado na etapa 7 (ver [RECOVERY.md](../RECOVERY.md)). Revisável.

## Contexto
Se o dispositivo for perdido, quebrado ou apagado (N erros de PIN), como o cidadão recupera a identidade? A tendência do dono do produto é **sem seed**.

## Alternativas
| | Seed phrase | Reemissão presencial (sem seed) |
|---|---|---|
| Conveniência | Alta | Baixa (deslocamento à autoridade) |
| Venda/compra de voto | **Seed é ativo vendável**: quem a compra passa a ser o eleitor | Dispositivo físico é vendável, mas a venda é detectável por revogação e reemissão |
| Superfície de ataque | Seed em papel, fotos, phishing | Autoridade passa a ser ponto central |
| Segredo de eleitor | Precisa ser derivável da seed (quebra isolamento) ou fica sem backup | `s` novo na reemissão (commitment antigo revogado) |
| Revogação do antigo | Difícil (seed clonável) | Natural |
| Custo operacional | Baixo | Alto |

## Recomendação
**Sem seed**: reemissão presencial, revogando o dispositivo/credencial antigo e o commitment de eleitor antigo. Registrar o custo operacional e a dependência da autoridade (THREAT-MODEL §5, X1).

## Perguntas
- Como revogar o commitment sem vincular a identidade (interação com ADR-0008)?
- Prazo máximo de reemissão; o que acontece com um voto já emitido?
- A reemissão gera nova chave de identidade e novo ID Cívico ou preserva o ID? (KEY-HIERARCHY, pergunta 5)

## Consequências
Sem seed, a perda total do dispositivo exige processo presencial; elimina o vetor "venda de seed".

## Decisão e implementação (etapa 7)
- **Sem seed.** Um mnemônico tornaria a identidade clonável sem rastro, quebraria o isolamento identidade↔voto (princípio 5) e criaria um ativo vendável (ADR-0015). A garantia mais forte do desenho é que a chave só existe naquele dispositivo.
- **Reemissão presencial** (`issuer-cli reissue`): a autoridade reverifica a pessoa (os dados precisam bater com o cadastro original), **revoga toda credencial ativa daquele cidadão** e emite uma nova para o dispositivo novo, ligada ao **mesmo cidadão** (`citizen_ref`). Uma primeira emissão para quem já tem cadastro é **recusada** (senão uma pessoa teria duas identidades).
- **Revogação imediata** (`issuer-cli revoke`) para dispositivo roubado, sem precisar de dispositivo novo.
- **Lista de revogação assinada** (`teko_crypto::credential::RevocationList`): só IDs de credencial aleatórios, `seq` crescente, assinada pela raiz da autoridade. Quem verifica prova de atributo (`teko-client`) recusa credencial revogada; o dispositivo perdido não pode ser avisado, então a checagem fica no verificador e na autoridade de registro.
- **Registro de eleitor já feito** (decisão do dono do produto, opção B): **o registro antigo continua valendo e não se refaz** naquele pleito (a autoridade guarda o registro por **cidadão**, não por chave de dispositivo). Quem perde o dispositivo depois de registrar perde o voto naquele pleito, mas vota nos demais. Evita voto duplo sem a autoridade conhecer a chave de eleitor. Alternativas descartadas: novo registro (risco de voto duplo se o aparelho antigo for destravado) e "código de revogação" (reintroduz um segredo de backup).
- **Novo ID Cívico:** a reemissão gera chave nova, logo ID Cívico novo (KEY-HIERARCHY, pergunta 5).

## Pesquisa que embasou a decisão
- **Status/revogação em larga escala:** listas de bits por índice (W3C Bitstring Status List; IETF Token Status List, adotada pela arquitetura da EUDI Wallet) escondem *qual* credencial se consulta ao baixar a lista inteira (herd privacy), mas revelam cada revogação; acumuladores/ZK são mais privados e bem mais complexos. Evolução natural daqui: Token Status List.
- **Carteira perdida (EUDI/ARF):** revoga-se a atestação da carteira, a revogação é irreversível e o emissor reverifica antes de reemitir.
- **NIST SP 800-63B-4 (2025):** recuperação por nova prova de identidade é válida quando a conta nasceu com prova presencial; perguntas de segurança não valem.
- **Revoto (Estônia):** vale o último voto contra coerção, mas metadados de revoto vazam e há resultado de 2026 sugerindo que revoto sem segredos pré-combinados não dá resistência robusta. Por isso **não** há revoto aqui.
