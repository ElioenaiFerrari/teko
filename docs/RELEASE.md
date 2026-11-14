# Releases e verificação (como o usuário confere o que está rodando)

Status: **parcialmente implementado**. Cada item diz o que existe hoje e o que falta.

## 1. Código aberto e build reproduzível

| Item | Estado |
|---|---|
| Código aberto, dependências travadas (`Cargo.lock`, `--locked`) | Existe |
| Toolchain fixada (`rust-toolchain.toml`, Rust 1.100.0) | Existe |
| `make repro`: compila `device-emulator` e `issuer-cli` duas vezes, em diretórios diferentes, e compara os hashes | **Existe e passou** (mesmo hash, nesta máquina) |
| Reprodutibilidade entre **máquinas/SOs diferentes** | **Não verificada** (só um ambiente testado) |
| Reprodutibilidade da **Suite (Tauri)** | **Não verificada** (inclui assets embutidos e bibliotecas do sistema; precisa de teste próprio) |
| `make release-check`: o build normal não contém os botões automáticos (`test-buttons`) | **Existe e passou** |

Como reproduzir você mesmo: `git clone` do commit da release → `make repro` e compare o SHA-256 impresso com o publicado.

## 2. Firmware (assinado M-de-N)

O dispositivo só aceita firmware com **assinaturas de pelo menos M de N instituições independentes** e recusa versão menor que a instalada. Três conferências independentes:

1. **A Suite** confere antes de enviar e mostra a *impressão digital*.
2. **O dispositivo** confere de novo (bootloader) e mostra a mesma impressão digital na sua tela: **compare as duas**.
3. **Você, por fora da Suite:**
   ```bash
   cargo run -p issuer-cli -- verify-firmware --manifest ARQ.manifest --blob ARQ.blob [--installed N]
   # ou, no ambiente de teste:  make verify-fw
   ```
   Imprime a versão, a impressão digital, quantas assinaturas válidas e se o hash confere.

Hoje as três chaves de firmware ficam todas em `dev/` (teste). Em produção, cada uma deve estar com uma instituição diferente; senão o M-de-N não protege nada.

## 3. Dispositivo genuíno

A Suite envia um desafio aleatório; o dispositivo assina com a chave de lote e a Suite confere a cadeia até a raiz da autoridade pinada (`trust.txt`). Para não confiar na Suite, reverifique com o `issuer-cli` (ainda **não há** subcomando de atestação independente; é uma lacuna conhecida).

## 4. Release da Suite (assinatura dos artefatos)

**Não implementado.** Falta decidir a ferramenta e quem guarda a chave. Proposta para revisão: publicar `SHA256SUMS` junto de uma assinatura (minisign ou cosign) em um canal independente do download, com a chave pública conhecida por outro caminho. Veja as perguntas abaixo.

## 5. Dependências

`deny.toml` (cargo-deny: advisories, licenças, fontes) e o workflow de CI estão escritos, mas **ainda não foram executados** (cargo-deny não está instalado aqui e o repositório não tem remoto). Rode uma vez e ajuste a lista de licenças antes de confiar.

## 6. Perguntas em aberto

1. **Ferramenta de assinatura dos releases da Suite** (minisign, cosign/sigstore, GPG). *Recomendação:* minisign (simples, uma chave pública pequena), com a chave offline. **PENDENTE (dono do produto)**.
2. **Quem são as N instituições do firmware** e como as chaves são custodiadas (HSM, cerimônia de geração). Fora do escopo do estudo, mas define o valor real do M-de-N. **PENDENTE**.
3. **Alvo de reprodutibilidade:** exigir reprodução em Linux **e** macOS no CI? *Recomendação:* sim para o firmware (núcleo `no_std`); Suite em best effort. **PENDENTE**.
4. **Subcomando independente de atestação** (`issuer-cli verify-device`, falando com o dispositivo sem a Suite). *Recomendação:* implementar antes da fase de hardware. **PENDENTE**.
