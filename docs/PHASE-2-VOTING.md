# Fase 2 — Votação e governança eleitoral (especificação para revisão)

Status: **spec em revisão; etapas 1 e 2 do plano já implementadas** (as que não dependem das decisões de criptografia): `teko-election` (§3.1 e §7) e `teko-bulletin` (§5). **Nada** de registro anônimo, prova ZK, apuração ou cédula foi escrito: depende das decisões do §10. Toda decisão de produto/segurança abaixo continua **PENDENTE** com alternativas e recomendação.

| Etapa do §9 | Estado |
|---|---|
| 1. `teko-election` (eleição com vários pleitos, jurisdições, M-de-N, ciclo de vida por pleito) | **Feito** (21 testes) |
| 2. `teko-bulletin` (log Merkle append-only, provas de inclusão/consistência, head assinado, arquivo) | **Feito** (13 testes + 3 de integração) |
| 3. Dispositivo: `LoadElection`, `GetElection`, `GetVoterKey`, `SignBallot` real (jurisdição, fase, um voto por pleito, nome do candidato vindo do manifesto), chave de eleitor por pleito; `issuer-cli` com domicílio eleitoral, governança e eleição de exemplo; Suite com a tela de votação por pleito | **Feito** (protótipo: cédula assinada, **não enviada**) |
| 4. `tools/election-admin-cli` (publicar no quadro, simular votos, apurar) + apuração pura e incremental em `teko-election` + tela de **Resultados ao vivo** na Suite (top 10 por pleito, por cargo/estado) | **Feito** (ver §5.1). Dados de demonstração são **simulados** e rotulados |
| 5–9 | Não iniciadas (registro anônimo, prova de voto, apuração, explorador com dados reais) |

**Etapa 3 (o que o dispositivo faz hoje):**
- Verifica a eleição sozinho (M-de-N do manifesto **e** de cada transição, replay do histórico) e **a re-verifica a cada uso**, então adulterar o armazenamento não adianta.
- **Recusa histórico mais antigo** que o já conhecido (`Downgrade`): sem isso, um host reapresentaria um histórico em que um pleito já encerrado ainda está aberto.
- Decide **elegibilidade pelo domicílio da própria credencial**: nacional ⇒ todos; estadual/municipal ⇒ só o seu. Pleito de outro estado ⇒ `NotEligible`; fase errada ⇒ `WrongPhase`; sem nenhum prompt ao usuário.
- **A tela mostra cargo, jurisdição e o NOME do candidato vindos do manifesto verificado**; o host só envia o **índice**. As linhas quebram por palavra (um nome nunca é cortado no meio).
- **Um voto por pleito por dispositivo** (`AlreadyVoted`), persistido; recusar na tela não gasta o voto. O registro é gravado **antes** de liberar a assinatura.
- A chave de eleitor é **por pleito**; a assinatura cobre `election_id`, `contest_id`, `contest_hash` e o índice.
- A Suite confere a assinatura da cédula antes de mostrá-la.

**Regras de ciclo de vida que implementei como proposta (revise, ADR-0016):** só avança um passo por vez; `Cancelada` só até a apuração e exige motivo; cada transição exige M-de-N, número de sequência sem buracos, horário não-decrescente e vínculo ao hash do manifesto; **não se abre registro/votação antes do horário do manifesto e não se encerra a votação antes do horário previsto** (encerramento antecipado é decisão de governança: **PENDENTE**).

Pré-requisito já entregue: dispositivo emulado, Suite, `issuer-cli`, atestação, credenciais de identidade (ver CLAUDE.md). O que **não** existe: registro de eleitor, prova de elegibilidade, urna, apuração, administração de eleições.

---

## 0. Achado que muda o desenho da Fase 1 (leia primeiro)

O desenho inicial era `C = Poseidon(s)` com o segredo `s` **dentro** do dispositivo, mais "prova ZK gerada no host a partir de uma assinatura/nullifier determinístico do dispositivo". Há uma contradição:

- Para provar em ZK "conheço `s` tal que `Poseidon(s)` está na árvore", **quem gera a prova precisa conhecer `s`**. É o desenho do Semaphore. Se o host gera a prova, o host vê `s` e o princípio 3 (chaves nunca saem do dispositivo) cai, e malware vende/rouba o voto.
- O que o PLUME/ERC-7524 resolve é diferente: o dispositivo produz uma **assinatura** e um **nullifier** deriváveis de uma **chave de assinatura**; o host prova em ZK que a assinatura é válida, que a chave pública está na árvore e que o nullifier foi calculado corretamente, **sem nunca ver a chave privada**.

**Consequência proposta:** o "segredo de eleitor" deixa de ser um escalar `s` com `C = Poseidon(s)` e passa a ser um **par de chaves de eleitor** `(sk_v, pk_v)` (por eleição, derivado de entropia própria do dispositivo). O que entra na árvore é `pk_v` (ou um hash dele). O voto é uma **assinatura** do dispositivo sobre a cédula, e a prova ZK é gerada no host sobre essa assinatura.

O código atual (`teko_crypto::voter::commitment/nullifier`, mensagens `GetVoterCommitment`/`SignBallot`) é um **stub descartável** e será substituído. Isso é a ADR-0006, agora com a recomendação concreta (§4).

---

## 1. Objetivos e não-objetivos

**Objetivos**
1. **Elegibilidade:** só cidadãos elegíveis votam, **uma vez por eleição**.
2. **Sigilo do voto:** ninguém (nem a autoridade, nem o administrador, nem o host do eleitor) liga a identidade à cédula.
3. **Verificabilidade:** qualquer pessoa confere que a apuração soma exatamente as cédulas válidas registradas, e cada eleitor confere que a sua entrou (verificabilidade individual) **sem** ganhar um recibo transferível (ver §6).
4. **O que se vê é o que se assina:** a cédula é confirmada na tela do dispositivo, com os nomes vindos de um manifesto de eleição assinado (M-de-N), não do host.
5. **Governança transparente:** ciclo de vida da eleição e parâmetros mudam só com assinatura de M instituições.

**Não-objetivos (por enquanto)**
- Resistência completa à coerção/compra de votos (receipt-freeness total). Veja §6: é a decisão mais difícil e é sua.
- Votação em produção real, requisitos legais, ICP-Brasil (ADR-0011).
- Hardware real (ADR-0012).

---

## 2. Papéis

| Papel | Faz | Confiança necessária |
|---|---|---|
| **Cidadão/eleitor** | Dispositivo + Suite | — |
| **Autoridade de identidade** | Já existe (emite credencial). Na Fase 2 também diz "esta pessoa é elegível para a eleição E" | Não deve conseguir ligar identidade a cédula |
| **Administrador da eleição** | Define candidatos, datas, regras; publica o manifesto | Assinatura M-de-N; auditável |
| **Cadastro eleitoral** | Atesta o **domicílio eleitoral** (UF e município) de cada cidadão; no Brasil é a Justiça Eleitoral, distinta da autoridade de identidade civil (ADR-0017) | Mesma exigência de não ligar identidade a cédula |
| **Registro (registrar)** | Aceita chaves de eleitor `pk_v` anônimas e publica a árvore | Não vê identidade |
| **Quadro de avisos / blockchain** | Armazena manifestos, árvore, cédulas e provas, append-only | Não-confiável, mas verificável |
| **Curadores (trustees)** | (Se houver cédula cifrada) guardam fatias da chave de apuração | Limiar t-de-n |
| **Auditores/observadores** | Reverificam tudo de forma independente | — |

---

## 3. Fluxo de ponta a ponta (alto nível)

```
 (A) Criar eleição     Admin ──manifesto──► assinado M-de-N ──► quadro (estado: Rascunho→Candidatos→Registro)
 (B) Registrar eleitor Cidadão: autentica na Autoridade (identidade) ──► recebe prova "elegível p/ E" (token cego)
                       Dispositivo: gera (sk_v,pk_v) da eleição E
                       Cidadão ──(canal separado, depois de atraso)──► Registro: (pk_v, token)  ──► árvore
 (C) Votar             Suite mostra candidatos; Dispositivo mostra a cédula (nomes do manifesto verificado)
                       Dispositivo assina (voto, E) e emite nullifier; Host gera prova ZK
                       Suite ──(canal anônimo)──► quadro: (cédula, nullifier, prova)
 (D) Apurar            Fecha a votação ──► soma (aberta ou cifrada+limiar) ──► resultado + prova de apuração
 (E) Auditar           Qualquer um reconstrói a contagem a partir do quadro
```

**Ponto crítico (B):** a Autoridade vê "o cidadão X pediu o token da eleição E" (identidade); o Registro vê `pk_v` (anônimo). Os dois **não podem** ser ligados. É o ADR-0008.

---

### 3.1 Cargos, jurisdições e calendários (ADR-0017)

Uma eleição real não é uma única pergunta: há **vários cargos**, cada um com **sua jurisdição, seus candidatos e seus prazos**, e o cidadão só vota nos pleitos do seu domicílio (com o Presidente aberto a todos). O modelo:

- **Eleição** = contêiner com um manifesto assinado M-de-N (ex.: "Eleições 2026").
- **Pleito (*contest*)** = uma disputa: cargo, jurisdição, candidatos, **calendário próprio** e **ciclo de vida próprio**.

| Cargo | Jurisdição | Quem pode votar | Exemplo de código |
|---|---|---|---|
| Presidente | Nacional | **Todos** | (vazio) |
| Governador, Senador, Deputado federal/estadual | Estado | Domicílio eleitoral nessa UF | `SP` |
| Prefeito, Vereador | Município | Domicílio eleitoral nesse município | `3550308` (código IBGE) |
| Plebiscito/referendo | Qualquer nível | Conforme a jurisdição declarada | — |

**Regras já implementadas em `teko-election`:**
- O cargo precisa combinar com a jurisdição (um "Governador" com escopo nacional é rejeitado).
- `is_eligible(domicílio, escopo)`: nacional ⇒ todos; estadual ⇒ mesma UF; municipal ⇒ mesmo município. Um carioca não vota para governador de SP; um paulista do interior vota para governador de SP mas não para prefeito da capital.
- **Cada pleito tem suas datas** (registro, abertura, encerramento) e **sua máquina de estados**; abrir ou fechar um pleito não mexe nos outros. A sequência de decisões de governança é global (para auditoria), mas os horários só não podem voltar **dentro** de um mesmo pleito.
- As transições se amarram ao **hash do pleito** (não do manifesto inteiro). Assim uma **revisão posterior pode acrescentar pleitos** (por exemplo, um **2º turno** depois da apuração do 1º) sem invalidar o histórico dos existentes; revisão que **altere ou remova** um pleito é recusada.

**O que isso muda na privacidade** (novas ameaças V11–V15 no THREAT-MODEL):
1. **Conjunto de anonimato = quem pode votar naquele pleito.** Presidente: todo o país. Prefeito de um município pequeno: poucos milhares, às vezes centenas. O sigilo do voto municipal é estruturalmente mais fraco.
2. **A chave de eleitor tem que ser por pleito, não por eleição.** Com a mesma chave no voto de presidente e no de prefeito, qualquer um liga os dois votos, e o anonimato do presidente **encolhe para o tamanho do município**. Por isso `pk_v` é derivada com `election_id` **e** `contest_id` (§4.1).
3. **Registros em momentos e canais separados:** registrar todos os pleitos de uma vez, pela mesma conexão, liga as chaves por tempo/rede.
4. **O domicílio é revelado ao registro** (inevitável: ele precisa saber em qual pleito o `pk_v` entra), mas não a identidade.
5. **O domicílio vale numa data de corte** (cadastro eleitoral fecha antes da eleição); a lista de elegíveis é uma fotografia. Decisão em aberto (§10, #10).

## 4. Credencial de eleitor e prova (ADR-0006, 0008, 0013)

### 4.1 Chaves de eleitor
- Raiz de eleitor `K_v` (32 bytes, **entropia separada** da identidade, já decidido) fica no dispositivo.
- **Por pleito:** `(sk_v,E,C, pk_v,E,C) = KDF(K_v, "teko/v2/voter" ‖ election_id ‖ contest_id)`. O mesmo eleitor **não é ligável** entre eleições **nem entre pleitos da mesma eleição** (presidente, governador, prefeito usam chaves independentes). Ver §3.1, ponto 2.
- A **autoridade nunca** vê `pk_v,E`.

### 4.2 Registro anônimo (ADR-0008) — alternativas
| | A. Assinatura cega (RFC 9474, RSA-BSSA) | B. Token anônimo (Privacy Pass / BBS) | C. Prova ZK de elegibilidade |
|---|---|---|---|
| Como | O dispositivo cega `pk_v`; a Autoridade, após autenticar a pessoa e marcar "já emitiu p/ E", assina o valor cego; o dispositivo desfaz o cegamento e entrega `(pk_v, assinatura)` ao Registro | Autoridade emite token por eleição, resgatado anonimamente | O cidadão prova em ZK "tenho credencial no conjunto de elegíveis" e registra `pk_v` na mesma prova |
| Maturidade | Padrão publicado (RFC 9474) | Padrões (RFC 9576–9578) | Mais novo, circuito maior |
| Autoridade vê `pk_v`? | **Não** | **Não** | **Não** |
| Custo no dispositivo | Módulo RSA: pesado em MCU (pode ser feito no host, ver risco) | Médio | Prova no host, mas precisa de credenciais compatíveis com ZK |
| Risco | Autoridade emite assinaturas extras (*stuffing*); mitigar com contagem pública | Idem | Menor confiança na autoridade, maior complexidade |

*Recomendação:* **A** como linha de base de estudo (padrão maduro, simples de auditar), com **contagem pública de assinaturas emitidas vs. elegíveis** contra *stuffing*; **C** como evolução. **PENDENTE (dono do produto).**

*Risco de privacidade operacional (vale para qualquer opção):* correlação por **tempo e rede** entre "pedi o token" e "registrei `pk_v`". Mitigações propostas: atraso aleatório (horas), canais separados (outra conexão, de preferência anonimizada), janela de registro longa. Isso não é resolvido por criptografia e deve entrar no modelo de ameaças.

### 4.3 Prova de voto (ADR-0013)
A prova (gerada no host) afirma:
1. conheço uma **assinatura válida** do dispositivo sobre `(election_id, ballot)` por uma chave `pk_v`;
2. `pk_v` está na **árvore de eleitores** de raiz `R` (publicada no quadro);
3. o **nullifier** `N` está calculado corretamente a partir da assinatura/chave e de `election_id`;
4. a cédula é **válida** (candidato existe no manifesto).

Sistema de prova (ADR-0013), alternativas: **zkVM em Rust** (RISC Zero / SP1: escreve-se a verificação P-256 + Merkle em Rust comum, combina com o projeto, prova mais lenta), **Circom + Groth16** (prova pequena e verificação barata em EVM, mas ECDSA P-256 em circuito é caro e há cerimônia de *trusted setup*), **Halo2/Noir** (sem setup universal por circuito, curva de aprendizado). *Recomendação:* prototipar em **zkVM**, por ser Rust e reaproveitar o `p256` (reportam aceleração para P-256; **verificar** antes de depender). **PENDENTE.**

*Ponto de pesquisa:* o PLUME original é sobre secp256k1. Para P-256 é preciso um **nullifier determinístico equivalente**. Alternativa mais simples: derivar o nullifier como `N = PRF(sk_v, election_id)` e prová-lo **junto** com a assinatura no mesmo programa de prova (o dispositivo entrega `N` e uma assinatura sobre `(E, N, ballot)`; a prova checa que `N` é consistente com a mesma `pk_v`). Isso exige que o dispositivo prove consistência de `N` sem revelar `sk_v`, o que só é possível se `N` for **verificável a partir da assinatura** (como o PLUME) ou se o dispositivo emitir uma segunda prova. **É a principal lacuna técnica aberta** e merece uma ADR/estudo próprio antes de qualquer código.

---

## 5. Quadro de avisos / blockchain (ADR-0014)

O quadro guarda, em ordem e imutável: manifestos assinados, âncoras da árvore (raízes), cédulas+nullifiers+provas, resultado.

| | A. Cadeia EVM (L2 ou rede permissionada) com verificador de provas | B. Cadeia própria (Substrate/Cosmos) | C. Log transparente (Merkle append-only) atrás de uma `trait BulletinBoard` |
|---|---|---|---|
| Fidelidade a "blockchain" | Alta | Alta | Média (sem consenso entre muitos nós) |
| Esforço | Alto (Solidity + infra) | Muito alto | **Baixo** |
| Verificação on-chain da prova | Sim (se Groth16) | Sim | Não (verificadores off-chain) |
| Bom para estudo agora | Depois | Não | **Sim** |

*Recomendação:* começar com **C** (crate `teko-bulletin`, em Rust, com a mesma interface que depois ganha backend EVM), porque isola a decisão de blockchain e deixa o foco na criptografia. **PENDENTE.**

---

### 5.1 Resultados ao vivo (ADR-0018)

Requisito do produto: **um gráfico por cargo e por estado, top 10 do mais ao menos votado, com nome e votos em tempo real** (como a visão ao vivo do Trezor Suite). Como funciona:

- Cada **cédula** é uma entrada do quadro de avisos. A apuração é uma **função pura do log**: percorre as entradas em ordem, aplica a governança (manifesto e transições M-de-N) e conta. Qualquer pessoa refaz a contagem e confere a raiz do quadro: nada precisa ser confiado.
- **Regras de aceitação** (todas verificáveis só pelo log): assinatura da cédula válida e amarrada ao **hash do pleito**; a opção existe; o pleito estava em **VotacaoAberta no ponto do log em que a cédula foi anexada** (o tempo é a ordem do log, não um relógio); **uma cédula por chave de eleitor por pleito** (a primeira vale). Rejeitadas são **contadas por motivo**, não somem.
- **Tempo real** = a interface lê o quadro continuamente; o gráfico acompanha cada cédula aceita.

**Por que isso é uma decisão de segurança, não só de interface:**
1. Com **cédula pública** (ADR-0015, opção 1), *qualquer um* conta ao vivo; não dá para "esconder" a parcial de forma criptográfica. Só **cédula cifrada** (opção 2) permite esconder a distribuição até o fim.
2. Resultado parcial divulgado durante a votação **influencia** quem ainda não votou e **ajuda quem monitora ou compra voto** (vê o efeito imediatamente). Muitas eleições reais só divulgam depois do fechamento.
3. Por isso o manifesto declara uma **política de visibilidade** (`results_visibility`): `live` (distribuição ao vivo), `turnout_only` (só quantas cédulas, a distribuição depois do fechamento) ou `after_close` (nada até fechar). **Com cédula pública a política é só convenção de interface**; o log em claro continua contável por qualquer um. Enquanto a decisão #3 estiver em aberto, isso precisa ser dito na tela.
4. **Resultado por estado de um pleito nacional** (ex.: Presidente por UF) exige que cada cédula **revele a UF do eleitor**, o que reduz o conjunto de anonimato ao estado (decisão #15). Por ora o Presidente aparece **só no total nacional**; Governador aparece **por estado** porque o pleito já é estadual.

**Implementado:** `teko_election::tally`/`Tallier` (apuração pura e incremental), `election-admin publish|simulate|tally`, e a tela **Resultados ao vivo** da Suite (abas por cargo; um gráfico de barras horizontais por pleito, top 10 + "Outros"; atualização a cada ~1,5 s; barras animadas; aviso "DADOS SIMULADOS"; política de visibilidade aplicada na tela). O voto feito com o dispositivo real **entra no quadro e aparece no gráfico**.

**Dados de demonstração:** um único dispositivo não gera distribuição. O `election-admin-cli simulate` anexa cédulas sintéticas (chaves aleatórias) para o gráfico andar. A tela mostra **"DADOS SIMULADOS"** de forma inconfundível. No protótipo **não há registro anônimo**, então qualquer chave é aceita: a contagem **não é confiável** contra fraude (decisão #2).

## 6. Sigilo, cédula aberta ou cifrada e coerção (ADR-0015) — a decisão mais difícil

| Opção | Sigilo | Coerção / compra de voto | Complexidade |
|---|---|---|---|
| **1. Cédula pública + nullifier** (voto em claro na cadeia, anônimo por não-ligação) | Sigilo só por não-ligação; metadados podem vazar | **Recibo trivial**: o eleitor mostra a cédula e a prova → compra de voto fácil | Baixa |
| **2. Cédula cifrada para curadores** (limiar), apuração depois do fechamento | Sigilo até o fim; curadores são ponto de confiança | Ainda há recibo se o eleitor guarda a aleatoriedade | Média |
| **3. Re-voto (vale o último) / MACI** | Melhor contra coerção (o coator não sabe qual voto valeu) | Resistente a compra; precisa de coordenador de confiança (MACI) | Alta |
| **4. Cédula cifrada + prova de que não há recibo (receipt-free)** | Forte | Pesquisa ativa | Muito alta |

*Recomendação:* **1** para a primeira versão de estudo, **declarando em letras grandes que não resiste à compra de voto**, e desenhar a interface para permitir **3** depois. É decisão de produto e de segurança: **PENDENTE (dono do produto).** O THREAT-MODEL já registrava C2–C5 como risco residual total; a Fase 2 não os resolve sozinha.

---

## 7. Manifesto de eleição e governança (ADR-0016)

**Manifesto** (assinado M-de-N pelas instituições administradoras): `election_id`, título, **lista de pleitos** (cada um com `contest_id`, cargo, título exibido, **jurisdição**, lista **ordenada** de candidatos/opções com nome e número, e **calendário próprio**), tipo de cédula, chave do registro, `revision`. (Regra de apuração, curadores e parâmetros de prova entram quando as decisões do §10 saírem.) O **dispositivo verifica a assinatura M-de-N** e mostra o nome do candidato **a partir do manifesto**, nunca a partir do host.

**Ciclo de vida, por pleito** (máquina de estados; cada transição é uma mensagem assinada M-de-N no quadro e amarrada ao hash daquele pleito):
`Rascunho → CandidatosPublicados → RegistroAberto → VotacaoAberta → VotacaoEncerrada → Apurada → Auditada`
(+ `Cancelada` com justificativa, só até a apuração; cancelar um pleito não afeta os outros). Nada volta de estado. Mudanças viram **nova revisão do manifesto que só pode acrescentar pleitos** (ex.: 2º turno).

**Governança:** quem são as N instituições, quanto vale M, como rotacionar chaves e como contestar uma eleição são **decisões de produto fora do escopo técnico**, mas o sistema precisa expor os pontos de decisão. **PENDENTE.**

---

## 8. Mudanças no dispositivo e no protocolo (propostas, ainda não implementadas)

| Mensagem | Substitui / acrescenta | Confirma? | Tela do dispositivo |
|---|---|---|---|
| `LoadElection{manifest}` | novo | Sim | "Eleição X, abre em…, assinada por k de n" |
| `GetVoterKey{election_id}` | substitui `GetVoterCommitment` | Sim | "Registrar-me na eleição X?" |
| `BlindRegistration{...}` (se opção A) | novo | Sim | idem |
| `SignBallot{election_id, option_index}` | substitui o stub | **Sim** | "VOTAR em: **<nome vindo do manifesto>** na eleição X?" |

Invariantes novas a testar: a cédula só é assinada se o candidato exibido veio do manifesto verificado; `sk_v` nunca sai; chaves de eleitor de eleições diferentes não são ligáveis; nada na resposta liga `pk_v` à chave de identidade; o dispositivo recusa eleição com M-de-N insuficiente ou fora de fase.

---

## 9. Plano de implementação (após aprovação, ordem sugerida)

1. `teko-election`: manifesto, máquina de estados, verificação M-de-N (reaproveita `teko-attestation`). **Sem criptografia nova.**
2. `teko-bulletin`: trait `BulletinBoard` + implementação em arquivo/memória (log Merkle, prova de inclusão).
3. Dispositivo: `LoadElection`, `SignBallot` real (exibe nome do manifesto), chaves de eleitor por eleição. Emulador + testes de propriedade.
4. `tools/election-admin-cli`: cria/assina manifestos e avança fases.
5. ✅ Registro anônimo (ADR-0008: RFC 9474, chave RSA por pleito) + `issuer-cli` como autoridade de registro. Implementado; falta ZK/nullifier (etapa 6).
6. Prova de voto: primeiro uma **prova simulada** com a mesma interface (para fechar o fluxo e a UI), depois o provador real (ADR-0013).
7. Apuração aberta + verificação independente (`election-admin-cli verify`).
8. ✅ Suite: explorador público com dados reais do quadro (regras, candidatos, calendário, decisões da governança com assinaturas) e conferência de inclusão Merkle por posição. Fechamento automático no prazo (`election-admin close-due`). Falta: comparar heads com testemunhas independentes (split view, V19).
9. Atualizar THREAT-MODEL, KEY-HIERARCHY, DEVICE-PROTOCOL, TEST-MATRIX a cada etapa.

---

## 10. Decisões que preciso de você (todas **PENDENTES**)

| # | Decisão | Recomendação |
|---|---|---|
| 1 | **Voter credential = par de chaves** (e não `Poseidon(s)`), por causa do achado do §0 | Aceitar |
| 2 | **Registro anônimo** (ADR-0008): A assinatura cega, B token anônimo ou C prova ZK | A agora, C depois |
| 3 | **Sigilo/coerção** (ADR-0015): cédula pública, cifrada, re-voto/MACI | Pública no estudo, com aviso explícito; desenhar para re-voto |
| 4 | **Sistema de prova** (ADR-0013): zkVM, Circom, Halo2/Noir | Prototipar em zkVM |
| 5 | **Quadro de avisos** (ADR-0014): EVM, cadeia própria ou log Merkle | Log Merkle atrás de trait |
| 6 | **Governança** (ADR-0016): N instituições, M, rotação de chaves | Reusar o M-de-N da atestação (2-de-3 em teste) |
| 7 | **Quem apura?** Apuração aberta (cédula em claro) vs. curadores com limiar | Aberta no estudo |
| 8 | Aceitar que a **primeira versão não resiste à compra de voto** | Sim, declarado na UI e nos docs |
| 9 | **Quem atesta o domicílio eleitoral** (ADR-0017): a mesma autoridade de identidade ou um cadastro eleitoral separado | Conceitualmente separado; no estudo, o `issuer-cli` simula os dois papéis |
| 10 | **Data de corte** do domicílio (fotografia da lista de elegíveis) e o que acontece com quem se muda | Fotografia fixa no manifesto; mudança só vale na próxima eleição |
| 11 | **Voto em branco e nulo**, e cargos **proporcionais/multi-vaga** (deputado, vereador: voto em candidato ou legenda) | Branco/nulo como opções explícitas; proporcional resolvido no ADR-0046 (lista aberta, quociente + D'Hondt, barreiras, sorteio) |
| 12 | **Eleitor no exterior** (vota só para Presidente) e outras exceções de domicílio | Modelar como domicílio especial; fora do primeiro corte |
| 13 | **Encerramento antecipado** de um pleito (hoje o sistema recusa fechar antes do horário) | Manter recusa; emergência vira cancelamento motivado |
| 14 | **Política de resultados parciais** (ADR-0018): `live`, `turnout_only` ou `after_close` | ✅ **Decidido:** nada de parciais, **criptograficamente** (ADR-0019: cédulas cifradas, só os totais são decifrados no fechamento por M-de-N validadores) |
| 15 | **Resultado por UF/município de um pleito nacional**: exige a UF do eleitor na cédula (reduz o anonimato ao estado) | Não fazer no primeiro corte; Presidente só no total nacional |
| 16 | **Quem é o operador do quadro** e como terceiros fixam e comparam *heads* (testemunhas) para detectar *split view* | Vários espelhos independentes; no estudo, um operador local |

## 11. Riscos e perguntas em aberto

1. **Lacuna técnica central (§4.3):** nullifier determinístico verificável para P-256 sem revelar `sk_v`. Exige estudo/prova de conceito antes da etapa 6.
2. **Correlação por rede/tempo** entre autoridade e registro (§4.2): só mitigável operacionalmente.
3. **Autoridade maliciosa** (*stuffing*, recusar elegíveis): contagem pública e auditoria; não eliminada.
4. **Disponibilidade:** quem roda o quadro? Censura/derrubada durante a janela de votação.
5. **Dispositivo comprometido/clonado:** sem seed, o risco é físico; a revogação de eleitor perdido sem ligar identidade a `pk_v` é um problema aberto (ADR-0003 × ADR-0008).
6. **Acessibilidade e UX de conferência:** o eleitor precisa comparar o que o dispositivo mostra com o que ele quer; telas pequenas e candidatos com nomes parecidos pedem desenho cuidadoso.
7. **Referências a estudar antes de implementar:** Helios, Belenios, Semaphore, MACI, RFC 9474, Privacy Pass (RFC 9576–9578), PLUME/ERC-7524. *Listadas por nome; não verifiquei os detalhes de cada uma aqui.*
