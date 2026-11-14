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
| **Elegibilidade por jurisdição**: Presidente é de todos; Governador só da UF; Prefeito só do município (propriedade sobre qualquer UF/município) | election: `president_is_national_governor_state_mayor_municipal`, `scopes_and_domiciles_are_validated`, `eligibility_matches_the_jurisdiction_exactly` (P) |
| **Vários pleitos independentes**, cada um com suas datas e seu estado; abrir/cancelar um não afeta os outros | election: `contests_run_independently_with_their_own_dates`, `opening_one_contest_does_not_open_another_...`, `cancelling_one_contest_leaves_the_others_running` |
| Transição amarrada ao pleito exato (hash); pleito desconhecido recusado | election: `transitions_are_bound_to_one_exact_contest` |
| Cargo incompatível com a jurisdição recusado; revisões só acrescentam pleitos (2º turno), nunca alteram/removem | election: `structural_rules_are_enforced`, `revisions_may_add_contests_but_never_change_or_drop_one`, `a_runoff_added_in_a_later_revision_keeps_earlier_history_valid` |
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

## Fase 2, etapa 3: o dispositivo e a votação por pleito

| Garantia | Teste |
|---|---|
| Eleição exige M-de-N (manifesto e transições); manifesto forjado ou sem governança configurada é recusado, sem prompt | device-core: `an_election_needs_m_of_n_governance_signatures_and_a_configured_policy`; e2e `a_forged_or_old_election_never_reaches_a_vote` (Suite e dispositivo) |
| Fase e elegibilidade por pleito, calculadas pelo dispositivo a partir do domicílio da credencial | device-core: `loading_an_election_reports_each_contest_phase_and_eligibility`; e2e `election_flow_president_for_all_governor_and_mayor_by_domicile` |
| Presidente para todos; governador de outro estado recusado; prefeito/governador fora da fase recusados; nenhum prompt para pedido recusado | device-core: `you_cannot_vote_outside_your_jurisdiction_or_outside_the_voting_phase`; e2e idem |
| A tela mostra cargo, jurisdição e o nome do candidato **do manifesto**; a assinatura amarra eleição, pleito, versão do pleito e opção | device-core: `the_ballot_screen_shows_the_candidate_from_the_manifest_and_the_signature_verifies`; election: `signature_binds_every_field` |
| Um voto por pleito, persistido; rejeitar na tela não gasta o voto | device-core: `a_device_votes_once_per_contest_and_remembers_across_restarts` |
| Pleitos abrem em datas próprias; **histórico mais antigo é recusado** (anti-rollback) | device-core: `contests_open_on_their_own_dates_and_old_history_is_refused` |
| Chaves de eleitor distintas por pleito, estáveis, nunca a chave de identidade; raiz de eleitor ≠ raiz de identidade | device-core: `voter_keys_are_per_contest_stable_and_never_the_identity_key`; crypto: `deterministic_and_independent_across_contests_elections_and_roots`, `voter_keys_are_domain_separated_from_the_identity_key` |
| Eleição armazenada é re-verificada a cada uso (adulterar o armazenamento não ajuda) | device-core: `the_stored_election_is_reverified_so_tampering_with_storage_does_not_help` |
| Wipe remove eleição e registro de voto | device-core: `wiping_removes_the_election_and_the_vote_record` |
| Nenhuma resposta vaza chave/segredo (inclui `GetVoterKey`/`SignBallot`) | device-core: `no_response_exposes_secrets` |
| Credencial traz o domicílio e rejeita domicílio inválido; eleição de exemplo reproduz com 4 fases | issuer: `credential_carries_the_electoral_domicile_and_rejects_a_bad_one`, `demo_election_is_signed_by_governance_and_replays_with_four_phases` |
| Governança no `trust.txt` é tudo-ou-nada | attestation: `governance_is_all_or_nothing` |
| Tela do dispositivo quebra por palavra (nome de candidato nunca cortado) | device-core: `text::wrap_breaks_on_words_and_hard_splits_only_what_cannot_fit` |

## Fase 2, etapa 4: apuração e resultados ao vivo

| Garantia | Teste |
|---|---|
| A apuração é função pura do log: conta por opção, ranqueia (desempate por posição) com balde "Outros" | election: `counts_per_option_and_ranks_with_an_others_bucket` |
| "Tempo" é a ordem do log: cédula só vale com o pleito em VotacaoAberta naquele ponto | election: `a_ballot_counts_only_while_its_contest_is_open_in_log_order`, `the_tally_is_deterministic_and_depends_on_log_order` |
| Uma cédula por chave por pleito, a primeira vale | election: `one_ballot_per_voter_key_per_contest_first_wins`; proptest `totals_always_add_up` |
| Toda rejeição é contada por motivo (assinatura, versão do pleito, opção, duplicada, malformada, pleito desconhecido); nada some | election: `every_invalid_ballot_is_counted_by_reason_never_dropped_silently` |
| Governança forjada no log (transição com 1 assinatura, revisão que altera pleito) é ignorada e contada, sem parar a apuração | election: `forged_governance_entries_are_ignored_and_counted_without_stopping_the_count`, `ballots_before_any_valid_manifest_are_orphans` |
| Política de visibilidade mascara parciais até o fechamento; cancelado oculta tudo | election: `visibility_policy_masks_the_public_view_until_the_voting_closes` |
| Apuração incremental == replay completo a cada passo | election: `incremental_snapshots_equal_a_full_replay_at_every_point`; admin: `incremental_feeding_follows_the_board_and_equals_a_full_replay` |
| Raiz Merkle incremental (O(log n)) == raiz recursiva em todo tamanho | bulletin: `the_incremental_frontier_equals_the_recursive_root_at_every_size` |
| Leitor ao vivo ignora cauda de escrita em andamento; escritor continua estrito; dano no meio é erro | bulletin: `a_live_reader_ignores_an_in_progress_tail_but_a_writer_still_refuses_it`; admin: `tallying_while_a_writer_is_mid_append_still_works` |
| Publicar é idempotente; simulação gera eleitores novos a cada rodada e marca o quadro como simulado; streaming ao vivo | admin: `publish_is_idempotent_...`, `simulated_voters_are_counted_valid_ranked_and_labelled_as_simulated`, `live_simulation_streams_ballots_that_a_reader_can_follow` |
| **Voto real do dispositivo (Presidente e Governador de SP, chaves distintas) entra no quadro e é contado uma vez** | e2e: `election_flow_president_for_all_governor_and_mayor_by_domicile` |

## Fase 2, etapa 5: registro anônimo (ADR-0008)

| Garantia | Teste |
|---|---|
| O RFC 9474 está certo: codificação PSS, mensagem cega, assinatura cega e token final reproduzem **byte a byte** o vetor A.1; adulterar qualquer parte é recusado | registration: `rfc9474.rs` (5 testes) |
| Fluxo completo com RSA-2048 real; cegamento determinístico (retry seguro); token preso à mensagem, à chave e aos bytes; sementes diferentes dão valores cegos sem relação; resposta forjada nunca vira token | registration: `flow.rs` (3 testes) |
| Pleito com chave de registro só conta chaves registradas; token de outro pleito/chave não vale; registro fora da fase ou repetido é recusado e contado; **sem verificador, falha fechado**; pleito sem chave não muda | election: `contests_requiring_registration_count_only_registered_keys`, `a_token_is_useless_for_another_key`, `without_a_verifier_registration_contests_fail_closed`, `contests_without_a_registration_key_are_unchanged` |
| O dispositivo confirma na própria tela, recusa fora do domicílio/fase/sem chave de registro, não adivinha sem a matemática cega, e resposta inválida da autoridade nunca vira token | device-core: `registration::*` (5 testes) |
| Quadro: só chaves registradas contam; token de um pleito não vale no outro; simulador registra de verdade (fluxo cego completo) | admin: `only_registered_keys_count_and_a_token_is_bound_to_its_contest`, `simulated_voters_...` |
| **Ponta a ponta com dispositivo real:** a autoridade vê o cidadão e um valor cego (nunca a chave de eleitor; o livro dela não a contém), recusa pedido adulterado e pedido ligado a outro pleito, e o voto registrado é contado | e2e: `election_flow_president_for_all_governor_and_mayor_by_domicile` |

## O que **não** está coberto por teste automático (honestidade)

- **Anonimato real do registro:** a propriedade de cegamento vem da matemática do RFC (vetor oficial) e o teste confere que o livro e o pedido não contêm a chave; **não há teste estatístico nem de canal/tempo**. No protótipo, autoridade e quadro rodam no mesmo processo.
- **Implementação RFC 9474 própria sem auditoria independente.**

- **Tempo constante da comparação do PIN:** o código usa `subtle::ConstantTimeEq`, mas não há teste de temporização.
- **`--listen` só em loopback:** verificado manualmente (recusa `0.0.0.0`); falta teste automatizado.
- **A tela de Resultados ao vivo (JS):** só exercitada com o mock no navegador (reconciliação de DOM, animação, abas). A apuração que a alimenta é testada; o desenho, não.
- **A janela da Suite (Tauri) e a UI JS:** não há teste automatizado de interface (a tela de votação foi exercitada só com o mock no navegador; ali um laço de re-render travou a página e foi corrigido); a UI foi exercitada com o mock no navegador e o app real só foi aberto até conectar no emulador.
- **`no_std` em alvo embarcado real:** o CI compila o núcleo para `thumbv7em-none-eabihf` (passou em 2026-11-14); não há teste de execução em hardware.
- **Quadro de avisos de operador único:** sem testemunhas independentes, um operador pode mostrar logs diferentes a pessoas diferentes (*split view*). Só um head fixado e comparado por terceiros detecta isso; não há essa comparação implementada.
- **Fuzzing contínuo do parser:** só `proptest`; sem `cargo-fuzz`.
