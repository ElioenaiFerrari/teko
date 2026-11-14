# Modelo de ameaças

Status: **rascunho para revisão**. Os testes citados como `prop_*` são ilustrativos: os nomes reais estão em [TEST-MATRIX](TEST-MATRIX.md). Metodologia: ativos → adversários → STRIDE por fronteira → ameaça / mitigação / risco residual / teste.

## 1. Ativos

| ID | Ativo | Por que importa |
|---|---|---|
| A1 | Chave de identidade (privada) | Personificação do cidadão |
| A2 | Segredo de eleitor `s` | Identifica/vende o voto; quebra anonimato |
| A3 | PIN | Desbloqueia A1/A2 |
| A4 | Credenciais de atributos | Dados pessoais |
| A5 | Chaves de lote de atestação | Falsificar dispositivos "genuínos" |
| A6 | Chaves raiz da autoridade / de firmware | Emitir identidades e firmware falsos |
| A7 | Firmware | Controla tudo no dispositivo |
| A8 | Privacidade do voto (não-vinculação identidade↔voto) | Propriedade central do sistema |
| A9 | Integridade do que o usuário assina | Assinar algo diferente do exibido |

## 2. Adversários

| ID | Adversário | Capacidades |
|---|---|---|
| E1 | Malware no host | Controla Suite, SO, USB/transporte; vê e altera todo o tráfego; pode apresentar UI falsa |
| E2 | Ladrão/achador do dispositivo | Posse física; tentativas de PIN; (HW real) extração |
| E3 | Coator / comprador de votos | Pressiona ou paga o cidadão; exige prova de voto/posse de segredo |
| E4 | Atacante de supply chain | Adultera firmware, dispositivo, dependências, build ou release da Suite |
| E5 | Autoridade maliciosa/comprometida | Controla emissão, registros e chaves da autoridade |
| E6 | Observador de rede/serviços | Correlaciona IDs entre serviços |

## 3. Ameaças e mitigações

Colunas: **STRIDE** (S spoofing, T tampering, R repudiation, I info disclosure, D DoS, E elevation).

### 3.1 Host comprometido (E1)

| # | Ameaça | STRIDE | Mitigação | Risco residual | Teste |
|---|---|---|---|---|---|
| H1 | Malware troca o hash/documento enviado para assinatura | T | Dispositivo mostra resumo legível + hash e só assina o que foi confirmado; o hash exibido é o hash assinado | Usuário não confere a tela; resumo legível enganoso (o resumo vem do host) → mostrar sempre o hash e rotular o resumo como "informado pelo computador" | `prop_signed_hash_equals_displayed_hash` |
| H2 | Suite pede assinatura sem confirmação | E | Máquina de estados: `SignDocument` → `ButtonRequest` → só assina após confirmação física | — | `prop_no_signature_before_confirmation` |
| H3 | Phishing de PIN (Suite falsa pede PIN) | S/I | O PIN só é digitado no dispositivo; o protocolo não tem campo de PIN; documentação ensina o usuário | Usuário digita PIN em tela falsa do host; no HW real, teclado do dispositivo | `prop_no_message_carries_pin` |
| H4 | Extração de chaves via protocolo | I | Nenhuma mensagem retorna material privado; tipos de segredo sem `Serialize` | Bug de implementação | `prop_no_message_exposes_private_key` |
| H5 | Replay de respostas/requisições | T | `request_id` + nonce no desafio de atestação; confirmações atreladas à requisição pendente | Replay de atestação já válida é inofensivo (desafio novo) | `prop_replayed_ack_rejected` |
| H6 | Mensagens malformadas / confusão de versão | D/E | Parser total (sem pânico), limites de tamanho, `UnsupportedVersion` | — | `prop_parser_never_panics`, fuzz |
| H7 | Correlação de IDs entre serviços | I | IDs pairwise por serviço com chave derivada, distintos e não correlacionáveis | Serviços que coludem via ID Cívico ou atributos revelados | `prop_pairwise_ids_distinct_and_unlinkable` |
| H8 | Suite falsa/adulterada | S/T | Release assinado + build reproduzível; a Suite verifica atestação, mas **o dispositivo é o árbitro** de tudo que importa | Suite adulterada mente na UI (ex.: diz "genuíno"); mitigação: fingerprint do firmware e ID Cívico conferidos na tela do dispositivo | e2e + checagem manual documentada |
| H9 | Suite mente sobre o resultado da atestação | S | Atestação é verificável por terceiros; usuário pode reverificar com ferramenta independente (`issuer-cli verify`) | Usuário não reverifica | e2e |

### 3.2 Roubo/perda do dispositivo (E2)

| # | Ameaça | Mitigação | Risco residual | Teste |
|---|---|---|---|---|
| T1 | Brute force de PIN | Contador de tentativas persistente, **atraso exponencial**, **wipe após N erros** | PIN fraco; com N alto e PIN curto, a probabilidade de acerto é N/10^d | `prop_wrong_pin_n_times_wipes` |
| T2 | Comparação de PIN revela informação por tempo | `subtle::ConstantTimeEq`; hash do PIN com sal e KDF lento | Canais laterais de HW | `prop_pin_compare_constant_time` (estrutural) |
| T3 | Reiniciar/cortar energia para zerar o contador | Incrementar e persistir o contador **antes** de comparar o PIN | Rollback do armazenamento (no emulador) | `prop_counter_persisted_before_check` |
| T4 | Extração física de chaves | **Fora de escopo no emulador**; HW real: SE/MCU com proteção de leitura (ADR-0012) | Total no emulador | — |
| T5 | Dispositivo perdido ainda é válido | Revogação pela autoridade na reemissão (ADR-0003); lista de revogação | Janela entre perda e revogação; revogação de lote vs. individual (ver §4) | e2e de revogação (futuro) |

### 3.3 Coerção e venda de voto (E3)

| # | Ameaça | Mitigação | Risco residual |
|---|---|---|---|
| C1 | **Seed phrase vendável**: vendê-la transfere identidade e voto | Recomendação: **sem seed** (ADR-0003, pendente) | Se houver seed, a compra de voto é trivial |
| C2 | Comprador exige **prova de como votou** (recibo) | O dispositivo não emite comprovante transferível de voto | Receipt-freeness completa exige desenho da fase de votação; **trabalho futuro** |
| C3 | Votar sob vigilância (coerção presencial) | Fora de escopo desta fase; mecanismos como re-voto pertencem à fase da blockchain | Total nesta fase |
| C4 | Comprador exige entrega do dispositivo + PIN | Impossível impedir tecnicamente; mitigado por re-voto/revogação (futuro) | Total |
| C5 | Prova de posse de `s` ("eu tenho a credencial") vendida | O segredo não é exportável; prova ZK não prova *como* votou | Venda do dispositivo inteiro |

### 3.4 Supply chain (E4)

| # | Ameaça | Mitigação | Risco residual | Teste |
|---|---|---|---|---|
| S1 | Firmware adulterado | Manifesto com **M-de-N** assinaturas de instituições independentes; bootloader recusa | Colusão de M instituições | `prop_firmware_without_m_of_n_rejected` |
| S2 | **Downgrade** para versão vulnerável | Contador monotônico de versão no bootloader | — | `prop_downgrade_rejected` |
| S3 | Assinaturas duplicadas contadas como M | Contar **chaves distintas** do conjunto N | — | `prop_duplicate_signatures_count_once` |
| S4 | Dispositivo falsificado ("clone") | Atestação com cadeia até a raiz | Vazamento de chave de lote | `prop_attestation_chain_verifies` |
| S5 | Dependência Rust maliciosa | `cargo-deny`/`cargo-vet`, `--locked`, poucas dependências, revisão | Zero-day em dependência | CI |
| S6 | Build não reproduzível esconde backdoor | Builds reproduzíveis verificados por terceiros | Pouca verificação independente | CI de reprodutibilidade |
| S7 | Suite adulterada na distribuição | Assinatura + hashes em canal independente | Usuário não verifica | doc de verificação |

### 3.5 Autoridade maliciosa ou comprometida (E5)

| # | Ameaça | Mitigação | Risco residual |
|---|---|---|---|
| X1 | Emite identidades falsas (cidadãos fantasma) | Auditoria pública do registro de emissões (futuro); separação de funções | Confiança na autoridade é irredutível nesta fase |
| X2 | **Liga identidade ↔ commitment de eleitor** no registro | Canal separado + mecanismo sem vínculo (assinatura cega / credencial anônima / prova de pertinência) — **ADR-0008, a questão de segurança mais importante** | Enquanto o ADR-0008 estiver pendente, o stub é **ingênuo e vincula** (documentar) |
| X3 | Chave raiz única comprometida | Raízes múltiplas independentes, M-de-N para firmware; chaves raiz offline | Chave raiz de emissão de credencial ainda é única nesta fase (pergunta em aberto) |
| X4 | Rastreia dispositivos pela atestação | **Chaves de lote** (ADR-0007), atestação nunca acompanha voto | Lotes pequenos reduzem o anonimato |
| X5 | Autoridade vê o dispositivo gerar chaves | As chaves nascem no dispositivo; só a pública vai à autoridade | Firmware malicioso (mitigado por S1/S6) |

### 3.6 Outras

| # | Ameaça | Mitigação | Teste |
|---|---|---|---|
| O1 | Erro de digitação do ID Cívico | bech32m com checksum | `prop_civic_id_detects_typos` |
| O2 | Colisão/ambiguidade entre domínios de hash | Tags de domínio únicas e versionadas | `prop_domain_separation` |
| O3 | Segredos permanecem em memória | `zeroize` no `Drop` | revisão + teste estrutural |
| O4 | Canal lateral de tempo em assinaturas | Bibliotecas de tempo constante | — |

## 4. Tensões a registrar (não resolvidas)

1. **Atestação em lote × revogação individual:** chaves de lote protegem privacidade, mas impedem revogar um dispositivo individual pela atestação; a revogação precisa ocorrer no nível da credencial de identidade.
2. **O dispositivo conhece identidade e segredo de eleitor.** A garantia é "nunca derivados um do outro e nunca emitidos juntos". Um firmware malicioso poderia vazá-los juntos; mitigação: M-de-N + build reproduzível + open source.
3. **Resumo legível vem do host.** O dispositivo só confia no hash; o resumo é rotulado como não verificado.
4. **Emulador não tem isolamento.** Todas as garantias de resistência física são aspiracionais até o HW real.

## 5. Premissas e fora de escopo

- O emulador **não** oferece isolamento de hardware; um processo malicioso no mesmo host poderia ler sua memória. Serve para validar protocolo e lógica, não segurança física.
- Assinatura com validade jurídica (ICP-Brasil): trabalho futuro.
- Prova ZK real, blockchain de votação, governança: fases futuras.
- Verificação presencial da pessoa pela autoridade acontece fora do sistema e é assumida correta.

## 5.1 Limitações conhecidas do emulador e da Suite (Fase 1)

Registradas após o passe de hardening de 2026-11-13. Nenhuma é aceitável em hardware real.

| Limitação | Efeito | Mitigação atual | Resolve com |
|---|---|---|---|
| O arquivo de estado do emulador guarda `k_root` e `s` em **claro** (permissão 0600) | Quem lê o arquivo obtém as chaves; o PIN deixa de importar | Só ambiente de teste; ver aviso no README | Secure element / flash protegida (ADR-0012) |
| Hash do PIN = SHA3 iterado 20 mil vezes (placeholder) | Se o hash vazasse, PIN de 6 dígitos cai em segundos offline | O contador de tentativas + wipe é a defesa real | KDF em hardware |
| Contador de PIN e atraso exponencial usam arquivo e relógio de parede | Restaurar um backup do arquivo devolve tentativas; mudar o relógio pula o atraso | Persistir o contador antes de comparar (testado) | Contador monotônico em hardware |
| Emulador escuta em TCP `127.0.0.1` | Outro processo local pode conectar e pedir operações | Recusa endereço não-loopback; derruba conexão ociosa (30 s); nada é assinado sem confirmação no terminal do emulador | Transporte USB HID |
| Suite = webview | Motor de navegador no processo | CSP `default-src 'none'`, sem recursos remotos, escape de todo dado do dispositivo, permissões Tauri só de eventos, limites de tamanho no IPC (32 MB) | — |
| Credenciais SD-JWT-like sem unlinkability | Dois verificadores correlacionam apresentações da mesma credencial | ADR-0005 | BBS+ ou credenciais de uso único |

## 5.2 Ameaças específicas da votação (Fase 2, propostas)

Não implementadas; ver `PHASE-2-VOTING.md`. Cada linha vira teste quando houver código.

| # | Ameaça | Mitigação proposta | Risco residual |
|---|---|---|---|
| V1 | Autoridade liga identidade ↔ `pk_v` | Registro anônimo (assinatura cega/token/ZK) | Correlação por tempo/rede (só operacional) |
| V2 | *Stuffing*: autoridade emite mais credenciais que elegíveis | Contagem pública emitidas vs. elegíveis; auditoria | Autoridade ainda decide quem é elegível |
| V3 | Voto duplo | Nullifier por eleição verificado no quadro | Falha do esquema de nullifier para P-256 (lacuna §4.3) |
| V4 | Host troca o candidato | Cédula confirmada no dispositivo com nome **do manifesto M-de-N** | Usuário não confere a tela |
| V5 | Manifesto falso | Dispositivo exige M-de-N válido e fase correta | Colusão de M instituições |
| V6 | **Compra de voto / recibo** | **Não mitigada na 1ª versão** (ADR-0015) | **Total** até haver re-voto/receipt-freeness |
| V7 | Coerção presencial | Fora de escopo | Total |
| V8 | Censura/queda do quadro na janela de votação | Espelhos/witnesses; janela longa | Depende de quem opera |
| V9 | Provador no host vê a chave de eleitor | Credencial = par de chaves; o provador só vê assinatura/nullifier | Bug no protocolo de prova |
| V10 | Eleitor perdido/clonado | Sem seed; revogação sem ligar identidade (ADR-0003 × ADR-0008) | Problema aberto |
| V11 | **Ligar os votos de um mesmo eleitor entre pleitos** (presidente × governador × prefeito) e reduzir o anonimato ao menor conjunto | Chave de eleitor **por pleito** (`election_id ‖ contest_id`), nullifier por pleito | Falha no isolamento de chaves |
| V12 | **Conjunto de anonimato pequeno** em município pequeno (centenas de eleitores) | Limiar mínimo de eleitores por pleito (ADR-0017); aviso na UI | Em município muito pequeno o sigilo é fraco por natureza |
| V13 | Registrar-se em jurisdição onde não mora (domicílio falso) | A **autoridade** valida o domicílio ao conceder o direito de registro; o dispositivo não decide | Autoridade/cadastro errado ou fraudado |
| V14 | Correlação por **tempo e rede** entre os registros de vários pleitos | Registrar cada pleito em sessão/canal/horário distintos | Só mitigação operacional |
| V16 | **Resultado parcial ao vivo** influencia eleitores e ajuda a monitorar/comprar voto | Política de visibilidade no manifesto (`turnout_only`/`after_close`); **só impõe com cédula cifrada** (ADR-0015/0018) | Com cédula pública, o log em claro é contável por qualquer um |
| V17 | **Dados simulados** confundidos com resultado real | Marcador "DADOS SIMULADOS" inconfundível; simulador só no `election-admin-cli` | Usuário ignora o aviso |
| V18 | **Quebra por UF de pleito nacional** reduz o anonimato ao estado | Não fazer no primeiro corte (decisão #15) | Eleitorado pequeno |
| V19 | Operador do quadro mostra logs diferentes a pessoas diferentes (*split view*) ou atrasa/omite cédulas | *Heads* assinados fixados e comparados por testemunhas; apuração refeita por terceiros | Sem testemunhas independentes hoje |
| V15 | Mudança de domicílio perto da eleição (voto duplo em dois lugares ou perda do direito) | Data de corte e fotografia da lista (decisão #10) | Casos de borda |

## 6. Perguntas em aberto

1. **Valor de N (tentativas de PIN antes do wipe) e comprimento do PIN.** Alternativas: 8 tentativas / PIN 6 dígitos (similar a Trezor/Ledger); 5 / 8 dígitos. *Recomendação:* 8 tentativas, PIN de 6–9 dígitos, atraso exponencial a partir da 4ª. **PENDENTE (dono do produto)**.
2. **Wipe após N erros apaga também a credencial?** Apagar tudo exige reemissão presencial (mais seguro, mais custo). *Recomendação:* apagar tudo. **PENDENTE**.
3. **Raiz de emissão de credenciais: única ou M-de-N?** *Recomendação:* chave única para credenciais nesta fase (offline, com ADR de rotação), M-de-N para firmware. **PENDENTE**.
4. **Como tratar o stub de registro de commitment enquanto ADR-0008 está aberto?** Alternativas: (a) registro ingênuo marcado como "NÃO PRIVADO"; (b) não implementar o registro, apenas a interface. *Recomendação:* (b). **PENDENTE**.
5. **Tamanho mínimo de lote de atestação** → ADR-0007. **PENDENTE**.
6. **Política de revogação de dispositivo perdido** → ADR-0003. **PENDENTE**.
