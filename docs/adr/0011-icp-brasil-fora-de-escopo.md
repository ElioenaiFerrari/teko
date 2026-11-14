# ADR-0011 — Assinatura com validade jurídica (ICP-Brasil): fora de escopo

**Status:** Proposto · **Decisão:** registro de escopo

## Contexto
A assinatura de documentos (`SignDocument`) deste projeto é uma assinatura criptográfica verificável pela chave de identidade, **sem validade jurídica** própria.

## Decisão proposta
Fora do escopo da Fase 1. Anotado como **trabalho futuro**: integração com a cadeia ICP-Brasil (certificados A3/A5, carimbo do tempo, políticas de assinatura CAdES/PAdES), que exige AC credenciada e hardware homologado.

## Consequências
A UI da Suite deve deixar claro que a assinatura é "criptográfica do Teko", não "assinatura qualificada".
