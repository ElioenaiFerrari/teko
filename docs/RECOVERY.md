# Recuperação de conta (sem seed)

**Não existe frase de recuperação.** A chave de identidade nasce no dispositivo e nunca sai dele (princípio 3). Se o dispositivo for perdido, roubado ou trocado:

1. **Bloqueio imediato (opcional):** a autoridade revoga a credencial (`issuer-cli revoke --civic-id ID`). Quem estiver com o aparelho não consegue mais provar atributos a quem confere a lista de revogação, nem registrar-se para votar.
2. **Reemissão presencial:** no dispositivo novo, na tela de emissão da Suite, abra "Perdi ou troquei de dispositivo", informe o **ID Cívico do dispositivo anterior** e o motivo. A autoridade reverifica seus dados, **revoga** a credencial anterior e emite a nova (`issuer-cli reissue`). O ID Cívico muda (a chave é nova).
3. **Sua identidade continua sendo uma só:** a autoridade liga as duas credenciais ao mesmo cidadão. Uma primeira emissão para quem já tem cadastro é recusada.

## E o voto?
- O que você **já votou** conta normalmente.
- Se você **já se registrou** para votar num pleito e perdeu o dispositivo antes de votar, **o registro antigo continua valendo e não se refaz** naquele pleito (decisão do dono do produto). Você vota normalmente nos pleitos em que ainda não se registrou. Motivo: a autoridade nunca vê a chave de eleitor (assinatura cega), então não consegue invalidar a antiga; refazer o registro permitiria voto duplo se o aparelho antigo fosse destravado.
- Não há revoto.

## Limites
- A autoridade é ponto central e de confiança: um funcionário malicioso poderia reemitir indevidamente (mitigação: dados precisam bater com o cadastro; registro auditável; reemissão revoga a anterior e fica no livro).
- O verificador precisa ter a lista de revogação **recente** (o `seq` mostra se está atrasada). O dispositivo perdido não pode ser avisado.
- A lista revela cada revogação (só IDs aleatórios de credencial). Evolução possível: Token Status List com herd privacy.
- Reemissão só presencial: o custo operacional é alto e é a escolha consciente de segurança (ADR-0003).

Veja também: [ADR-0003](adr/0003-recuperacao-seed-vs-reemissao.md), [ADR-0008](adr/0008-registro-de-commitment-sem-vinculo.md).
