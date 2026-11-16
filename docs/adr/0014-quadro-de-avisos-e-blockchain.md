# ADR-0014 — Quadro de avisos (blockchain) da votação

**Status:** Aceito **provisoriamente** (2026-11-14): log Merkle atrás de `BulletinBoard` **e** rede federada PoA de teste (`teko-chain`, `teko-node`). Detalhes em [NETWORKS.md](../NETWORKS.md). Confirmação do dono do produto pendente (#16).

## Contexto
Precisamos de um registro **append-only e verificável** para manifestos, raízes da árvore de eleitores, cédulas, nullifiers, provas e resultado.

## Alternativas
| | Cadeia EVM (L2/permissionada) | Cadeia própria (Substrate/Cosmos) | Log transparente Merkle atrás de `trait BulletinBoard` |
|---|---|---|---|
| Parece "blockchain" | Sim | Sim | Parcialmente (sem consenso distribuído) |
| Esforço | Alto | Muito alto | **Baixo** |
| Verificação de prova on-chain | Sim | Sim | Não (verificadores off-chain, independentes) |
| Isola a decisão de cadeia | Não | Não | **Sim** |

## Recomendação
Começar pelo **log Merkle** (`teko-bulletin`, Rust) com provas de inclusão e consistência, atrás de uma `trait` que depois ganha backend EVM. Assim a criptografia e o produto avançam sem travar na escolha de infraestrutura.

## Perguntas
Quem opera o quadro? Como evitar censura/queda na janela de votação? Quantos *witnesses*/espelhos independentes?

> **Atualização (ADR-0034, 2026-11-15):** a rede de **votos** deixou de ser PoA M-de-N e passou a **BFT institucional** (instituições de poder igual, certificado de commit, finalidade, sem blocos vazios). O log Merkle atrás de `BulletinBoard`, o `chain_id` e o perfil `mainnet` travado continuam como abaixo; o texto a seguir descreve o PoA original.

## Decisão provisória (etapa 6)
Rede **federada PoA**: validadores = instituições do M-de-N, M de N assinaturas por bloco, proponente em rodízio com troca de turno, perfis `devnet`/`testnet` e `mainnet` travada, `chain_id` no manifesto assinado e no bloco. Cédulas ainda públicas. EVM permissionada e cadeia própria (Substrate/Cosmos) ficam como alternativas futuras: o `BulletinBoard` e o `chain_id` isolam a escolha. Respostas parciais às perguntas acima: operam o quadro os validadores (não um operador único); queda de um validador não para a rede (view change); witnesses = os próprios validadores e qualquer réplica que verifique.
