# Protocolo Suite ↔ dispositivo

Status: **rascunho para revisão**. Serialização do payload depende do ADR-0004 (recomendação: protobuf/`prost`); a notação abaixo é neutra.

## 1. Princípios

- Mensagens tipadas e versionadas; o parser é **total** (nunca entra em pânico) e rejeita o que não reconhece.
- **Nenhuma mensagem carrega PIN, chave privada, seed ou `s`.** O PIN é digitado no dispositivo. Na **carteira** (ADR-0023) a frase de 24 palavras e a passphrase são **mostradas e digitadas só no dispositivo**: o host recebe apenas `ButtonRequest` (tipo) e o endereço público.
- Todo pedido que exige confirmação mostra **no dispositivo** o que será assinado; o dispositivo assina o que ele mesmo exibiu.
- O dispositivo é um servidor de uma requisição por vez (como Trezor): uma requisição pendente por vez; novo pedido durante interação → `Busy` ou `Cancel` explícito.

## 2. Framing

Compatível com HID de 64 bytes no futuro (fragmentação), mas o formato lógico é independente do transporte.

```
Frame lógico:
  magic        2 bytes   0x54 0x4B ("TK")
  proto_ver    1 byte    versão do protocolo (começa em 1)
  flags        1 byte    bit0: resposta; bit1: erro; demais reservados = 0
  msg_type     2 bytes   big-endian
  request_id   4 bytes   escolhido pelo host; ecoado na resposta
  payload_len  4 bytes   big-endian; máximo 64 KiB (1 MiB só em FirmwareUpdate/Chunk)
  payload      N bytes
  crc32        4 bytes   integridade de transporte (NÃO é autenticação)
```

- **Fragmentação HID (futuro):** cada pacote de 64 bytes = `0x3F` + 63 bytes de dados; o primeiro leva o cabeçalho.
- Campos reservados ≠ 0 → `MalformedFrame`. `payload_len` acima do limite → `MalformedFrame` sem alocar.
- Timeout de leitura de frame incompleto: descarta e responde `MalformedFrame`.

## 3. Versionamento e negociação

- `Initialize{ host_proto_max }` → `Features{ proto_ver, fw_version, state, device_id_class, capabilities[] }`.
- O dispositivo escolhe `min(host, device)`; se não houver versão comum → `UnsupportedVersion`.
- `msg_type` desconhecido → `UnknownMessage`. Campos novos só entram com nova `proto_ver` ou como opcionais ignoráveis.

## 4. Estados do dispositivo

`Virgem → FirmwareInstalado → Inicializado → CredenciaisEmitidas → [Bloqueado ⇄ Desbloqueado]`

(`Inicializado` = PIN definido e chaves geradas.)

## 5. Catálogo de mensagens

Legenda — **Confirma?**: exige confirmação física no dispositivo. Todas as respostas podem ser `Error{code, ...}`.

| Mensagem (request → response) | Estados aceitos | Confirma? | Tela do dispositivo |
|---|---|---|---|
| `Initialize{host_proto_max}` → `Features` | todos | Não | — |
| `GetFeatures` → `Features` | todos | Não | — |
| `AttestationChallenge{nonce[32]}` → `AttestationResponse{cert_chain, signature}` | todos exceto Virgem sem cert (ver pergunta 3) | Não | — |
| `FirmwareUpdate` begin/chunk/finish (abaixo) → `FirmwareStatus` | Virgem, FirmwareInstalado, Inicializado\*, CredenciaisEmitidas\* (\* só Desbloqueado) | **Sim** (instalar vX → vY, fingerprint) | "Instalar firmware vY? hash ab12…" |
| `SetupPin` → `ButtonRequest(PinEntry)` … → `PinSet` | FirmwareInstalado | PIN digitado **no dispositivo** (2x) | "Defina o PIN", "Repita o PIN" |
| `Unlock` → `ButtonRequest(PinEntry)` … → `Unlocked` ou `PinWrong{remaining, retry_after}` | Bloqueado | PIN no dispositivo | "Digite o PIN" |
| `GetCivicId` → `CivicId{id, pk_identity}` | Inicializado (após chaves), CredenciaisEmitidas, Desbloqueado | Não na leitura; **Sim** na primeira exibição para conferência (onboarding) | ID Cívico completo |
| `GetPairwiseId{service_id}` → `PairwiseId{id}` | Desbloqueado | Sim | "Gerar ID para `service_id`?" |
| `SignDocument{hash[32], summary}` → `ButtonRequest` → `DocumentSignature{sig, pk_identity}` | Desbloqueado | **Sim** | "Assinar documento?", resumo ("informado pelo computador") + hash |
| `ProveAttribute{predicate, verifier_nonce}` → `ButtonRequest` → `AttributeProof{proof}` | Desbloqueado | **Sim** | "Provar: `idade ≥ 18` para <verificador>?" |
| `GetVoterCommitment` → `VoterCommitment{C}` | Desbloqueado | Sim | "Exportar commitment de eleitor?" (stub) |
| `SignBallot{election_id, ballot_hash}` → `BallotSignature` | Desbloqueado | **Sim** | "Votar em <opção>?" (**stub**: retorna `NotImplemented` ou assinatura fictícia marcada) |
| `SetupWallet` → `ButtonRequest(WalletCreate, SeedWords×4, WalletPassphrase, WalletConfirm)` → `WalletReady{address, has_passphrase}` | Inicializado, desbloqueado, sem carteira | **Sim** (cada página + 3 posições digitadas no dispositivo) | Mostra 24 palavras em 4 páginas; pergunta passphrase; pede as palavras das posições sorteadas |
| `RestoreWallet` → `ButtonRequest(WalletRestore…)` → `WalletReady` | Idem | **Sim** (24 palavras digitadas + conferir o endereço) | "Digite as 24 palavras AQUI"; mostra o endereço derivado |
| `CheckWalletBackup` → `ButtonRequest(WalletCheck)` → `WalletBackupOk` ou `Failure` | Desbloqueado, com carteira | **Sim** (redigitar a frase) | "Verificar backup"; só diz se confere |
| `SignTransfer{chain_id, to_address, amount, fee, nonce}` → `ButtonRequest(SignTransfer)` → `TransferSigned{signature, public_key}` | Desbloqueado, com carteira | **Sim** | "ENVIAR MOEDAS": destino, valor, taxa, total e rede (o dispositivo monta a transação e assina o que mostrou) |
| `GetAddress` → `Address{address, public_key}` | Desbloqueado, com carteira | Não (dados públicos) | — |
| `Wipe` → `ButtonRequest` → `WipeDone` | todos exceto Virgem | **Sim** (pressionar e segurar) | "APAGAR TUDO? Irreversível." |

Mensagens de apoio:

- `ButtonRequest{kind, request_id}` (dispositivo → host): "estou aguardando o usuário"; o host mostra "confirme no dispositivo".
- `ButtonAck` (host → dispositivo): apenas sinaliza que o host recebeu o `ButtonRequest`; **não é confirmação**. A confirmação é um evento físico local. Isso evita que malware "confirme" por software.
- `Cancel` (host → dispositivo): aborta a interação pendente; dispositivo responde `Error{UserRejected}` ou `Cancelled`.
- `Ping` / `Success` / `Failure` mínimos para teste.

### 5.1 FirmwareUpdate

```
FirmwareBegin{ manifest }            // manifesto: versão, hash do blob, tamanho, assinaturas M-de-N
  → dispositivo valida manifesto (bootloader): ≥M assinaturas distintas de N raízes; versão ≥ contador
  → se inválido: Error{InsufficientSignatures | BadSignature | Downgrade}
  → se válido: ButtonRequest(FirmwareInstall) e usuário confirma no dispositivo
FirmwareChunk{ offset, data }        // repetido; resposta FirmwareStatus{received}
FirmwareFinish{}                     // valida hash do blob vs manifesto; instala; incrementa contador de versão
  → FirmwareStatus{installed_version}
```

- Apagar/atualizar firmware **não** apaga chaves por padrão; mudança de política → pergunta 5.

### 5.2 Predicados de `ProveAttribute`

Conjunto fechado e versionado: `AgeAtLeast{n}`, `HasAttribute{name}` (revela só existência), `Reveal{name}` (revela o valor — exige tela explícita). Nada de predicados arbitrários (evita phishing por escopo).

## 6. Códigos de erro

| Código | Significado |
|---|---|
| `MalformedFrame` | Cabeçalho/CRC/tamanho inválidos |
| `UnknownMessage` | `msg_type` desconhecido |
| `UnsupportedVersion` | Sem versão de protocolo comum |
| `InvalidState` | Mensagem não permitida no estado atual |
| `Busy` | Há interação pendente |
| `PinLocked{retry_after_ms}` | Atraso exponencial em curso |
| `PinWrong{remaining}` | PIN incorreto (informação, sem revelar o PIN) |
| `DeviceWiped` | Limite de tentativas excedido; dispositivo apagado |
| `UserRejected` | Usuário rejeitou no dispositivo |
| `Timeout` | Usuário não respondeu a tempo |
| `BadSignature` | Assinatura inválida (manifesto/atestação) |
| `InsufficientSignatures` | Menos que M assinaturas distintas |
| `Downgrade` | Versão menor que a instalada |
| `HashMismatch` | Hash do firmware ≠ manifesto |
| `NotImplemented` | Stub (ex.: `SignBallot`) |
| `Internal` | Falha interna (sem detalhes sensíveis) |

Mensagens de erro **nunca** incluem material secreto nem diferenciam causas de forma que vaze informação (ex.: PIN).

## 7. Matriz estado × mensagem

✔ aceita · ✖ `InvalidState` · (c) com confirmação

| Mensagem | Virgem | FW Inst. | Inicializado | Cred. Emitidas | Bloqueado | Desbloqueado |
|---|---|---|---|---|---|---|
| Initialize / GetFeatures | ✔ | ✔ | ✔ | ✔ | ✔ | ✔ |
| AttestationChallenge | ✔\* | ✔ | ✔ | ✔ | ✔ | ✔ |
| FirmwareUpdate | ✔(c) | ✔(c) | ✖ | ✖ | ✖ | ✔(c) |
| SetupPin | ✖ | ✔ | ✖ | ✖ | ✖ | ✖ |
| Unlock | ✖ | ✖ | ✖ | ✖ | ✔ | ✖ |
| GetCivicId | ✖ | ✖ | ✔(c) | ✔ | ✖ | ✔ |
| GetPairwiseId | ✖ | ✖ | ✖ | ✖ | ✖ | ✔(c) |
| SignDocument | ✖ | ✖ | ✖ | ✖ | ✖ | ✔(c) |
| ProveAttribute | ✖ | ✖ | ✖ | ✖ | ✖ | ✔(c) |
| GetVoterCommitment | ✖ | ✖ | ✖ | ✖ | ✖ | ✔(c) |
| SignBallot (stub) | ✖ | ✖ | ✖ | ✖ | ✖ | ✔(c) |
| SetupWallet / RestoreWallet | ✖ | ✖ | ✖ | ✖ | ✖ | ✔(c)\* |
| CheckWalletBackup / GetAddress | ✖ | ✖ | ✖ | ✖ | ✖ | ✔ (c) |
| SignTransfer | ✖ | ✖ | ✖ | ✖ | ✖ | ✔(c)\*\* |

\*\* Exige carteira pronta (`Features.wallet_ready`).

| Wipe | ✖ | ✔(c) | ✔(c) | ✔(c) | ✔(c) | ✔(c) |

\* Só em `Inicializado` (antes do ID Cívico) e uma única vez: `GetCivicId` e `GetIdentityRequest` respondem `InvalidState` enquanto não houver carteira pronta (`Features.wallet_ready`).

Nota: após `CredenciaisEmitidas` o dispositivo entra em `Bloqueado` no próximo ciclo/no reinício/após inatividade; `Desbloqueado` expira por inatividade. O estado "Inicializado" é transitório e **desbloqueado** (acabou de definir o PIN). Mensagens de emissão de credencial (`GetIdentityRequest` / `LoadCredential`) são detalhadas no ONBOARDING-FLOW e serão adicionadas ao catálogo (pergunta 2).

## 8. Invariantes testáveis

1. Nenhum tipo de mensagem contém campo de chave privada, PIN, seed ou `s` (verificação estrutural + `proptest` sobre codificação).
2. Toda assinatura é precedida de `ButtonRequest` e de confirmação física; sem confirmação, nenhuma assinatura.
3. O hash assinado == hash exibido.
4. `ButtonAck` do host **nunca** substitui a confirmação física.
5. O parser aceita ou rejeita qualquer sequência de bytes sem pânico; `decode(encode(m)) == m`.
6. Mensagem fora do estado permitido → `InvalidState` sem efeito colateral.
7. Respostas de erro não variam em função de segredo.
8. **Carteira:** nenhuma resposta ou mensagem intermediária contém palavra da frase, entropia, semente ou passphrase (teste varre todos os frames de um fluxo completo); erros nunca ecoam palavras nem posições.

## 8.1 Estado implementado (Fase 1)

- **Mensagens de emissão adicionadas:** `GetIdentityRequest{authority_nonce, authority_name}` → `IdentityRequest{pk_identity, pop_signature, attestation_chain, attestation_signature}` e `LoadCredential{credential}` → `CredentialLoaded`.
- **`FirmwareUpdate`** foi dividido em `FirmwareBegin`/`FirmwareChunk`/`FirmwareFinish`.
- **`ButtonAck` não é exigido** pelo emulador: o dispositivo envia `ButtonRequest` e bloqueia na confirmação física; o `ButtonAck` do host é aceito e ignorado (nunca vale como confirmação).
- **`Unlock` é idempotente** se já desbloqueado. Uma **nova conexão sempre começa bloqueada** (como replugar o USB).
- **`GetCivicId`** só exige confirmação física no estado `Inicializado` (onboarding, para conferência); depois apenas exibe na tela.
- **Polling passivo** (`GetFeatures`, `Initialize`, `Ping`, `AttestationChallenge`) **não renova** o tempo de desbloqueio.
- **Predicados:** `AgeAtLeast` (usa a menor claim derivada `age_over_K ≥ N`), `Has` (claim booleana `true`) e `Reveal` (mostra o valor na tela do dispositivo).
- `SignBallot`: o dispositivo pede confirmação e responde `NotImplemented` (nunca assina).

## 8.2 Mensagens de eleição (Fase 2, implementadas)

| Mensagem | Estados | Confirma? | Resposta / observação |
|---|---|---|---|
| `LoadElection{manifest, transitions[]}` | Desbloqueado + credencial | **Sim** | `ElectionLoaded{election_id, title, revision, seq, contests[]}`; verifica M-de-N e o histórico; recusa histórico mais antigo |
| `GetElection{}` | Desbloqueado + credencial | Não | Mesmo resumo, da eleição armazenada (re-verificada); cada pleito traz `registration_required` |
| `BeginRegistration{election_id, contest_id}` | Pronto + credencial; pleito com `registration_key`, em `RegistrationOpen`/`VotingOpen`, no domicílio | **Sim** ("REGISTRAR PARA VOTAR?") | `RegistrationRequest{blind_message, pk_identity, pop_signature, credential}`: o **dispositivo** calcula o valor cego da própria chave de eleitor (determinístico); a chave de eleitor **não** vai na resposta. Entregue à autoridade |
| `FinishRegistration{election_id, contest_id, blind_signature}` | Idem | Não | `RegistrationToken{voter_key, token}`; **recusa** (`BadSignature`) resposta que não vira assinatura válida. Publique `(voter_key, token)` no quadro, nunca junto do diálogo com a autoridade |
| `GetVoterKey{election_id, contest_id}` | Idem; registro aberto; elegível | **Sim** | `VoterKey{voter_key}`: chave **pública** do pleito |
| `SignBallot{election_id, contest_id, option_index}` | Idem; votação aberta; elegível; ainda não votou | **Sim** | `BallotSignature{signature, voter_key}`; a tela mostra o nome vindo do manifesto |

Substituem `GetVoterCommitment` e o stub antigo de `SignBallot`. Novos códigos de erro: `NotEligible`, `WrongPhase`, `AlreadyVoted`. `ContestInfo` informa fase, jurisdição, opções, `eligible` e `voted` (calculados **no dispositivo**).

## 9. Perguntas em aberto

1. **Serialização** (prost / postcard / CBOR) → ADR-0004. *Recomendação:* `prost`. **PENDENTE**.
2. **Mensagens de emissão** (`GetIdentityRequest` = chave pública + prova de posse; `LoadCredential`): não constavam na lista mínima, mas o onboarding precisa delas. Alternativas: incluir no protocolo (recomendado) ou reutilizar `GetCivicId`+`SignDocument`. **PENDENTE (dono do produto)**.
3. **Atestação em estado Virgem:** o dispositivo já tem certificado de lote de fábrica? *Recomendação:* sim, gravado na fabricação (como o Trezor), de modo que a atestação funcione antes do firmware do usuário. **PENDENTE**.
4. **Timeout de confirmação** (30 s? 60 s?) e expiração do estado Desbloqueado. *Recomendação:* 60 s de confirmação; Desbloqueado expira em 5 min de inatividade. **PENDENTE (dono do produto)**.
5. **Atualização de firmware preserva as chaves?** *Recomendação:* sim, exige Desbloqueado e confirmação. **PENDENTE**.
6. **`SignDocument`: o que exatamente é assinado?** `hash` puro ou `tag("teko/v1/doc-sign") ‖ hash`. *Recomendação:* sempre com tag (separação de domínio). **PENDENTE**.
7. **Tamanho máximo do `summary`** e charset (restringir a ASCII/Unicode seguro para evitar spoofing visual). **PENDENTE**.
8. **`SignBallot` no stub:** retornar `NotImplemented` ou assinatura fictícia marcada? *Recomendação:* `NotImplemented` com a tela de confirmação já funcional. **PENDENTE**.
