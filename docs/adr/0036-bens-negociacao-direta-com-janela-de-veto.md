# ADR-0036 — Bens: negociação direta com janela de veto do órgão

**Status:** Aceito provisoriamente (2026-11-16, dono do produto) e **implementado** · Substitui a venda/transferência com três consentimentos do [ADR-0022](0022-bens-registro-com-orgao-e-troca-atomica.md) e o consentimento assinado no dispositivo do [ADR-0025](0025-consentimentos-de-bens-no-dispositivo.md); ajusta o [ADR-0026](0026-bens-na-suite-com-orgao-e-contraparte-simulados.md) (o *relayer* já não paga as transações do usuário).

## Contexto
Até aqui vender ou transferir um bem exigia **três consentimentos numa só transação**: dono, comprador e quórum do órgão. O consentimento do dono saía do dispositivo, mas a transação era assinada por um *relayer* de teste (por isso o explorador mostrava outro endereço em "DE"). O dono do produto quer: (1) dono e comprador negociarem **direto**, sem o órgão em cada transferência; (2) o órgão virar um **guardião**, que pode vetar dentro de uma janela de tempo; (3) a transação sair da **carteira do próprio usuário**.

## Decisão
Uma **máquina de estados fixa** no módulo de bens (`teko-assets`), sem linguagem de condições:

```
(sem oferta) --Offer(dono)--> Oferecida --Accept(comprador)--> Pendente(release_at)
Pendente --Finalize(qualquer um, a partir de release_at)--> transferida
Pendente --Veto(quórum do órgão, antes de release_at)--> cancelada (preço devolvido)
Oferecida --Cancel(dono)--> (sem oferta)
```

1. **Cada parte envia a PRÓPRIA transação**, assinada no dispositivo dela. A assinatura da transação **é** o consentimento: acabam os `Approval` de usuário e a transação conjunta.
2. **Oferta a um comprador específico**, presa à **versão** do bem e ao **hash dos metadados** (ADR-0032). O `Accept` leva os **termos que o comprador viu** (preço, versão e hash): se o dono cancelar e reofertar em outros termos, o aceite antigo **não vale** (testado).
3. **Escrow:** no `Accept` o preço sai do comprador para um endereço derivado do bem (hash de classe+bem, **sem chave**: só o módulo o move). `Finalize` paga o vendedor; `Veto` devolve ao comprador. Tudo atômico (se o pagamento falhar, a transação inteira falha).
4. **A janela é medida em TEMPO**, por classe (`window_secs` no genesis; padrão 1200 s; a devnet usa 20 min): sem blocos vazios (ADR-0034) a altura não avança sozinha. O livro passou a conhecer o **horário do bloco** (`Ctx::time`, o do proponente, já conferido contra o relógio dos validadores). `Finalize` é uma **transação explícita** que qualquer um envia depois da janela; nada age sozinho.
5. **Veto:** o quórum M-de-N da classe assina (intenção presa ao bem, às partes, ao preço, à versão e ao **motivo**, que é público); só vale **dentro** da janela e só em negociação já aceita. Um gravame ou sua liberação durante a janela **cancela** a negociação e devolve o preço (a versão do bem mudou).
6. **Continua por quórum do órgão:** registro do bem, gravames e troca de registradores.
7. **Admissão por execução:** a rede recusa na hora, com o motivo ("veto window still open"), a transação que falharia agora, em vez de deixá-la no mempool até o tempo passar. Só transações que executariam iniciam altura (sem bloco vazio por transação recusada).
8. **Dispositivo:** `SignAssetOffer` e `SignAssetAccept` (substituem `SignAssetConsent`): o dispositivo monta a transação inteira dos campos que mostra (bem, resumo dos metadados, partes, preço, janela, taxa, versão, rede), pede o botão físico e assina; os bytes são os mesmos que o módulo decodifica (`teko_crypto::ledger::asset_offer_payload` e `asset_accept_payload`, conferidos byte a byte contra `Op::encode`).
9. **Removidos:** `Op::Transfer`, `Op::Swap` e `IntentKind::{Transfer,Swap}` (projeto de estudo, sem compatibilidade). Redes de teste antigas precisam ser recriadas.

## Consequências e limites honestos
- O veto só protege contra o que o órgão **sabe e lança** (o problema do oráculo continua): o ledger prova quem consentiu, não que o mundo concorde. Sem validade jurídica.
- O órgão pode vetar de forma abusiva (o motivo é público e fica registrado); **caução e penalidade do órgão** ficam para depois (como no ADR-0035). Hoje o veto de teste é simulado (`make devnet-veto`); um **serviço de órgão** que monitora e veta sozinho está fora desta etapa.
- Cancelar uma oferta ainda não aceita é possível no módulo (`Cancel`), mas **ainda não há mensagem do dispositivo** para isso; na simulação a contraparte aceita na hora.
- Oferta aberta (vitrine) e cancelamento do comprador depois do aceite ficam fora.
- O dono e o comprador pagam **a própria taxa** (a carteira precisa de saldo).
- Sem auditoria; só devnet.

## Testes
`teko-assets/tests/trade.rs` (máquina de estados, escrow, janela, veto, termos do aceite, gravame, conservação por propriedade), `tests/devnet.rs`, `teko-device-core` (telas e assinatura), `teko-client/tests/e2e.rs` (dispositivo real do emulador), `teko-node/tests/economy.rs` (rede real: janela cruzada em tempo real e recusa imediata antes dela) e `devnet-sim/tests/e2e.rs` (veto). Ver `docs/TEST-MATRIX.md`.

## Decisão
Aceita provisoriamente com: oferta a comprador específico; janela por classe (20 min na devnet); veto por quórum M-de-N com motivo público; prazo em tempo de bloco; `Finalize` explícito.
