# ADR-0008 — Registro do commitment de eleitor sem vínculo com a identidade

> **Atualização (ADR-0038):** o registro confere o **pacote da credencial cívica** e que ela está **ativa na chain**; o cidadão é identificado pelo `registry_id` (o mesmo numa reemissão), não por uma `citizens.log` da autoridade.

**Status:** Aceito **provisoriamente** (2026-11-14): assinatura cega RFC 9474, uma chave RSA por pleito. **Implementado** (etapa 5). A decisão formal do dono do produto continua pendente (§10 #2 de `PHASE-2-VOTING.md`). · **Criticidade: a mais alta da fase**

## Implementação (etapa 5)

- **Esquema:** RSABSSA-SHA384-PSS-Randomized (RFC 9474), RSA-2048, **uma chave por pleito** (`Contest.registration_key`, SPKI no manifesto assinado M-de-N; vazio = pleito não exige registro). Um registro de um pleito não vale em outro.
- **Implementação própria** em `crates/teko-registration`, sobre `rsa` 0.10.0, validada contra o **vetor oficial do Apêndice A.1 do RFC** (cada valor intermediário reproduzido). Motivo: `blind-rsa-signatures` 0.18 não compila com `rsa` 0.10.0 final, e a versão rc reabre o RUSTSEC-2023-0071. **Não foi auditada por terceiros.**
- **O dispositivo calcula a mensagem cega** (`BeginRegistration`): um host malicioso não consegue registrar uma chave de sua escolha. O cegamento é **determinístico** a partir de um segredo do dispositivo (`tagged_hash(REGISTRATION_BLIND, voter_root, eleição, pleito)`), então não há estado pendente e uma resposta perdida se refaz com a mesma mensagem cega.
- **Autoridade** (`issuer-cli`, `Issuer::sign_registration`) confere: credencial dela e não expirada; chave de identidade = a da credencial; prova de posse **deste pedido** (assina eleição, pleito e mensagem cega); domicílio cobre o pleito; fase `RegistrationOpen`/`VotingOpen`; **uma vez por (cidadão, pleito)** (repetir a MESMA mensagem cega é re-assinado). O livro da autoridade guarda `pleito, ID cívico, hash da mensagem cega`: nunca uma chave de eleitor.
- **Registro público:** `(voter_key, token)` vai ao quadro (`EntryKind::REGISTRATION`) **sem nada do diálogo com a autoridade**. A apuração só conta a cédula cuja chave tem token válido **para aquele pleito**, registrado antes na ordem do log; sem verificador instalado, falha fechado.
- **Ainda não coberto:** correlação por tempo/rede entre o pedido à autoridade e a publicação (o protótipo publica na hora, no mesmo processo); contagem pública de assinaturas emitidas vs. elegíveis (só existe o livro local); prova ZK de elegibilidade (opção C) e nullifier (ADR-0006/0013); revogação de `pk_v` em reemissão.

> **Atualização (Fase 2, 2026-11-14):** o que se registra passa a ser `pk_v` (chave pública de eleitor **por eleição**), não `C = Poseidon(s)`. Recomendação atual: **assinatura cega RFC 9474 (opção A)** como linha de base, com **contagem pública de assinaturas emitidas vs. elegíveis** contra *stuffing*, e prova ZK de elegibilidade (opção C) como evolução. Mitigar correlação por tempo/rede com atraso aleatório e canais separados. Detalhes e tabela comparativa: `docs/PHASE-2-VOTING.md` §4.2.

## Contexto
A autoridade precisa aceitar `C = Poseidon(s)` somente de cidadãos elegíveis, mas **sem saber qual `C` pertence a qual cidadão** (princípio 5). Um registro ingênuo — "o cidadão X, autenticado, envia C" — vincula identidade e voto por construção.

## Alternativas
1. **Assinatura cega (blind signature):** o dispositivo cega `C`, a autoridade assina o valor cego após autenticar o cidadão (marcando "já emitido"), o dispositivo remove o cegamento e apresenta `(C, assinatura)` por canal anônimo. A autoridade nunca vê `C`. *Prós:* simples, maduro (RSA blind / BlindBBS). *Contras:* assinatura cega com a curva escolhida; ataque por timing se o envio for imediato.
2. **Credencial anônima / token (ex.: Privacy Pass / BBS+):** mesma ideia com esquemas modernos.
3. **Prova ZK de pertinência a um conjunto de elegíveis:** o cidadão prova "sou elegível" sem revelar quem; um registro posterior aceita `C`. Mais forte, mais complexo.
4. **Registro ingênuo (só para teste):** vincula; rotular **NÃO PRIVADO**.

## Recomendação
Para a fase de estudo: **não implementar o registro** nesta fase; manter a interface (`GetVoterCommitment`) e o canal separado como conceito. Para a fase de votação: avaliar (1) assinatura cega como linha de base e (3) como evolução. Em qualquer caso: canal/sessão distintos, sem ID Cívico, atraso aleatório, nenhuma resposta da autoridade que contenha identidade.

## Riscos a documentar
- Correlação por tempo/IP entre emissão da identidade e registro do commitment.
- Autoridade que emite assinaturas cegas para não-eleitores (stuffing): exige contagem pública de assinaturas emitidas vs. elegíveis.
- Reemissão (ADR-0003): como revogar um `C` antigo sem saber de quem é.

## Consequências
Define o desenho de toda a fase de votação; deve ser decidido antes dela, não antes da Suite.

## Reemissão e registro (etapa 7)
O livro da autoridade passou a guardar o registro por **cidadão** (`citizen_ref`, estável entre reemissões), não por chave de identidade. Efeito: um cidadão reemitido **não consegue registrar de novo** no pleito em que já se registrou (decisão B do dono do produto; alternativas A/C descartadas em ADR-0003). A credencial revogada também não registra.
