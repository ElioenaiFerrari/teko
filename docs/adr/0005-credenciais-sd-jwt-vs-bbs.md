# ADR-0005 — Credenciais com divulgação seletiva: SD-JWT vs BBS+

**Status:** Proposto · PENDENTE. **Implementado como protótipo:** modelo simplificado estilo SD-JWT (digests de claims salgadas assinados pela autoridade; claims derivadas `age_over_N`). Sem unlinkability.

## Contexto
Provar "maior de 18" sem revelar a data de nascimento. O dispositivo guarda a credencial e assina a apresentação.

## Alternativas
| | SD-JWT (VC) | BBS+ |
|---|---|---|
| Maturidade/padrão | Padrão IETF (SD-JWT; confirmar número/status da RFC), amplo uso (OpenID4VC, eIDAS ARF) | Em padronização (IETF/W3C), bibliotecas menos maduras |
| Curva | Qualquer assinatura (P-256 `ES256`) | BLS12-381 (curva própria; pouco suporte em SE/MCU) |
| Divulgação seletiva | Sim, por revelar/ocultar *claims* | Sim |
| **Unlinkability** (apresentações não correlacionáveis) | **Não nativa**: assinatura e hashes se repetem; mitigar com credenciais de uso único/lote | **Sim** (provas ZK randomizadas) |
| **Predicados** (idade ≥ 18 sem revelar a data) | Não nativo: exige claim derivado (`over_18: true`) emitido pela autoridade | Possível com extensões/ZK |
| Peso no dispositivo | Baixo (verificar/assinar) | Alto (pareamentos; ZK no host ajuda) |

## Recomendação
**SD-JWT agora**, com claims derivados emitidos pela autoridade (`age_over_18`) para os predicados, **ciente da falta de unlinkability** (registrar no THREAT-MODEL, H7). **BBS+** como evolução quando houver suporte de hardware/bibliotecas. Esconder o formato atrás de um trait `CredentialScheme` em `teko-crypto`.

## Consequências
Com SD-JWT, um verificador que vê duas apresentações da mesma credencial pode correlacioná-las; mitigação por credenciais descartáveis ou por BBS+ depois.
