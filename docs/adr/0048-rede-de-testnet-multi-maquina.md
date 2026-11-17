# ADR-0048 — Rede de testnet em várias máquinas

**Status:** Aceito provisoriamente (2026-11-16, dono do produto) e **implementado** · Fecha o limite "TCP simples, sem criptografia entre nós" dos [ADR-0014](0014-quadro-de-avisos-e-blockchain.md) e [ADR-0034](0034-votos-em-bft-institucional-e-sem-blocos-vazios.md), o "canal entre validadores sem autenticação" da cerimônia do [ADR-0019](0019-cedulas-cifradas-elgamal-de-limiar.md) e a lista fixa de pares do [ADR-0045](0045-rotacao-das-instituicoes.md).

## Contexto
O `teko-node` só falava TCP puro em loopback: sem autenticação, sem cifra, sem limites. A identidade de um par era a posição no `peers.txt`; qualquer cliente chamava qualquer mensagem, inclusive `DKG_SHARE` (a parte secreta da chave da eleição) e `DKG_START` (reinicia a cerimônia). Todo endereço era `127.0.0.1` e a escuta era o endereço anunciado: nada ligava duas máquinas.

## Decisões do dono
- **Noise** (crate `snow`) vinculado à **chave de consenso P-256** da instituição.
- **Fase:** canal seguro, API pública separada com limites, testnet em várias máquinas e **descoberta de pares**.
- **Porta pública:** leitura e envio **sem login**, com limites (o cliente não confia no nó e reverifica tudo).

## Decisão técnica
1. **Crate `teko-p2p`** (só transporte, nenhuma regra de consenso). Handshake `Noise_XX_25519_ChaChaPoly_SHA256`. Cada nó tem uma chave estática X25519 própria (`node-<i>/noise.key`, 0600). A **prova de identidade** vai nos payloads do handshake: a chave de consenso assina (`NET_BIND`) a chave estática Noise do par, o `chain_id`, o hash do genesis e o papel (iniciador/respondedor). Quem verifica recusa se a assinatura não fecha com a chave estática **daquela conexão**, se a rede ou o genesis forem outros, se a instituição que atendeu não for a que se quis discar ou se a chave não estiver no conjunto em vigor. Sem prova válida, nada além do handshake é lido.
2. **Sessão cifrada:** mensagens lógicas em registros de até 65 000 bytes; o tamanho do corpo é conferido contra o limite do tipo **antes** de bufferizar; nonce de contador (repetição, adulteração e reordenação falham); o prazo cobre a **mensagem inteira** (gota-a-gota cai); as duas pontas trocam de chave em sincronia a cada 65 536 registros.
3. **Duas portas.** *Pares* (Noise): todos os tipos de mensagem de validador, o DKG e a troca de registros. *Pública* (sem login): só `SUBMIT`, `STATUS`, `GET_BLOCKS`, `ECON_TX`, `ECON_BLOCKS`, `ECON_STATUS`, uma requisição por conexão, com teto de conexões por IP e taxa por IP. O endereço de **escuta** é separado do **anunciado** (`--listen-peer`, `--listen-public`, `--advertise-*`, `--allow-remote`); a escuta fora de loopback é opt-in.
4. **Quem pode abrir sessão:** as chaves do conjunto de instituições em vigor da chain de votos **por época** (mais as trocas pendentes, ADR-0045) e os validadores da economia, atualizadas a cada segundo pelas lanes. Uma chave que sai do conjunto deixa de conectar.
5. **Limites.** Conexões (global, por IP e até 4 por chave), prazo de handshake de 5 s, taxa por sessão, **pontos por mau comportamento com banimento temporário** (10 min; handshake falho e mensagem proibida somam; um banimento também encerra as sessões da chave). O IP de loopback nunca é banido (numa rede local todos o compartilham). O envio de consenso usa **uma thread por par com fila limitada** (descarta o excedente; o motor reenvia a cada segundo) e **duas sessões por par** (gossip e chamadas), para um download de blocos nunca atrasar um voto.
6. **Cerimônia de chave.** O coordenador também é uma instituição (prova a chave como qualquer par). `DKG_START` leva as chaves dos participantes em ordem; um nó só aceita se **ele** é o participante daquele índice; `DKG_SHARE` só devolve a parte do índice `r` à instituição dona de `r` (nunca a própria, nunca outra); os endereços das partes vêm do próprio nó, não do chamador.
7. **Descoberta.** Cada validador publica um **registro de endereço assinado** (`PeerRecord`: chave, endereço de pares e público, `seq` = hora da assinatura, validade de 1 dia, no máximo 7) e os nós os trocam a cada 20 s pelas sessões autenticadas. Só valem chaves do conjunto de validadores, com `seq` maior, não vencidos, de endereço bem formado e da mesma rede; o livro persiste em `node-<i>/peers.book`. O `peers.txt` (`index peer=… public=… key=…`) é só o **bootstrap**: um registro assinado mais novo vence. Um nó que mudou de endereço é reencontrado sem editar arquivo nenhum.
8. **Operação.** `teko-node keygen` (cada instituição cria a chave **na própria máquina**; só a pública viaja), `genesis` (monta `network.txt` e `peers.txt` das públicas e endereços), `economy-init`, `genesis-hash` (conferência fora da banda; o hash entra no handshake: genesis diferente = recusado). `make devnet` usa exatamente o mesmo caminho seguro (não há atalho de teste). Runbook em [docs/TESTNET.md](../TESTNET.md).

## Validação
Em 2026-11-17 o dono do produto executou o roteiro [TESTNET-TWO-MACHINES](../TESTNET-TWO-MACHINES.md) em **dois computadores na mesma rede local** (4 instituições, 2 por máquina), todos os passos: `keygen`, `genesis`, conferência do `genesis-hash`, subida dos nós, `status`, cerimônia de chave e `blocks`, e a derrubada de validadores (a rede tolerou uma queda, pausou com duas e retomou). Validação manual, sem teste automatizado equivalente: os testes automatizados seguem em loopback.

## Limites honestos
- **Não validado ainda:** pela internet, atrás de NAT, com mais de dois computadores e com latência alta.
- A **porta pública não é cifrada** nem autentica o servidor: quem observa a rede vê o que cada cliente consulta (o conteúdo é reverificado pelo cliente, então um nó mentiroso é detectado, não obedecido).
- A descoberta cobre só **validadores** (conjunto fechado): não há nós observadores, NAT traversal, DHT nem rede aberta.
- **Qualquer instituição pode coordenar o DKG e, portanto, sabotá-lo** (a confiança entre elas é igual): o sigilo não depende disso, a disponibilidade sim.
- O banimento por pontos é local de cada nó; uma instituição que erra de boa-fé (por exemplo, relógio ou versão) pode ficar 10 minutos sem falar com um vizinho.
- A chave de consenso segue em arquivo (sem HSM); a chave Noise é por nó; os relógios precisam estar sincronizados (tolerância de 60 s do BFT).
- `snow` é dependência nova e não foi auditada por nós; o vínculo entre a chave Noise e a de consenso é nosso (testado, não revisado por terceiros). Sem auditoria.
