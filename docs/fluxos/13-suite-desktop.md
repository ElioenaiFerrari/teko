# Suite desktop

← [Índice da documentação](../README.md)

## Resumo
A **Teko Suite** é o app Tauri v2 (interface em HTML/CSS/JS, regras em Rust). Ela conversa com o dispositivo, o emissor e a rede, mas **nunca** tem campo de PIN nem vê chaves.

## Quem participa
Interface, comandos Tauri, `teko-client`, dispositivo, quadro e rede.

## Diagramas

![Arquitetura da Suite: interface, comandos Tauri, cliente e dispositivo; a feature dev-simulation e o acesso ao quadro e à rede](../diagramas/suite-arquitetura.png)

*Fontes: [SVG](../diagramas/suite-arquitetura.svg) · [HTML](../diagramas/suite-arquitetura.html)*

## Passo a passo
1. `make emulator` (terminal 1): o PIN é digitado **aqui**.
2. `make suite` (terminal 2): onboarding guiado (inclui a **Carteira** e a frase de recuperação), identidade, **Carteira** (endereço e saldo de teste), assinar, votar e **Resultados**. Para ter saldo de teste: `make fund ADDR=<seu endereço> AMOUNT=1000`.
3. Sem emulador ligado a Suite abre no Explorador público.
4. Pré-visualização só visual: `make preview` (usa backend falso fora do pacote).

## Tela Início
É a tela de abertura depois de desbloquear. Reúne, sem repetir lógica das outras telas: **saldo**, **bens**, **stake**, **votos pendentes** (pleitos abertos em que você pode votar e ainda não votou), o **ID Cívico**, o **endereço da carteira** e lembretes de segurança, com atalhos para cada área. Cada bloco carrega sozinho: se um falhar (por exemplo, não há livro de economia na rede), os outros continuam. Não tem nenhuma ação sensível própria.

## Tela Bens (ambiente de teste)

![Bens na Suite: a tela chama o comando de negócio, o seu dispositivo mostra e assina o consentimento, o órgão e a contraparte simulados assinam e o livro local executa a troca atômica](../diagramas/bens-suite.png)

*Fontes: [SVG](../diagramas/bens-suite.svg) · [HTML](../diagramas/bens-suite.html)*

Mostra os bens em nome da sua carteira e permite **vender** ou **comprar** de uma contraparte simulada, além de pedir ao **órgão simulado** que registre um bem de teste, penhore ou libere. O **seu consentimento** é assinado no dispositivo; o resto é simulado e só existe com `dev-simulation`. Decisão: [ADR-0026](../adr/0026-bens-na-suite-com-orgao-e-contraparte-simulados.md).

## Tela Validadores (ambiente de teste)
Mostra saldo livre, **seu stake**, saídas pendentes e a lista de validadores com a **parcela do poder de voto**; permite **travar**, **sair** e **resgatar**, sempre com a confirmação no dispositivo ([ADR-0027](../adr/0027-stake-assinado-no-dispositivo.md)). O livro é o mesmo mundo de teste local da tela Bens; a ligação com o conjunto de validadores do consenso é o M6.

## O que é verificado e por quem
A Suite confere atestação, firmware e a cadeia do nó; as confirmações sensíveis são do dispositivo.

## O que pode dar errado
Resultados antigos ou eleição trocada: a Suite amarra o cache à primeira entrada do quadro. Dados de teste só aparecem no build de desenvolvimento.

## Onde está no código
`apps/suite/src-tauri/src/commands.rs`, `apps/suite/ui`, `crates/teko-client`, `crates/teko-transport`.

## Testes que cobrem
`crates/teko-client/tests/e2e.rs` (onboarding ponta a ponta sem janela) e `scripts/release-check.sh` (o build de produção não tem `dev-simulation` nem botões automáticos).

## Limites honestos
Botões automáticos do emulador existem apenas sob a feature `test-buttons`, nunca no build normal.
