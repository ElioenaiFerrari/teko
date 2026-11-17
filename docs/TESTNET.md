# Testnet em várias máquinas (runbook)

Prova de conceito, **dados fictícios, chaves de teste**. A mainnet segue travada. Cada instituição roda **um** `teko-node` na própria máquina; só chaves públicas e arquivos públicos viajam. Desenho e limites: [ADR-0048](adr/0048-rede-de-testnet-multi-maquina.md) e [NETWORKS](NETWORKS.md).

## 0. O que você precisa
- 3 ou mais máquinas (uma por instituição; com 4, uma pode cair) com o repositório compilado (`cargo build --release -p teko-node`).
- Relógios sincronizados (NTP): o BFT recusa blocos com horário a mais de 60 s do relógio de quem vota.
- Duas portas TCP por máquina: a de **pares** (Noise; só as outras instituições precisam alcançá-la) e a **pública** (clientes). Padrão sugerido: 7901 e 7911.

## 1. Cada instituição cria a SUA chave (na própria máquina)
```
teko-node keygen --dir net/node-0        # troque 0 pelo seu índice depois que o coordenador o definir
```
Imprime a **chave pública**; envie-a ao coordenador. A chave secreta (`validator.key`) nunca sai da máquina.

## 2. O coordenador monta a rede
Arquivo `validators.txt` (a ordem é a ordem dos validadores; o índice de cada um é a linha):
```
02ab…  peer=a.exemplo.org:7901  public=a.exemplo.org:7911
03cd…  peer=b.exemplo.org:7901  public=b.exemplo.org:7911
02ef…  peer=c.exemplo.org:7901  public=c.exemplo.org:7911
```
```
teko-node genesis --dir net --network testnet --validators validators.txt
teko-node economy-init --dir net --founder tekow1…      # opcional: a economia (o fundador é a sua carteira)
```
Distribua `net/network.txt`, `net/peers.txt` (e `net/economy.txt`) a todas as instituições.

## 3. Todos conferem o genesis
```
teko-node genesis-hash --dir net
```
O hash tem de ser **idêntico** em todas as máquinas (compare por outro canal). Ele entra no handshake: um nó com outro genesis é recusado.

## 4. Cada instituição sobe o nó
```
teko-node run --dir net --index I \
  --listen-peer 0.0.0.0:7901 --listen-public 0.0.0.0:7911 \
  --advertise-peer a.exemplo.org:7901 --advertise-public a.exemplo.org:7911 \
  --allow-remote
```
- A porta de pares só fala com uma chave do conjunto em vigor; a pública só lê e envia o que se autoassina, com limites. Libere no firewall a de pares para as **outras instituições** e a pública para quem for usar a Suite ou o explorador.
- Se o endereço de alguma máquina mudar, basta subir o nó com o novo `--advertise-*`: os outros o reencontram pelo registro assinado (a cada 20 s), sem editar `peers.txt`.

## 5. Conferir
```
teko-node status --dir net       # altura, pares online, época das instituições
teko-node dkg --dir net          # cerimônia de chave da eleição (o coordenador usa a chave de uma instituição desta pasta)
```
`status` precisa dos endereços **públicos** do `peers.txt`; o `dkg` fala pela porta de pares com a chave `node-<i>/validator.key` que estiver na pasta.

## O que dá errado
| Sintoma | Causa provável |
|---|---|
| `another genesis` no log do handshake | `network.txt`, `peers.txt` ou `economy.txt` diferentes: compare `genesis-hash` |
| `not a validator in force` | a chave não está no conjunto (ou saiu por rotação) |
| nó sobe e ninguém o enxerga | `--advertise-*` não alcançável de fora, ou firewall na porta de pares |
| o nó recusa subir | escuta fora de loopback sem `--allow-remote` |
| blocos não avançam | menos de mais de 2/3 das instituições no ar, ou relógios fora de 60 s |
| uma instituição "some" por ~10 min | banimento por mau comportamento (versão ou relógio errados); some sozinho |

## Limites honestos
Veja o [ADR-0048](adr/0048-rede-de-testnet-multi-maquina.md): porta pública sem cifra, sem NAT traversal nem nós observadores, qualquer instituição pode atrapalhar a cerimônia (disponibilidade, não sigilo), chaves em arquivo, sem auditoria.
