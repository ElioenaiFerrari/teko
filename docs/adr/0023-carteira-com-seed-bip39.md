# ADR-0023 — Carteira com frase de recuperação (BIP-39 + SLIP-10 P-256), separada da identidade

**Status:** **Aceito** (2026-11-14, dono do produto): seed estilo carteira Bitcoin, **24 palavras**, **passphrase opcional**, confirmação por **3 posições sorteadas** no onboarding. **Implementado** (M5a). **Supera em parte o [ADR-0003](0003-recuperacao-seed-vs-reemissao.md)**: o "sem seed" continua valendo para a **identidade e o voto**; a **carteira** é uma exceção separada.

## Contexto
Com moeda e bens no livro-razão (ADR-0021, ADR-0022), perder o dispositivo significa perder fundos, e não existe autoridade que devolva moeda. A reemissão presencial recupera a **identidade**, não o dinheiro. O ADR-0003 recusou a seed porque ela seria um ativo vendável que clona identidade e voto; esse risco desaparece se a seed **só** controlar a carteira.

## Decisão
- **Entropia própria.** A carteira nasce de 32 bytes do RNG do dispositivo, **independentes** de `K_root` e do segredo de eleitor `s`. Nada da identidade ou do voto deriva da frase, nem o contrário (princípio 5). Vender ou perder a frase não revela quem você é nem como votou.
- **BIP-39 em inglês, 24 palavras**, com **passphrase opcional** (25ª palavra), só ASCII imprimível até 64 caracteres (evita normalização Unicode). A frase aparece **somente na tela do dispositivo**, em páginas de 6 palavras; **nenhuma mensagem do protocolo** a carrega (o teste de nomes de campos proíbe `seed`, `mnemonic` etc.).
- **Derivação SLIP-10 para `nist256p1`** (P-256, como o resto do Teko), caminho todo *hardened* `m/7240'/0'/0'/0'/i'`. O dispositivo guarda só a **chave mestra derivada** (64 bytes) e um indicador de passphrase; **nunca** a frase nem a passphrase.
- **Endereço** = hash da chave pública (o mesmo `address_of` do livro-razão), exibido em bech32m com prefixo `tekow` (o checksum detecta erro de digitação).
- **Onboarding obrigatório e confirmado no dispositivo.** Etapa nova entre PIN e ID Cívico: o dispositivo mostra a frase, pergunta se quer passphrase (digitada 2 vezes) e **sorteia 3 posições distintas** para o usuário digitar no dispositivo. Errou: mostra a frase de novo e sorteia outras posições (até 5 tentativas); cancelou: nada é salvo. Sem a carteira pronta, o dispositivo **recusa** `GetCivicId` e `GetIdentityRequest` (a Suite não consegue pular o backup).
- **Restaurar** (`RestoreWallet`): o usuário digita as 24 palavras **no dispositivo**; o dispositivo valida lista e checksum, pede a passphrase, **mostra o endereço** e pede confirmação (uma frase errada gera outra carteira em silêncio, então o usuário compara). **Verificar backup** (`CheckWalletBackup`, como o "Check backup" do Trezor Safe 3): redigitar a frase; o dispositivo só diz se confere.
- **Apagar** (`Wipe` ou excesso de PIN) destrói a carteira do dispositivo; ela se recupera pela frase.
- **Exceção ao princípio 3** ("chaves nunca saem"): a frase sai **só para o humano**, pela tela, uma vez, e nunca para o host.

## Alternativas consideradas
- **Sem backup:** incompatível com saldo real.
- **Conta multi-assinatura de vários dispositivos:** exige conta M-de-N no livro; fica para depois.
- **Seed dividida (SLIP-39):** possível como extensão.
- **secp256k1 e caminhos BIP-44:** interoperaria com carteiras Bitcoin, mas o Teko usa P-256 em tudo; **importar a frase em outra carteira não dá o mesmo endereço**.

## Riscos e limites
- **Quem tem a frase tem o dinheiro:** phishing, fotos e papel perdido continuam sendo o principal risco; a Suite não tem campo de frase de propósito.
- **Passphrase esquecida = fundos perdidos** (avisos no dispositivo e na Suite).
- **Emulador sem isolamento de hardware:** a frase passa por um terminal comum (o emulador limpa a tela depois de cada página, mas não há garantia de hardware).
- Implementação própria da derivação SLIP-10 (validada com os vetores oficiais) sobre o crate `bip39`; **sem auditoria**.
- A carteira ainda **não assina transações** (`SignTransaction` é o próximo marco, M5b).
