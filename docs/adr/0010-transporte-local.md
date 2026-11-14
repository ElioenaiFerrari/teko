# ADR-0010 — Transporte local: socket Unix vs TCP loopback

**Status:** Aceito provisoriamente (2026-11-13: dono do produto mandou seguir; recomendação adotada: TCP loopback 127.0.0.1) · revisável

## Contexto
`trait Transport` abstrai o canal; USB HID virá depois. Precisamos de uma implementação local para o emulador.

## Alternativas
| | Unix domain socket | TCP loopback (`127.0.0.1`) |
|---|---|---|
| Portabilidade | Windows limitado | **Todas as plataformas** |
| Controle de acesso | Permissões do arquivo | Qualquer processo local conecta |
| Simplicidade | Alta | Alta |
| Realismo (USB = canal exclusivo) | Maior | Menor |

## Recomendação
**TCP loopback** (portável, Tauri multiplataforma) **escutando só em `127.0.0.1`** com porta configurável. Como o host já é não confiável, a segurança do canal não é premissa de nenhuma garantia: tudo que importa é decidido no dispositivo. O mesmo framing do DEVICE-PROTOCOL é usado sobre qualquer transporte.

## Consequências
Outros processos locais podem conectar ao emulador; aceitável pelo modelo de ameaças (host não confiável), mas o emulador deve aceitar **uma** conexão por vez e logar conexões.
