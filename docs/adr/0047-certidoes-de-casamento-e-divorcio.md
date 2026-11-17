# ADR-0047 — Certidões de casamento e de divórcio

**Status:** Aceito provisoriamente (2026-11-16, dono do produto) e **implementado** · Usa o modelo de credenciais na chain do [ADR-0037](0037-credenciais-na-chain-emitidas-por-orgaos.md).

## Contexto
Pediu-se suporte a duas credenciais do registro civil: a **certidão de casamento** e a **certidão de divórcio**. O modelo de credenciais já cobre o essencial (órgão M-de-N, janela de contestação, revogação); faltavam os tipos, as regras dos dados e o fluxo que liga o divórcio ao casamento.

## Decisões do dono
- **Titulares:** uma credencial para **cada cônjuge** (mesma matrícula, cada uma nomeando o outro), pois uma credencial tem um titular só e cada um precisa provar o casamento sozinho.
- **Divórcio:** a certidão de divórcio **referencia** a matrícula do casamento e o órgão **revoga** as certidões de casamento: o casamento deixa de valer e o divórcio fica como prova.
- **Privacidade:** tipos **privados**: a chain guarda só o compromisso; os dados ficam com o titular, que escolhe o que revelar. Estado civil é dado pessoal sensível.

## Decisão técnica
1. **Tipos** `certidao-casamento` e `certidao-divorcio` no genesis da devnet (`DEVNET_CRED_TYPES`): órgão de 3 membros, 2-de-3, janela de contestação, `public = false` e **validade 0** (não vencem; só a revogação as encerra).
2. **Dados** (`teko-credentials::civil`, regras puras e canônicas, como a candidatura do ADR-0040). Casamento: `matricula;data;regime;nome;conjuge;cartorio`. Divórcio: `matricula;data;forma;nome;conjuge;cartorio`. Matrícula de 1 a 40 caracteres (letras, dígitos, ponto, hífen); data real `AAAA-MM-DD` (bissexto incluído); regime: comunhão parcial, comunhão universal, separação total ou participação final; forma: judicial ou extrajudicial; os dois cônjuges têm nomes diferentes. `divorce_ends` confere que o divórcio é da **mesma matrícula e do mesmo par** (titular e cônjuge) e **depois** do casamento.
3. **Emissor** (aba Certidões). *Casamento:* valida tudo antes da primeira confirmação, recusa data futura e as mesmas chaves, e propõe uma certidão para cada cônjuge (duas confirmações no dispositivo); os dados de cada titular vão para `dev/issued` (entrega fora da chain), como nas outras credenciais privadas. *Divórcio:* o cartório só encerra casamentos **que ele emitiu** e que estão com as duas certidões válidas; monta o divórcio de cada cônjuge **a partir da certidão de casamento** (nomes e matrícula não podem divergir), confere `divorce_ends`, propõe as duas certidões de divórcio e **só depois** dá o voto de revogação das duas de casamento; os outros membros completam o quórum.
4. **Simulador de teste.** `make devnet-cosign` também completa **revogações** que um membro real iniciou (os membros de teste votam; só dev).

## Limites honestos
- A chain não valida os dados (privados): a coerência entre divórcio e casamento é garantida **pelo Emissor** e por quem verifica o que o titular mostra, não pelo livro de economia.
- **Só se encerra um casamento emitido pelo mesmo cartório** que tem os dados; certidões de outros emissores precisariam da revelação dos dados pelo titular.
- Se a revogação não for concluída pelo quórum, o casamento segue válido (a certidão de divórcio já existe): o órgão precisa terminar o processo.
- Sem validade jurídica; sem outros atos do registro civil (óbito, união estável, averbações, anulação).
- O titular entrega a quem verifica os dados (e o sal) e a chain mostra se a credencial está válida ou revogada; sem prova de conhecimento zero sobre atributos.
