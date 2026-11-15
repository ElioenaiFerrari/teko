# Votação cifrada

← [Índice da documentação](../README.md)

## Resumo
A cédula é montada **no dispositivo**: uma cifra por opção (ElGamal exponencial sobre P-256) mais provas de que há exatamente uma opção marcada. Na rede ninguém vê o voto; os validadores só conferem que a cédula é **válida**.

## Quem participa
Dispositivo, Suite, validadores, quadro (blocos).

## Diagramas

![Sequência do voto: o dispositivo cifra e assina, a Suite repassa, os validadores conferem as provas e a cédula entra no bloco](../diagrams/encrypted-ballot.png)

![Anatomia da cédula: chave de eleitor, token do registro, assinatura, cifras por opção, prova 0/1 por opção e prova de soma igual a 1](../diagrams/ballot-anatomy.png)

## Passo a passo
1. A Suite envia `SignBallot{pleito, opção}`; a tela mostra o **nome da opção** (vindo do manifesto) e pede confirmação.
2. O dispositivo cifra cada posição, gera as provas (0 ou 1 por opção; soma = 1) e assina com a chave de eleitor do pleito. A aleatoriedade é apagada da memória.
3. A Suite repassa a cédula ao nó.
4. O nó confere provas, token do registro, assinatura e **voto único**; se válida, entra no próximo bloco.

## O que é verificado e por quem
Todos os validadores: provas de conhecimento zero (Chaum-Pedersen com Fiat-Shamir forte, ligado a eleição, pleito, posição e chave), token e unicidade.

## O que pode dar errado
Cédula com duas opções ou nenhuma (prova inválida), eleitor não registrado, voto repetido, votar após o prazo (o dispositivo recusa e `close-due` fecha).

## Onde está no código
`crates/teko-ballot-crypto` (`elgamal.rs`, `proofs.rs`, `ballot.rs`), `crates/teko-election/src/ballot.rs`, `tools/teko-node`. Decisão: [ADR-0019](../adr/0019-cedulas-cifradas-elgamal-de-limiar.md).

## Testes que cobrem
Unit e proptest em `teko-ballot-crypto`; `tools/teko-node/tests/network.rs`. Medido: 12 opções ≈ 2,4 kB, ~22 ms para cifrar, ~18 ms para verificar.

## Limites honestos
Sem prova ZK de elegibilidade nem *nullifier* (o voto único é checado pela chave de eleitor). Criptografia própria, sem auditoria. Risco "colher agora, decifrar depois" contra computação quântica.
