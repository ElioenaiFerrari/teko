# ADR-0026 — Bens na Suite com órgão e contraparte simulados (só desenvolvimento)

> **Atualização (ADR-0036):** o *relayer* só leva as transações do **órgão** simulado (registro, gravame, veto, finalização); as transações do usuário (oferta e aceite) saem da carteira dele, assinadas no dispositivo.

**Status:** Aceito provisoriamente (2026-11-14, continuação do M5, dono do produto). **Implementado** (tela **Bens** da Suite, `teko_assets::devnet`).

## Contexto
Os bens ([ADR-0022](0022-bens-registro-com-orgao-e-troca-atomica.md)) exigem três consentimentos: dono, comprador e o quórum do órgão. O consentimento do cidadão já é assinado no dispositivo ([ADR-0025](0025-consentimentos-de-bens-no-dispositivo.md)). Faltava uma interface para exercitar o fluxo, mas **não existem** órgão registrador nem livro de economia na rede, e há um único dispositivo emulado.

## Decisão
- **Tela "Bens"** na Suite: patrimônio (bens em nome da carteira, versão e gravames), venda e compra com preço e ações do órgão (registrar bem de teste, penhorar e liberar gravame).
- **O consentimento do cidadão é real:** vem do **dispositivo**, que mostra bem, partes, preço e versão e assina após o botão.
- **O resto é simulado, só com `dev-simulation`:** o **órgão registrador** (3 chaves de teste em `dev/registrar-*.key`, quórum 2-de-3), a **contraparte** (outra pessoa de teste, `dev/counterparty.key`) e um *relayer* que paga as taxas das transações do órgão. As chaves de teste são criadas sob demanda em `dev/`.
- **Livro local de teste:** o mesmo `dev/economy.*` do saldo de moeda, agora com os módulos de **moeda e bens** reproduzidos pelo `Ledger` real (`teko_assets::devnet`). A troca é **atômica** (bem e moeda), como no ADR-0022.
- **Build de produção:** os comandos de bens respondem "indisponível" e a tela mostra o aviso; não há órgão nem contraparte simulados na Suite de produção (conferido por `release-check`, que já exige que a autoridade de teste não esteja lá).

## Riscos e limites
- **Tudo é teste:** o "órgão" é a própria máquina do desenvolvedor, então a tela **não demonstra** a confiança no órgão (o problema do oráculo do ADR-0022 continua).
- A contraparte é uma só e automática; **não há negociação entre duas pessoas reais** nem ofertas pendentes. Isso exigiria o livro na rede (M6).
- Uma única classe de bens (`imovel`) e uma única carteira por dispositivo.
- Sem auditoria; sem valor real.

**Atualização (ADR-0032):** a tela Bens registra imóvel ou veículo com formulário validado em Rust e a venda envia o registro de metadados ao dispositivo, que mostra o resumo e assina o hash.
