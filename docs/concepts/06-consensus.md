# 6. Consenso: o motor BFT e suas duas cadeias

## O que é
**Consenso** é como vários computadores, que não confiam uns nos outros, **concordam sobre a mesma sequência de blocos**, mesmo com falhas ou traidores. Sem isso, cada um teria um livro diferente.

## Qual algoritmo o Teko usa
O **mesmo motor BFT** (`teko-bft`) para os dois livros, com regras de poder diferentes (ADR-0034):

| | Livro de **votos** | Livro de **economia** |
|---|---|---|
| Algoritmo | **BFT estilo Tendermint** | **BFT estilo Tendermint** |
| Quem valida | as **instituições** do genesis, poder igual (1 voto cada), sem moeda | quem tem moedas travadas (*stake*) |
| Bloco é final quando | mais de 2/3 das instituições deram precommit (certificado de commit) | mais de 2/3 do poder deu precommit |
| Tolera | menos de 1/3 das instituições falhas (3 instituições: nenhuma; 4: uma) | menos de 1/3 do poder falho ou malicioso |

Por que **não** dar poder por stake aos votos: quem ordena as cédulas não pode ser quem tem mais moeda; a legitimidade vem de instituições independentes e responsabilizáveis; e votos e moeda devem ficar desacoplados. Por que **não** *Proof of History*: ela prova ordem e passagem de tempo, mas não dá acordo bizantino e concentra o trabalho num líder, o contrário de validadores iguais.

**Nenhum bloco nasce sozinho:** uma altura só começa quando há entrada (voto) ou transação esperando, ou quando os pares já a começaram. O horário do bloco é o do proponente, que cada validador confere contra o próprio relógio (60 s).

![As duas cadeias do Teko: o mesmo motor BFT, com instituições de poder igual nos votos e poder por stake na economia](../diagrams/consensus-lanes.png)

## PoA, o que havia antes
Um validador da vez (rodízio por altura) propunha o bloco; ele só valia com M assinaturas de N. Era simples e rápido, mas não tinha finalidade nem resistia a um proponente que censura. Por isso foi substituído (ADR-0034).

## BFT (Tendermint) passo a passo
BFT = *Byzantine Fault Tolerance*: tolerância a falhas "bizantinas", em que o defeituoso pode mentir e agir de má-fé. Em cada **altura** (número do bloco) há **rodadas**:

1. **Proposta:** o proponente da rodada (sorteado de forma determinística e ponderada pelo poder) propõe um bloco.
2. **Prevote:** cada validador vota no bloco se ele for válido (ou em "nada").
3. **Precommit:** se viu **mais de 2/3** dos prevotes no mesmo bloco, o validador **trava (lock)** nesse bloco e dá precommit.
4. **Commit:** com **mais de 2/3** de precommits o bloco é decidido. Esses precommits assinados formam o **certificado de commit**, uma prova portátil que qualquer um verifica.
5. Se o tempo estoura (*timeout*), vem a **rodada seguinte** com outro proponente e prazo maior.

![Rodadas do BFT: proposta, prevote, precommit e commit; timeout leva a uma nova rodada](../diagrams/bft-rounds.png)

### Por que "mais de 2/3"?
Dois grupos com mais de 2/3 do poder cada **se sobrepõem em mais de 1/3**. Se menos de 1/3 for traidor, nessa interseção há alguém honesto, e o honesto só vota **uma vez por rodada**. Logo, dois blocos diferentes nunca são decididos na mesma altura (**segurança**).

![Por que o quórum é mais de 2/3: a interseção de dois quóruns tem mais de 1/3 e inclui alguém honesto](../diagrams/two-thirds-quorum.png)

### Lock, equivocação e slash
- **Lock:** depois de dar precommit num bloco, o validador não prevota outro bloco nas rodadas seguintes. Por isso ele é gravado em disco **antes** do voto sair (`economy.wal`, [ADR-0030](../adr/0030-slash-automatico-e-bloqueio-persistido.md)): um reinício não faz esquecer.
- **Equivocação:** assinar dois votos conflitantes (mesma altura, rodada e tipo). Os dois votos assinados são uma **evidência**, que qualquer um verifica.
- **Slash:** o nó que vê a evidência envia uma transação que **queima parte do stake** do traidor, uma vez por ofensa.

![Certificado de commit e evidência: precommits assinados viram certificado; votos conflitantes viram evidência](../diagrams/bft-certificate.png)

## Código
Std-only: o teste de quórum do Teko, o sorteio ponderado do proponente e o formato do WAL.

```rust
/// Mais de 2/3 do poder total. Multiplica em vez de dividir: sem decimais.
fn quorum(poder: u64, total: u64) -> bool {
    poder * 3 > total * 2
}

/// Sorteio ponderado e determinístico: um número derivado de (altura, rodada)
/// cai numa "fatia" proporcional ao poder de cada validador.
/// (O Teko deriva esse número com um hash com tag; aqui, uma mistura simples.)
fn proponente(poderes: &[u64], altura: u64, rodada: u32) -> usize {
    let total: u64 = poderes.iter().sum();
    let sorteio = altura
        .wrapping_mul(6364136223846793005)
        .wrapping_add(rodada as u64 * 1442695040888963407)
        .rotate_left(17);
    let mut fatia = sorteio % total;
    for (i, p) in poderes.iter().enumerate() {
        if fatia < *p { return i; }
        fatia -= *p;
    }
    0
}

/// Linha do WAL: "altura rodada [rodada_do_lock id_hex]".
fn ler_wal(linha: &str) -> Option<(u64, u32, Option<(u32, String)>)> {
    let mut it = linha.split_whitespace();
    let altura = it.next()?.parse().ok()?;
    let rodada = it.next()?.parse().ok()?;
    let lock = match (it.next(), it.next()) {
        (Some(r), Some(id)) => Some((r.parse().ok()?, id.to_string())),
        _ => None,
    };
    Some((altura, rodada, lock))
}

fn main() {
    // total 100: 67 é quórum, 66 não
    assert!(quorum(67, 100));
    assert!(!quorum(66, 100));

    // quem tem mais poder propõe mais vezes (estatístico)
    let poderes = [70, 20, 10];
    let mut cont = [0u32; 3];
    for h in 0..10_000 { cont[proponente(&poderes, h, 0)] += 1; }
    assert!(cont[0] > cont[1] && cont[1] > cont[2]);

    assert_eq!(ler_wal("7 2"), Some((7, 2, None)));
    assert_eq!(ler_wal("7 2 1 ab12"), Some((7, 2, Some((1, "ab12".into())))));
    println!("proposições: {cont:?}");
}
```

## Onde está no Teko
- `crates/teko-bft`: motor sans-I/O (`Engine`, `is_quorum`, `proposer`, `Commit`, `Evidence`, `locked`/`start_height_locked`).
- `crates/teko-economy` + `tools/teko-node/src/economy.rs`: economia sobre BFT, WAL, envio automático do `Slash`.
- `crates/teko-chain`: a cadeia de votos (bloco com certificado, réplica verificada, `VoteApp`); `tools/teko-node/src/lane.rs`: o hospedeiro BFT genérico das duas cadeias.
- Fluxos: [Consenso BFT](../flows/11-bft-consensus.md), [Rede e sincronização](../flows/10-network-and-sync.md). ADRs 0028 e 0030.

## Cuidados
- O BFT precisa que **menos de 1/3** do poder seja falho. Com 3 validadores de poder igual, o quórum exige os 3 (2 de 3 **não** passa de 2/3), então a rede não tolera nenhuma falha; com 4, o quórum é 3 e ela tolera 1.
- Na **fase de fundador** só um validador existe: é centralizado por desenho (ADR-0028).
- Sem auditoria; só devnet/testnet (a mainnet é travada).
