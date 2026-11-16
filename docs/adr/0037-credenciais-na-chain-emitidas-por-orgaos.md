# ADR-0037 — Credenciais na chain, emitidas por órgãos com janela de contestação e validade

> **Emenda (ADR-0039):** o item 5 mudou. A chave de credencial **por tipo** agora é derivada da **carteira** (`m/7240'/1'/t'`), não da raiz de identidade: volta com a frase de 24 palavras. Segue sem ligação **pública** com o endereço, a identidade e o voto. Os atributos privados passam a ir à chain **cifrados ao titular**.

**Status:** Aceito provisoriamente (2026-11-16, dono do produto) e **implementado (etapa A)** · Complementa o [ADR-0005](0005-credenciais-sd-jwt-vs-bbs.md) (que continua PENDENTE para a credencial de identidade) e segue o padrão do [ADR-0036](0036-bens-negociacao-direta-com-janela-de-veto.md).

## Contexto
Até aqui a única credencial era um blob SD-JWT **fora da chain**, emitido por uma autoridade simulada de chave única e instalado pela Suite. O dono do produto quer permissões e cargos (CNH A, CNH B, "candidato a cargo X no estado Y"…) **sempre na chain**, emitidos por **órgãos e comunidades autorizados**, **fora da Suite do cidadão**, com **janela de contestação de N dias** (como os bens) **e prazo de validade**.

## Decisão
1. **Módulo `teko-credentials` (id 3)** no livro de economia, mesmo padrão do `teko-assets`. Um **tipo** de credencial nasce na gênese (`credtype=<nome> <M> contest=<s> validity=<s> public=<0|1> tekow1…`) e é governado por um **órgão**: M-de-N **endereços de carteira** (o hash da chave, que o dono já tem; a chave pública não precisa estar na gênese).
2. **Emissão por transações de cada membro**, cada uma assinada no dispositivo dele: `Propose` (conta como a 1ª aprovação) → `Approve` dos outros. Ao chegar a M, a credencial fica **Pendente** por `contest` segundos.
3. **O estado é calculado pelo horário do bloco** (`Ctx::time`), sem transação para ativar ou expirar: Proposta → Pendente → **Ativa** → Expirada (`valid_until`). Na janela o quórum pode **Vetar** (motivo público); depois dela só **Revogar** (também por quórum e com motivo público).
4. **Validade por tipo:** `validity=0` é vitalícia (não aceita `valid_until`); caso contrário `valid_until` é obrigatório, no máximo `validity` à frente, e tem de passar da janela.
5. **Privacidade:** o titular é uma **chave de credencial por tipo**, derivada no dispositivo da raiz da identidade (`GetCredentialKey`), **sem ligação** com a identidade, a carteira e a chave de eleitor. Tipos **privados** (CNH) guardam na chain só um **compromisso salgado**; os atributos e o sal ficam com o titular (pacote entregue pelo emissor). Tipos **públicos** (candidato) guardam os atributos em claro, e o compromisso é o hash deles.
6. **O dispositivo monta e assina a transação inteira** (`SignCredentialOp`): recalcula o compromisso e o id da credencial **a partir dos atributos que mostra** (princípio 1); uma votação sobre uma credencial privada sem os atributos mostra "atributos NÃO conferidos" e só o compromisso.
7. **App separado do emissor** (`apps/issuer`, `make issuer`): conecta ao dispositivo plugado, mostra órgãos, emite, aprova, veta e revoga. A **Suite do cidadão só lê** (tela *Credenciais*: gera a chave por tipo e mostra o que foi emitido para ela).
8. **Devnet:** `make devnet ISSUER=tekow1…` (padrão: o `FOUNDER`) põe a carteira do emissor em cada órgão de teste (`cnh-a`, `cnh-b`, `candidato`, 2-de-3), com 2 membros simulados; `make devnet-cosign` aprova com eles.

## Consequências e limites honestos
- O livro prova **quais membros do órgão emitiram**, não que o que atestaram é verdade (problema do oráculo, como nos bens). Sem validade jurídica.
- Quem só vê a chain não vê atributos de tipos privados (a menos que o titular os mostre com o sal). A chave de credencial é estável por tipo: duas credenciais do mesmo tipo do mesmo titular são ligáveis entre si.
- **Ainda não feito:** troca de membros do órgão em execução (só na gênese), caução do órgão por veto/emissão abusivos, o titular provar a posse da chave de credencial a um verificador, e a credencial de identidade cívica, que agora é o tipo `civic` ([ADR-0038](0038-id-civico-como-credencial-na-chain.md)).
- Etapas B (candidatos por credencial e credencial vitalícia de eleito) e C (o eleitor conferir o próprio voto) estão em [ADR-0038/0039, planejados].

## Testes
`teko-credentials/tests/credentials.rs` (ciclo por horário, janela, veto, revogação, validade, privacidade, proptest), `teko-economy` (gênese), `teko-device-core` (telas, recusas e transação assinada no livro real), `teko-client/tests/e2e.rs` (dispositivo real do emulador) e `devnet-sim/tests/e2e.rs` (nós reais: proposta do emissor + cosign). Ver `docs/TEST-MATRIX.md`.

## Decisão
Aceita provisoriamente com: tipos e órgãos na gênese; membros por endereço de carteira; janela de contestação **e** validade por tipo; compromisso salgado para dados privados; app Emissor separado.
