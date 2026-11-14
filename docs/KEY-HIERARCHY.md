# Hierarquia de chaves

Status: **proposta para revisão**. Curvas e recuperação dependem de ADR-0002 e ADR-0003 (pendentes). Onde há dependência, a curva é escrita como `Curve` (Ed25519 ou P-256).

## 1. Princípios

- Chaves nascem no dispositivo (RNG do dispositivo) e **nunca saem**.
- **Derivação por finalidade com separação de domínio:** cada uso tem uma tag única e versionada; saídas de um domínio nunca são válidas em outro.
- **Isolamento identidade ↔ voto:** o segredo de eleitor não é derivável da identidade nem vice-versa.

## 2. Tags de domínio

Formato: `"teko/v1/<finalidade>"`, codificadas com prefixo de comprimento antes de entrar em qualquer hash/KDF.

| Tag | Uso |
|---|---|
| `teko/v1/identity` | Derivação da chave de identidade |
| `teko/v1/civic-id` | Hash que gera o ID Cívico |
| `teko/v1/pairwise` | Chave/IDs pairwise |
| `teko/v1/voter` | Segredo de eleitor (se derivado; ver pergunta 1) |
| `teko/v1/doc-sign` | Prefixo do que é assinado em `SignDocument` |
| `teko/v1/attr-proof` | Provas de atributos |
| `teko/v1/attestation` | Desafio de atestação |
| `teko/v1/fw-manifest` | Assinatura do manifesto de firmware |

Primitivas propostas: `SHA3-256` para hashes etiquetados; `KMAC256` (ou HKDF-SHA3) para derivação. Decisão final no ADR-0002/etapa 2.

## 3. Raiz

Semente interna `K_root` (32 bytes) gerada por `Rng` na inicialização. Armazenada apenas em `SecureStorage`, `zeroize` ao sair de escopo. **Nunca exportada.** Existência de backup depende do ADR-0003 (recomendação: sem backup / sem seed).

```
K_root ──KMAC(tag=identity)──▶ sk_identity ──▶ pk_identity
       ──KMAC(tag=pairwise)──▶ k_pairwise
s (segredo de eleitor) ◀── entropia SEPARADA (recomendação), não deriva de K_root
```

## 4. Chave de identidade

- Par `Curve`, assinatura identificável. A credencial emitida pela autoridade vincula `pk_identity` ao cidadão.
- Usada para: `SignDocument`, prova de posse na emissão, assinatura de provas de atributos.
- Nunca usada para votar.

## 5. ID Cívico

```
digest   = SHA3-256( len(tag) ‖ "teko/v1/civic-id" ‖ version(0x01) ‖ pk_identity )
payload  = digest[0..20]                       // 160 bits
civic_id = bech32m( hrp = "teko", data = version ‖ payload )
```

- Exemplo de formato: `teko1qxy…` (~40 caracteres), checksum bech32m (detecta erros de digitação com alta probabilidade; até 4 erros garantidos no comprimento típico).
- **Não é RG nem CPF**; não contém dado pessoal.
- Versão no payload permite migrar o algoritmo.
- Trade-off do truncamento em 160 bits: resistência a colisão ≈ 2^80 (adequada para identificador, não para commitment forte).

## 6. IDs por serviço (pairwise)

```
pid = KMAC256( key = k_pairwise, data = len(service_id) ‖ service_id, tag = "teko/v1/pairwise" )
pairwise_id = bech32m( hrp = "tekop", data = version ‖ pid[0..20] )
```

- Serviços diferentes obtêm IDs sem relação computável entre si sem `k_pairwise`.
- `service_id` canônico (ex.: origem normalizada) para impedir que variações do mesmo serviço gerem IDs diferentes — pergunta 3.
- Limitação: se o serviço também recebe o ID Cívico ou atributos reveladores, pode correlacionar por outros meios.

## 7. Segredo de eleitor

- `s` (32 bytes, ou elemento do campo do Poseidon) — **isolado** das demais chaves.
- Commitment `C = Poseidon(s)`; nullifier por eleição `N = Poseidon(s, election_id)` ou equivalente derivado por PLUME/ERC-7524 (ver ADR-0006).
- **Divisão dispositivo/host:** o dispositivo produz um **nullifier/assinatura determinística**; a prova ZK é gerada no host sobre esse valor, sem que `s` saia do dispositivo.
- **Nesta fase:** interface (`GetVoterCommitment`, `SignBallot`) e **stub**; Poseidon e ZK reais ficam para a fase de votação.
- O dispositivo nunca emite `C` junto de `pk_identity` na mesma resposta; o registro de `C` ocorre por canal separado (ADR-0008).

## 8. Credenciais de atributos

- Estrutura: `{ subject_key = pk_identity, atributos, emissor, validade, id_revogação }`, assinada pela autoridade.
- **Divulgação seletiva**: provar predicados (ex.: `idade >= 18`) sem revelar o valor. Formato em ADR-0005 (SD-JWT vs BBS+).
- Armazenada no dispositivo; apresentação é assinada pela chave de identidade com nonce do verificador.
- Nesta fase: credenciais e prova simuladas pelo stub em `teko-crypto`.

## 9. Chaves fora do dispositivo do cidadão

| Chave | Quem detém | Observação |
|---|---|---|
| Raiz da autoridade (credenciais) | Autoridade, offline | Assina credenciais e certificados de lote |
| Chaves de lote de atestação | Fabricante/autoridade | Uma chave por lote grande (ADR-0007); a chave de lote assina o certificado de atestação do dispositivo |
| Chaves de assinatura de firmware | N instituições independentes | Manifesto válido com ≥ M assinaturas distintas |

## 10. Tabela-resumo

| Chave | Curva | Nasce | Vive | Quem vê | Exportável | Zeroize |
|---|---|---|---|---|---|---|
| `K_root` | — | Dispositivo | SecureStorage | Ninguém | Não | Sim |
| `sk_identity` | `Curve` | Dispositivo | derivada/armazenada | Ninguém | Não | Sim |
| `pk_identity` | `Curve` | Derivada | Pública | Todos | Sim | — |
| `k_pairwise` | — | Derivada | Dispositivo | Ninguém | Não | Sim |
| `s` (eleitor) | campo Poseidon | Dispositivo (entropia separada) | SecureStorage | Ninguém | Não | Sim |
| `C` | — | Derivado de `s` | Registro (canal separado) | Público | Sim | — |
| Chave de lote | `Curve` | Fabricante | HSM da autoridade | Fabricante | Não | — |
| Chaves de firmware | `Curve` | Instituições | HSM/offline | Instituição | Não | — |
| PIN | — | Usuário | Só hash+sal+KDF | Ninguém | Não | Sim |

## 11. Perguntas em aberto

1. **`s` com entropia separada ou derivado de ramo independente de `K_root`?** Entropia separada garante que `s` nunca seja recomputável da identidade, mas torna `s` não recuperável por qualquer mecanismo de seed. Derivado é recuperável, mas aumenta a dependência de `K_root`. *Recomendação:* entropia separada (coerente com "sem seed"). **PENDENTE (dono do produto)**.
2. **KMAC256 vs HKDF-SHA3 vs HKDF-SHA256.** *Recomendação:* KMAC256 (uma família, SHA-3 nativo, domínio por customização). **PENDENTE** (técnica).
3. **Canonicalização do `service_id`** (origem web, URL, URN?). *Recomendação:* string UTF-8 normalizada, definida no DEVICE-PROTOCOL. **PENDENTE**.
4. **HRP** `teko` (ID Cívico) e `tekop` (pairwise): aceitar ou distinguir também por versão? **PENDENTE**.
5. **Chave de identidade por dispositivo ou por cidadão?** Relacionada à reemissão/revogação (ADR-0003). **PENDENTE**.
6. **Curva** → ADR-0002; **formato de credencial** → ADR-0005; **nullifier** → ADR-0006. **PENDENTE**.
