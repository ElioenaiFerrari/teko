# ADR-0019 — Cédulas cifradas até o fechamento (ElGamal exponencial de limiar + DKG)

**Status:** **Aceito** (2026-11-14, dono do produto, após pesquisa): ElGamal exponencial de limiar sobre P-256, chave por DKG entre os validadores, **sem** time-lock por ora. **Implementado** (etapa 8). Substitui a regra "só da tela" do ADR-0018 por sigilo criptográfico. Resolve a pergunta #14 de `PHASE-2-VOTING.md` §10.

## Contexto
O requisito é que **ninguém veja parciais durante a votação** (para ninguém votar em quem "está ganhando"). Com cédulas públicas no quadro, qualquer pessoa conta; esconder só na tela é convenção. Também não queremos abrir votos individuais depois.

## Decisão
- **Cédula = vetor one-hot de cifrados ElGamal exponencial** em P-256: um por opção, `(rG, mG + rY)`. Provas **disjuntivas de Chaum-Pedersen** de que cada entrada é 0 ou 1 e prova de que a **soma é exatamente 1**. Desafios Fiat-Shamir **fortes**: o hash cobre a chave pública, todos os cifrados e o contexto (eleição, pleito, posição da opção, chave do eleitor), o que evita a falha de Fiat-Shamir fraco de versões antigas do Helios e impede reaproveitar a cédula em outro pleito, eleição ou chave.
- **O dispositivo cifra e prova** (`teko-device-core`): mostra o nome do candidato do manifesto verificado, gera a aleatoriedade com o seu RNG, **zeroiza** e entrega à Suite só os cifrados. O corpo assinado (`BallotBody`) leva o **hash do cifrado** e `option_index = 0`: o host não consegue alterar o voto nem ler a opção na assinatura. A Suite ainda sabe o que o usuário escolheu na própria tela (ela é a interface do usuário); o que ela não pode é trocar o voto.
- **Soma homomórfica:** o `Tallier` verifica as provas de cada cédula, soma os cifrados por opção e mostra só **quantas cédulas foram aceitas**. Nenhum voto individual é decifrado.
- **Resultado:** no fechamento (transição `VotingClosed` assinada), cada validador publica a sua **parte de decifração** de cada soma com uma prova de Chaum-Pedersen (`EntryKind::RESULT`). Com `threshold` partes válidas de validadores distintos, o `Tallier` combina, resolve o log discreto pequeno e **só libera os totais se somarem exatamente o número de cédulas aceitas**. Partes antes do fechamento, duplicadas, de fora, de outra versão do pleito ou com prova errada são recusadas e contadas (`bad_results`). Com um validador fora do ar, 2 de 3 bastam.
- **Chave por DKG** (`teko-ballot-crypto::dkg`, `teko-node dkg`): Feldman VSS com **prova de posse** (Schnorr) de cada participante, entre os processos dos validadores; as partes secretas vão direto entre os nós, e o coordenador só repassa dados públicos. O registro público da cerimônia (`DkgRecord`, `EntryKind::DKG`) é **reverificado por qualquer um** e a chave da eleição precisa ser igual à do manifesto assinado (`ElectionManifest.tally_key`; vazio = cédulas em claro, só desenvolvimento).
- **Serviço do validador:** só decifra um pleito que **a sua própria réplica verificada** mostra encerrado.

## Pesquisa que embasou
ElectionGuard (ElGamal + Chaum-Pedersen + guardiões k-de-n), Helios e Belenios (ElGamal exponencial, provas de 0/1, decifração de limiar), "How Not to Prove Yourself" (Fiat-Shamir fraco), GJKR 1999 e FROST/ChillDKG (DKG com prova de posse), drand/tlock (time-lock, descartado por ora), literatura de receipt-freeness (Helios não é: basta guardar a aleatoriedade).

## Medidas (release, laptop; relatório de `measure_size_and_time`)
| Opções | Bytes da cédula | Cifrar+provar | Verificar |
|---|---|---|---|
| 2 | 452 | ~3,9 ms | ~3,6 ms |
| 12 | 2.392 | ~21,6 ms | ~17,9 ms |
| 16 (máximo) | 3.168 | ~28,7 ms | ~23,6 ms |
Um milhão de cédulas de 12 opções leva ~5 h de verificação em um núcleo (paralelizável/verificação em lote é trabalho futuro). Limites: ≤16 opções por pleito cifrado; 1.000 entradas por bloco.

## O que isto NÃO garante (honestidade)
1. **Conluio de M validadores abre antes do prazo** (2 de 3 na devnet): a decifração só é "honesta" por software. Endurecimento futuro: time-lock (drand) como segunda camada.
2. **Sigilo de longo prazo:** ElGamal em curva elíptica é quebrável por computador quântico futuro; cédulas cifradas e públicas hoje poderiam ser abertas depois ("colher agora, decifrar depois"). Sem alternativa de limiar pós-quântica pronta e auditada; risco aceito.
3. **Viés do DKG de Pedersen** (GJKR 1999): um adversário ativo pode enviesar a distribuição da chave pública, não escolhê-la nem aprendê-la; aceito como Helios/Belenios. A prova de posse impede o ataque de chave maliciosa.
4. **Receipt-freeness só "honest-but-remembering":** a aleatoriedade nasce e morre no dispositivo; um firmware malicioso poderia vazá-la e permitir compra de voto. Mitigação do projeto: firmware M-de-N e build reproduzível.
5. **Canal entre validadores:** no protótipo, loopback sem autenticação nem cifra; numa cerimônia real exige canal autenticado e cifrado entre instituições, difusão consistente e tratamento de reclamações.
6. **Sem ZK de elegibilidade nem nullifier** (etapa à parte); a unicidade vem do registro cego e da chave de eleitor por pleito.
7. **Implementação criptográfica própria, sem auditoria** (`teko-ballot-crypto`, sobre `p256` 0.14): não há crate auditado de ElGamal de limiar para P-256.

## Consequências
`ElectionManifest.tally_key`, `BallotBody.ciphertext_hash`, `SubmittedBallot.encrypted`, `BallotSignature.encrypted`, `EntryKind::DKG`/`RESULT`; o resultado deixa de depender do relógio e passa a depender dos validadores; novo requisito operacional: rodar a cerimônia (`make devnet-dkg`) antes de assinar o manifesto.

> **Atualização (ADR-0044):** num pleito de **várias vagas** (Senador) a prova "soma = 1" vira "o total ponderado está em 1..=N"; com uma vaga ela é a de sempre.
