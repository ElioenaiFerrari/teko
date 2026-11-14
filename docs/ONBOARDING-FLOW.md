# Fluxo de primeira utilização (onboarding)

Status: **rascunho para revisão**. Cada passo mostra o que aparece na **Suite** (não confiável) e no **dispositivo/emulador** (confiável), as mensagens do protocolo e os caminhos de erro. Resultado esperado: dispositivo em `CredenciaisEmitidas`, `Bloqueado ⇄ Desbloqueado`.

Convenção: tudo que é sensível (PIN, conferência de ID, confirmações) acontece **no dispositivo**. A Suite apenas orienta.

## Visão geral

```
Detectar → Atestar → Firmware → PIN → Chaves/ID → Emissão → Commitment → Pronto
Virgem ─────────────▶ FW Inst. ─▶ Inicializado ─────▶ Cred. Emitidas ─▶ Bloqueado/Desbloqueado
```

## Passo 1 — Detectar o dispositivo

| Suite mostra | Dispositivo mostra | Protocolo |
|---|---|---|
| "Procurando dispositivo…" → "Dispositivo encontrado" + estado (Virgem, FW vX…) | Logo/tela inicial | `Initialize` → `Features` |

Erros: nenhum dispositivo → instruções para conectar/abrir o emulador; `UnsupportedVersion` → "atualize a Suite".

## Passo 2 — Verificar autenticidade (atestação)

| Suite mostra | Dispositivo mostra | Protocolo |
|---|---|---|
| "Verificando se este dispositivo é genuíno…" | — | `AttestationChallenge{nonce}` → `AttestationResponse{cadeia, assinatura}` |
| Resultado: **"Dispositivo genuíno"** (✔, lote/fabricante) ou **"Falhou"** (✖, motivo) | — | Suite verifica: assinatura sobre `tag ‖ nonce ‖ Features`; cadeia lote → raiz; raiz esperada embutida/pinned |

- Falha: **não prosseguir**; mostrar causa (cadeia inválida, raiz desconhecida, assinatura inválida, lote revogado) e "não use este dispositivo".
- A atestação nunca contém dados do usuário nem acompanha voto.
- A Suite é não confiável: oferecer comando independente para reverificar (`issuer-cli verify-device`).

## Passo 3 — Verificar e instalar/atualizar o firmware

| Suite mostra | Dispositivo mostra | Protocolo |
|---|---|---|
| "Firmware instalado: nenhum / vX. Disponível: vY (assinado por k de n instituições: A, B, C)" | — | `GetFeatures` |
| Suite verifica o manifesto M-de-N **antes de enviar** e exibe o fingerprint | — | (local) |
| "Confirme no dispositivo" | "Instalar firmware vY? Fingerprint ab12 cd34…" [Confirmar / Rejeitar] | `FirmwareBegin` → `ButtonRequest(FirmwareInstall)` |
| Barra de progresso | "Instalando…" | `FirmwareChunk`* |
| "Firmware vY instalado e verificado" | "Firmware vY" + fingerprint | `FirmwareFinish` → `FirmwareStatus` |

Erros: `InsufficientSignatures`/`BadSignature` (firmware recusado), `Downgrade` ("versão mais antiga que a instalada — recusada"), `HashMismatch`, `UserRejected`. O **bootloader do dispositivo é o árbitro**, não a Suite.
Estado: `Virgem → FirmwareInstalado`. Se já há firmware atual, o passo é pulado/atualiza.

## Passo 4 — Definir o PIN (no dispositivo)

| Suite mostra | Dispositivo mostra | Protocolo |
|---|---|---|
| "Defina seu PIN **no dispositivo**. Este computador não vê o PIN." (sem campo de entrada) | "Defina o PIN" → "Repita o PIN" | `SetupPin` → `ButtonRequest(PinEntry)` … `PinSet` |
| "PIN definido ✔" | "PIN definido" | — |

Regras: PIN de comprimento definido (pergunta 1 do THREAT-MODEL); as duas digitações devem coincidir, senão o dispositivo recomeça; PIN trivial (`123456`, repetido) é recusado **pelo dispositivo**. Erros: `UserRejected`, `Timeout`.
Aviso ao usuário: **perder o PIN = reemissão presencial**; **N erros = apagamento**.

## Passo 5 — Geração de chaves e conferência do ID Cívico

| Suite mostra | Dispositivo mostra | Protocolo |
|---|---|---|
| "Gerando suas chaves dentro do dispositivo…" | "Gerando chaves…" | (interno após `PinSet`) |
| "Seu ID Cívico: `teko1…`. **Confira que é idêntico ao exibido no dispositivo.**" | ID Cívico completo (em blocos de 4–5 caracteres) [Confirmar] | `GetCivicId` → `ButtonRequest` → `CivicId` |

- A confirmação no dispositivo é a âncora contra uma Suite que mostra um ID diferente.
- Estado: `Inicializado` (PIN definido, chaves geradas).
- `s` (segredo de eleitor) também é gerado agora, **mas não é exibido nem exportado**; só `C` poderá sair, depois, por canal separado.

## Passo 6 — Emissão da credencial de cidadão

Pré-requisito **fora do sistema**: a pessoa foi verificada presencialmente pela autoridade.

| Suite mostra | Dispositivo mostra | Protocolo / `issuer-cli` |
|---|---|---|
| "Apresente-se à autoridade. Código de emissão: `XXXX-XXXX`" | — | — |
| "Pedindo prova de posse da chave…" | "Emitir credencial para a autoridade <nome>? ID `teko1…`" [Confirmar] | `GetIdentityRequest{authority_nonce}` → `ButtonRequest` → `IdentityRequest{pk_identity, atestação, assinatura}` |
| Entrega o pedido à `issuer-cli` | — | `issuer-cli issue --request req.bin --subject <dados>` (a autoridade verifica atestação e prova de posse, assina a credencial **vinculada a `pk_identity`**) |
| "Carregando credencial…" | "Instalar credencial? Emissor: …, atributos: nome, data de nascimento…" [Confirmar] | `LoadCredential{credential}` → `ButtonRequest` → `CredentialLoaded` |
| "Credencial emitida ✔" | "Credencial instalada" | — |

Erros: credencial com `subject_key` ≠ chave do dispositivo → recusada; assinatura do emissor inválida → recusada; emissor não reconhecido → recusada.
Estado: `Inicializado → CredenciaisEmitidas`.

## Passo 7 — Registro do commitment de eleitor (canal separado)

Objetivo: a autoridade/registro aceita `C` como eleitor elegível **sem ligá-lo à identidade**.

| Suite mostra | Dispositivo mostra | Protocolo |
|---|---|---|
| "Registro de eleitor: etapa independente da sua identidade" | "Exportar commitment de eleitor? (não identifica você)" [Confirmar] | `GetVoterCommitment` → `VoterCommitment{C}` |
| Envio por **canal separado** (outra sessão/conexão, sem ID Cívico, sem metadados correlacionáveis) | — | Mecanismo: **ADR-0008 (pendente)** |

- **Nesta fase é stub**: interface e fluxo de telas; nenhum registro real. Enquanto o ADR-0008 estiver aberto, qualquer registro simulado é marcado **"NÃO PRIVADO — apenas teste"**.
- Cuidados: não enviar `C` na mesma requisição que `pk_identity`/ID Cívico; não reutilizar sessão/conexão; evitar correlação por tempo (atraso aleatório; pergunta 4).

## Passo 8 — Conclusão

| Suite mostra | Dispositivo mostra |
|---|---|
| "Pronto. Seu dispositivo está configurado." + resumo: genuíno ✔, firmware vY ✔, ID `teko1…`, credencial ✔ | Tela de espera (Bloqueado) |
| Funcionalidades disponíveis após `Unlock`: ver identificação, assinar documento, provar atributo, votar (stub) | "Digite o PIN" quando solicitado |

## Fluxos laterais

- **Uso diário:** `Unlock` (PIN no dispositivo) → ações; `Desbloqueado` expira por inatividade.
- **PIN errado:** `PinWrong{remaining}`; a partir da 4ª tentativa, atraso exponencial (`PinLocked{retry_after}`); após N → `DeviceWiped`, volta a `Virgem`/`FirmwareInstalado`.
- **Wipe voluntário:** `Wipe` com confirmação (segurar botão); depois, reemissão presencial.
- **Perda/troca de dispositivo:** reemissão presencial na autoridade com **revogação** do dispositivo antigo (depende do ADR-0003).
- **Atualização de firmware posterior:** mesmo passo 3, exige `Desbloqueado`.
- **Sem dispositivo:** Suite abre em modo leitura (explorador público; placeholder).

## Perguntas em aberto

1. **Como a autoridade vincula a pessoa verificada ao pedido de emissão?** Alternativas: código de emissão de uso único entregue presencialmente (recomendado); QR no balcão; a autoridade digita o ID Cívico mostrado no dispositivo. *Recomendação:* código de uso único + conferência do ID Cívico na tela. **PENDENTE (dono do produto)**.
2. **Que atributos a credencial contém nesta fase?** *Recomendação:* nome, data de nascimento, nacionalidade, número de registro interno (não CPF/RG). **PENDENTE**.
3. **Como a Suite entrega o pedido à `issuer-cli` e recebe a credencial:** arquivos (offline, recomendado para estudo) ou canal local? **PENDENTE**.
4. **Registro de commitment (passo 7):** mecanismo → ADR-0008; atraso aleatório antes do envio? **PENDENTE**.
5. **Qual o fluxo do passo 5 se o usuário disser que os IDs não coincidem?** *Recomendação:* abortar, oferecer `Wipe`, orientar a não usar o computador. **PENDENTE**.
6. **Dispositivo já inicializado conectado à Suite nova:** reconhecer e pular onboarding (recomendado); oferecer verificação de atestação sempre. **PENDENTE**.
7. **Atestação ocorre antes do firmware do usuário** (certificado de fábrica) → ver DEVICE-PROTOCOL, pergunta 3. **PENDENTE**.
8. **Idioma da Suite** (pt-BR apenas ou i18n desde o início). **PENDENTE**.
