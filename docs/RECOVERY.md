# Recuperação: identidade (sem seed) e carteira (com frase)

> **Duas recuperações diferentes.** A **identidade e o voto** não têm seed: recuperam-se por **reemissão presencial** (abaixo). A **carteira** (moeda e bens) tem uma **frase de 24 palavras** própria, de entropia separada: veja [Carteira e frase de recuperação](#carteira-e-frase-de-recuperação) e o [ADR-0023](adr/0023-carteira-com-seed-bip39.md).

**Para a identidade, não existe frase de recuperação.** A chave de identidade nasce no dispositivo e nunca sai dele (princípio 3). Se o dispositivo for perdido, roubado ou trocado:

1. **Bloqueio imediato (opcional):** o órgão **revoga** a credencial **na chain** (quórum, motivo público), pelo app Teko Emissor. Quem estiver com o aparelho não consegue mais provar atributos a quem lê a chain, nem registrar-se para votar.
2. **Reemissão presencial:** no dispositivo novo, a Suite gera o **pedido** e o órgão, no Emissor, **revoga** a credencial anterior (se ainda não o fez) e emite a nova informando o **mesmo registro** (`registry_id`). O ID Cívico muda (a chave é nova).
3. **Sua identidade continua sendo uma só:** o órgão deduplica pelo `registry_id`; a chain não impede por si só duas identidades de chaves diferentes (ADR-0038).

## E o voto?
- O que você **já votou** conta normalmente.
- Se você **já se registrou** para votar num pleito e perdeu o dispositivo antes de votar, **o registro antigo continua valendo e não se refaz** naquele pleito (decisão do dono do produto). Você vota normalmente nos pleitos em que ainda não se registrou. Motivo: a autoridade nunca vê a chave de eleitor (assinatura cega), então não consegue invalidar a antiga; refazer o registro permitiria voto duplo se o aparelho antigo fosse destravado.
- Não há revoto.

## Limites
- A autoridade é ponto central e de confiança: um funcionário malicioso poderia reemitir indevidamente (mitigação: dados precisam bater com o cadastro; registro auditável; reemissão revoga a anterior e fica no livro).
- O verificador precisa **ler a chain** para saber da revogação. O dispositivo perdido não pode ser avisado e continua com a credencial instalada (mas ela não vale mais).
- Veto, revogação e o motivo são **públicos na chain**.
- Reemissão só presencial: o custo operacional é alto e é a escolha consciente de segurança (ADR-0003).

## Carteira e frase de recuperação

- No onboarding o dispositivo gera a carteira, **mostra 24 palavras na tela dele** e confirma que você anotou pedindo **3 posições sorteadas**. Há uma **passphrase** opcional (esquecê-la = perder os fundos).
- Perdeu o dispositivo? No novo, **Restaurar com minha frase**: você digita as 24 palavras **no dispositivo** e confere o endereço mostrado. O endereço é o mesmo de antes; o saldo é do livro, não do aparelho.
- A frase recupera **dinheiro, bens, histórico e credenciais** (CNH, candidatura…): as chaves de credencial vêm da carteira e os atributos privados estão na chain, cifrados a elas ([ADR-0039](adr/0039-recuperar-tudo-pela-carteira.md)). Na Suite, tela **Credenciais → Recuperar tudo**.
- A frase **não** recupera a identidade, o ID Cívico, a credencial cívica nem o registro de voto: esses seguem o processo presencial acima.
- Nunca digite a frase em computador, celular ou site. A Suite não tem campo para ela.

Veja também: [ADR-0003](adr/0003-recuperacao-seed-vs-reemissao.md), [ADR-0008](adr/0008-registro-de-commitment-sem-vinculo.md).
