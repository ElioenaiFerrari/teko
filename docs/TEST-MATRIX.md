# Matriz de testes: invariante → teste que a cobre

Rastreabilidade do THREAT-MODEL (que usa nomes `prop_*` ilustrativos) para os testes reais. Rode tudo com `make test`.
Legenda: **P** = `proptest` (muitos casos), **U** = unitário, **E** = ponta a ponta por TCP (`crates/teko-client/tests/e2e.rs`).

## Invariantes exigidas no spec

| Invariante | Onde | Teste | Tipo |
|---|---|---|---|
| PIN errado N vezes apaga o dispositivo | device-core | `wrong_pin_n_times_wipes_the_device` (N de 3 a 8; confere que a chave raiz sumiu do armazenamento) | P |
| | e2e | `eight_wrong_pins_wipe_the_device_over_the_wire` | E |
| Acerto antes do limite zera o contador | device-core | `correct_pin_resets_the_counter` | P |
| Atraso exponencial entre tentativas | device-core | `exponential_delay_between_attempts`, `config::backoff_is_exponential_and_capped` | U |
| Tentativa é contada **antes** de comparar o PIN | device-core | `counter_is_persisted_before_the_pin_is_checked` | U |
| Firmware sem M-de-N é recusado | attestation | `accepted_iff_m_distinct_valid_signers`, `duplicate_signatures_count_once`, `unknown_signers_do_not_count`, `tampering_invalidates_signatures` | P |
| | device-core | `firmware_without_m_of_n_is_refused` | U |
| | e2e | `firmware_without_m_of_n_never_reaches_the_device` (Suite recusa; Suite maliciosa é barrada pelo bootloader) | E |
| Downgrade é recusado | attestation | `downgrade_is_refused` | P |
| | device-core | `downgrade_is_refused_and_upgrade_accepted` | U |
| | e2e | `full_onboarding_and_daily_use` (v2 → v1 recusado na Suite **e** no dispositivo) | E |
| Firmware corrompido é recusado | device-core / e2e | `corrupted_blob_is_refused_and_not_installed` / (mesmo e2e acima) | U/E |
| Nenhuma mensagem expõe chave privada ou PIN | protocol | `no_secret_field_names` (guarda estrutural sobre os campos) | U |
| | device-core | `no_response_exposes_secrets` (sequências aleatórias de pedidos; procura chaves, hash do PIN e o PIN nos bytes enviados) | P |
| IDs pairwise de serviços diferentes são distintos | crypto | `pairwise_ids_differ_across_services_and_keys`, `pairwise_is_stable_and_canonical` | P |
| | device-core | `pairwise_ids_are_stable_per_service_and_distinct_across_services` | U |
| ID Cívico: checksum válido e detecta erro de digitação | crypto | `civic_id_valid_and_detects_single_typos` | P |
| Assinatura só depois da confirmação no dispositivo | device-core | `no_signature_without_physical_confirmation` (rejeitar/expirar; `ButtonAck` do host não vale) | U |
| | e2e | `rejecting_on_the_device_prevents_any_signature` | E |
| O hash exibido no dispositivo é o hash assinado | device-core | `displayed_hash_equals_signed_hash` | P |
| | e2e | `full_onboarding_and_daily_use` (hash na tela == SHA3 do arquivo == hash do bundle) | E |

## Outras garantias testadas

| Garantia | Teste |
|---|---|
| Parser nunca entra em pânico; frame corrompido é detectado | protocol: `decode_never_panics`, `single_bit_flip_is_detected`, `decode_arbitrary_payload_never_panics`, `roundtrip` |
| Separação de domínio (tags) | crypto: `domain_separation`, `tag_and_part_boundaries_are_unambiguous`, `sign_verify_and_domain_binding` |
| Voto separado da identidade (segredo do eleitor não deriva da raiz) | device-core: `voter_commitment_is_independent_of_identity`; crypto: `nullifiers_are_per_election_and_unlinkable_to_commitment` |
| `SignBallot` nunca assina (stub) | device-core: `ballot_is_a_stub_that_never_signs` |
| Divulgação seletiva não vaza a data de nascimento | crypto: `selective_disclosure_roundtrip`; device-core: `attribute_proof_discloses_only_the_predicate` |
| Credencial presa à chave do dispositivo; emissor falso recusado | device-core: `credential_for_another_key_or_forged_issuer_is_refused`; issuer: `rejects_bad_proof_of_possession_forged_attestation_and_replayed_nonce` |
| Atestação presa ao desafio; cadeia falsa/revogada recusada | attestation: `genuine_device_verifies_and_nonce_is_bound`, `forged_chain_is_rejected`, `revoked_batch_is_rejected`; e2e `device_from_an_unknown_authority_fails_attestation` |
| Estados inválidos não têm efeito | device-core: `operations_rejected_in_wrong_state`, `locked_after_restart_until_pin_entered` |
| Desbloqueio expira; polling não o renova | device-core: `unlock_expires_after_idle_timeout`, `host_polling_does_not_keep_the_device_unlocked` |
| Polling não redesenha a tela | device-core: `passive_polling_does_not_redraw_the_screen` |
| Resumo malicioso (controle/bidi) recusado | device-core: `malicious_summary_is_refused`; `text::sanitize_rejects_spoofing` |
| Wipe exige confirmação e destrói tudo (mantém contador de firmware e atestação de fábrica) | device-core: `wipe_needs_confirmation_and_destroys_everything` |
| Conexão ociosa é derrubada e libera a "porta USB" | device-emulator: `idle_connection_is_dropped_and_the_port_is_freed` |
| Build normal sem botões automáticos | `make release-check` (script) |
| Build reproduzível | `make repro` (script) |

## Fase 2 (parcial): manifesto, ciclo de vida e quadro de avisos

| Garantia | Teste |
|---|---|
| Manifesto só vale com M-de-N assinaturas distintas; duplicadas e estranhos valem zero | election: `accepted_iff_m_distinct_valid_signers`, `duplicates_and_outsiders_count_zero` (P) |
| Qualquer alteração do conteúdo invalida as assinaturas; o hash ignora assinaturas | election: `tampering_invalidates_signatures`, `hash_ignores_signatures_but_not_content` (P) |
| Opções, calendário e textos inválidos/enganosos (bidi, quebra de linha, índices com buraco, duplicatas) são recusados | election: `structural_rules_are_enforced`, `text::rejects_spoofing_and_empty` |
| Só arestas legais do ciclo de vida são aceitas (modelo completo 8×8) | election: `only_legal_edges_are_accepted` (P) |
| Pular fase, voltar, repetir, buraco de sequência, outra eleição, outro manifesto, voltar no tempo, abrir cedo, fechar cedo | election: `skipping_going_back_and_repeating_are_refused`, `transitions_are_bound_to_one_election_and_one_manifest`, `schedule_gates_opening_and_closing` |
| Cancelamento exige motivo e encerra; não se cancela após a apuração | election: `cancellation_needs_a_reason_and_ends_the_election` |
| Provas de inclusão e de consistência válidas para **todo** tamanho/prefixo (RFC 6962/9162) e falham com folha, índice, caminho ou histórico adulterados | bulletin: `every_leaf_of_every_size_has_a_valid_inclusion_proof`, `every_prefix_has_a_valid_consistency_proof`, `proofs_fail_when_anything_is_wrong`, `growth_is_always_provable` (P) |
| Head assinado amarra tamanho, raiz e operador | bulletin: `head_signature_binds_size_root_and_operator` |
| Reescrever o histórico é detectado por um head fixado antes | bulletin: `old_heads_stay_provable_as_the_log_grows_and_rewrites_are_caught`, `rewriting_history_is_caught_by_a_previously_pinned_head` |
| Arquivo truncado/corrompido é **recusado**, não reparado; adulteração muda a raiz | bulletin: `truncated_or_corrupt_log_files_are_refused_not_repaired`, `flipping_a_stored_byte_changes_the_root_...` |
| Auditor independente refaz a eleição só com o quadro e a política; transição forjada não passa mesmo com operador cúmplice | bulletin (integração): `an_auditor_replays_the_whole_election_from_the_board`, `a_forged_transition_does_not_pass_the_audit_...` |

## O que **não** está coberto por teste automático (honestidade)

- **Tempo constante da comparação do PIN:** o código usa `subtle::ConstantTimeEq`, mas não há teste de temporização.
- **`--listen` só em loopback:** verificado manualmente (recusa `0.0.0.0`); falta teste automatizado.
- **A janela da Suite (Tauri) e a UI JS:** não há teste automatizado de interface; a UI foi exercitada com o mock no navegador e o app real só foi aberto até conectar no emulador.
- **`no_std` em alvo embarcado real:** o CI compila o núcleo para `thumbv7em-none-eabihf` (passou em 2026-11-14); não há teste de execução em hardware.
- **Quadro de avisos de operador único:** sem testemunhas independentes, um operador pode mostrar logs diferentes a pessoas diferentes (*split view*). Só um head fixado e comparado por terceiros detecta isso; não há essa comparação implementada.
- **Fuzzing contínuo do parser:** só `proptest`; sem `cargo-fuzz`.
