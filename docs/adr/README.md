# ADRs (Architecture Decision Records)

Formato: `NNNN-titulo-curto.md` com **Status**, **Contexto**, **Alternativas**, **Recomendação**, **Consequências**, **Decisão**.

Status possíveis: `Proposto` · `Aceito` · `Rejeitado` · `Substituído por NNNN`.
Regra: decisões de produto/segurança só passam a `Aceito` pela aprovação do dono do produto. Enquanto isso o campo **Decisão** fica `PENDENTE`.

| # | Título | Status | Decisão |
|---|---|---|---|
| [0001](0001-registrar-decisoes-com-adrs.md) | Registrar decisões com ADRs | Aceito | Aceita (processo) |
| [0002](0002-curva-ed25519-vs-p256.md) | Curva: Ed25519 vs P-256 | Aceito (provisório) | Recomendação adotada |
| [0003](0003-recuperacao-seed-vs-reemissao.md) | Recuperação: seed vs reemissão presencial | Aceito (provisório) | Recomendação adotada |
| [0004](0004-serializacao-do-protocolo.md) | Serialização do protocolo | Aceito (provisório) | Recomendação adotada |
| [0005](0005-credenciais-sd-jwt-vs-bbs.md) | Credenciais: SD-JWT vs BBS+ | Proposto | PENDENTE |
| [0006](0006-nullifier-plume-erc7524.md) | Nullifier determinístico (PLUME/ERC-7524) | Proposto | PENDENTE |
| [0007](0007-atestacao-em-lote.md) | Atestação em lote | Proposto | PENDENTE |
| [0008](0008-registro-de-commitment-sem-vinculo.md) | Registro de commitment sem vínculo com identidade | Aceito provisoriamente (RFC 9474, chave por pleito) | confirmar |
| [0009](0009-interface-do-emulador.md) | Interface do emulador | Aceito (provisório) | Recomendação adotada |
| [0010](0010-transporte-local.md) | Transporte local | Aceito (provisório) | Recomendação adotada |
| [0011](0011-icp-brasil-fora-de-escopo.md) | ICP-Brasil fora de escopo | Proposto | Registro de escopo |
| [0012](0012-hardware-alvo.md) | Hardware alvo | Proposto | Placeholder |
| [0013](0013-pilha-zk-para-a-prova-de-voto.md) | Sistema de prova para o voto | Proposto | PENDENTE (Fase 2) |
| [0014](0014-quadro-de-avisos-e-blockchain.md) | Quadro de avisos / blockchain | Proposto | PENDENTE (Fase 2) |
| [0015](0015-sigilo-e-resistencia-a-coercao.md) | Sigilo e resistência à coerção | Proposto | PENDENTE (Fase 2, crítico) |
| [0016](0016-manifesto-de-eleicao-e-governanca.md) | Manifesto de eleição e governança | Proposto | PENDENTE (Fase 2) |
| [0017](0017-cargos-jurisdicao-e-elegibilidade.md) | Cargos, jurisdição e elegibilidade (vários pleitos) | Proposto | PENDENTE (modelo de dados implementado) |
| [0018](0018-resultados-ao-vivo-e-visibilidade.md) | Resultados em tempo real e visibilidade | Proposto | PENDENTE (demo em andamento) |
