# ADR-0006 — Nullifier determinístico (PLUME / ERC-7524) e divisão dispositivo/host

**Status:** Proposto · **Decisão: PENDENTE (dono do produto)**

## Contexto
O voto usará uma prova ZK de pertinência ao conjunto de commitments e um nullifier por eleição para impedir voto duplo. O segredo `s` **nunca** deve sair do dispositivo, mas provas ZK completas (Groth16/PLONK) são pesadas para um MCU.

## Alternativas
1. **Dispositivo produz assinatura/nullifier determinístico (PLUME / ERC-7524); o host gera a prova ZK** sobre esse valor. Evita exportar `s` e mantém o dispositivo leve. Exige circuito que aceite o nullifier verificável como entrada pública.
2. **Dispositivo gera a prova ZK completa:** máxima segurança, inviável em MCU pequeno.
3. **Exportar `s` para o host provar:** viola o princípio 3 (rejeitado).

## Recomendação
Alternativa 1. **Nesta fase:** só a interface (`GetVoterCommitment`, `SignBallot`) e stubs em `teko-crypto`. O estudo de PLUME/ERC-7524 (compatibilidade com a curva escolhida no ADR-0002, e com o campo do Poseidon) vira um documento de pesquisa antes da fase de votação.

## Pontos a investigar
- PLUME é definido sobre secp256k1/ECDSA; adaptação para P-256 não é trivial.
- Poseidon opera em campo de curva de ZK (BN254/BLS12-381), diferente de P-256.
- O host que gera a prova vê o nullifier e o commitment: avaliar vazamento de privacidade para o host (princípio 1).

## Consequências
O desenho final do circuito de votação fica para a fase seguinte.
