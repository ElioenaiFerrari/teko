# ADR-0008 — Registro do commitment de eleitor sem vínculo com a identidade

**Status:** Proposto · **PENDENTE** · **Nada implementado** além de `GetVoterCommitment` (stub SHA3, não Poseidon). Nenhum registro existe nesta fase. · **Criticidade: a mais alta da fase**

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
