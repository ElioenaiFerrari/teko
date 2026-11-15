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
| [0018](0018-resultados-ao-vivo-e-visibilidade.md) | Resultados em tempo real e visibilidade | Proposto | Superado em parte pelo 0019 (parciais escondidos por cifra) |
| [0019](0019-cedulas-cifradas-elgamal-de-limiar.md) | Cédulas cifradas até o fechamento (ElGamal de limiar + DKG) | Aceito (etapa 8) | confirmar |
| [0020](0020-livro-razao-generico-e-modulos.md) | Livro-razão genérico com módulos (`teko-ledger`) | Aceito provisoriamente (M2) | confirmar |
| [0021](0021-moeda-teto-fixo-stake-e-taxas.md) | Moeda de teto fixo, stake, taxas e penalidades (`teko-coin`) | Aceito provisoriamente (M3) | **Opção B decidida** (ADR-0028) |
| [0022](0022-bens-registro-com-orgao-e-troca-atomica.md) | Bens: registro com órgão M-de-N, consentimentos e troca atômica (`teko-assets`) | Aceito provisoriamente (M4) | confirmar |
| [0023](0023-carteira-com-seed-bip39.md) | Carteira com frase de recuperação (BIP-39 + SLIP-10 P-256), separada da identidade | **Aceito** (M5a); supera em parte o 0003 | — |
| [0024](0024-assinar-transferencias-no-dispositivo.md) | Assinar transferências de moeda no dispositivo (`SignTransfer`) | Aceito provisoriamente (M5b) | confirmar |
| [0025](0025-consentimentos-de-bens-no-dispositivo.md) | Consentimentos de bens assinados no dispositivo (`SignAssetConsent`) | Aceito provisoriamente (M5c) | confirmar |
| [0026](0026-bens-na-suite-com-orgao-e-contraparte-simulados.md) | Bens na Suite com órgão e contraparte simulados (só desenvolvimento) | Aceito provisoriamente | confirmar |
| [0027](0027-stake-assinado-no-dispositivo.md) | Stake assinado no dispositivo e tela Validadores | Aceito provisoriamente | confirmar (início da rede: ADR-0021) |
| [0028](0028-economia-sobre-bft-e-fase-de-fundador.md) | Economia sobre consenso BFT, validadores por stake e fase de fundador | Aceito provisoriamente (M6a) | confirmar |
| [0029](0029-unidade-teko-e-ypy.md) | Unidade da moeda: TEKO e ypy (6 casas decimais) | Aceito | — |
