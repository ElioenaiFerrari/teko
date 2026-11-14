# ADR-0022 — Bens: registro com órgão M-de-N, consentimentos e troca atômica (`teko-assets`)

**Status:** Aceito provisoriamente (2026-11-14): o dono do produto aprovou "dono + comprador + órgão" para transferir bens e dois livros separados. **Implementado** como biblioteca (M4); ainda não roda na rede nem na Suite.

## Decisões
- **Classes de bens** (por exemplo `imovel`, `veiculo`) nascem na gênese, cada uma governada por um **órgão registrador**: um conjunto de chaves públicas com limiar M-de-N. A troca do conjunto (`UpdateRegistrars`) exige o **quórum atual**.
- **Registro inicial** do bem: só com o quórum do órgão.
- **Transferência** (`Transfer`) exige **três consentimentos**: o dono atual, o comprador e o quórum do órgão. Quem envia a transação pode ser qualquer um (um *relayer*), porque o que vale são as assinaturas dos consentimentos.
- **Gravames** (penhora, hipoteca...) são colocados e retirados só pelo quórum do órgão e **bloqueiam** a transferência enquanto existirem.
- **Troca atômica** (`Swap`, venda contra pagamento): os mesmos três consentimentos, mais o **preço**; o bem passa ao comprador e o comprador paga ao vendedor na moeda **na mesma transação** (`Ctx::call_as` do livro). Se faltar saldo, nada acontece.
- **Consentimento = assinatura de uma intenção**: o hash cobre rede, tipo de ação, classe, bem, partes, **preço** e a **versão** do bem. Por isso: uma aprovação de `Transfer` não vale como `Swap`; assinar por 3000 não autoriza 1; e depois de qualquer mudança do bem (versão +1) as aprovações antigas morrem (sem repetição).
- **`Ctx::call_as`** (no `teko-ledger`): um módulo chama outro, agindo em nome de um endereço. Só é seguro porque o módulo chamador **confere antes** a assinatura desse endereço. Módulos são código de consenso escolhido pelos operadores da rede; a profundidade é limitada.

## Limites (honestos)
- **Problema do oráculo:** o livro prova *quem consentiu*, não que o mundo físico concorde. O órgão é uma instituição confiável; o M-de-N e o histórico público só o tornam **responsável**, não incorruptível. Pilotos de registro em blockchain (Geórgia, Suécia) esbarraram nisso.
- Sem validade jurídica; sem ligação com cartórios, ONR ou SREI; metadados do bem são só um hash.
- Não há ainda transferência por herança, usufruto, fração do bem ou leilão.
- Quem ocupa cada chave de órgão no mundo real (e a custódia dessas chaves) é decisão institucional fora do código.
