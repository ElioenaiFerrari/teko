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

## Fase 2: fechamento no prazo e explorador

| Garantia | Teste |
|---|---|
| No prazo a governança (simulada, 2 de 3) fecha o pleito; antes não fecha nada; é idempotente; cédula anexada depois é recusada pela apuração pública e o fechamento não é contado como governança ignorada | admin: `at_the_deadline_governance_closes_the_contest_and_the_log_locks_the_vote` |
| O dispositivo recusa votar/registrar após `voting_closes`, sem pedir confirmação | device-core: `the_device_itself_refuses_to_vote_after_the_last_allowed_moment` |

## Fase 2, etapa 6: rede de votos em BFT institucional (ADR-0034)

| Garantia | Teste |
|---|---|
| Blocos encadeiam e a réplica reconstrói exatamente o mesmo log Merkle, verificando tudo | chain: `blocks_chain_and_a_second_replica_rebuilds_exactly_the_same_merkle_log` |
| Finalidade exige mais de 2/3 das instituições (4: 3 decidem, 2 não; 3: só as 3) e nada é commitado sem isso | chain: `finality_needs_more_than_two_thirds_of_the_institutions` |
| Certificado ausente, de outro bloco, com voto repetido, de estranho ou de outra rede é recusado | chain: `a_certificate_is_not_transferable_forged_or_made_by_outsiders` |
| Só o proponente que o certificado nomeia pode ter proposto o bloco | chain: `only_the_proposer_the_certificate_names_can_have_proposed_the_block` |
| Qualquer campo adulterado (chain_id, altura, pai, proponente, entradas, raiz, tamanho, tipo, bytes extras) é recusado mesmo com certificado válido | chain: `every_tampered_field_is_refused` |
| O horário só avança, e o validador só vota perto do próprio relógio (60 s) | chain: `block_time_only_moves_forward_and_a_validator_only_votes_near_its_clock` |
| Mempool sem repetição; o que o bloco leva sai dele; nunca há bloco vazio | chain: `the_mempool_dedupes_and_what_a_block_carries_leaves_it`, `a_block_is_never_empty_and_the_first_one_ties_to_the_chain_id` |
| 3 nós decidem o mesmo bloco com certificado completo; cada réplica verifica sozinha; duplicatas descartadas na rede toda | node: `three_nodes_decide_blocks_and_every_replica_verifies_to_the_same_log` |
| **Nenhum bloco nasce sozinho**: a rede ociosa fica na altura 0, uma entrada gera um bloco e depois ela para de novo | node: `no_block_is_made_on_its_own_the_chain_only_grows_with_entries` |
| Com 4 instituições uma pode cair, mesmo no turno de proponente (a rodada seguinte assume) | node: `with_four_institutions_one_can_be_down_even_when_it_is_the_proposer` |
| **Sem quórum a cadeia pausa (nunca bifurca)** e retoma quando o quórum volta | node: `without_a_quorum_the_chain_pauses_it_never_forks_and_resumes_when_the_quorum_returns` |
| Nó reiniciado reverifica seu log e recupera o atraso | node: `a_restarted_node_replays_its_log_and_catches_up` |
| Bloco com certificado sem quórum (2 de 4) é recusado por um nó e a cadeia não muda | node: `a_block_without_a_quorum_certificate_is_not_accepted_by_a_node` |
| Mainnet travada; perfil validado | profile: `mainnet_is_locked_everywhere`, `text_roundtrip_and_validation` |
| WAL (rodada e lock) sobrevive a reinício | node: `the_wal_remembers_the_round_and_the_lock_across_a_restart` |
| Eleição inteira na rede; cédulas cifradas ficam escondidas até o fechamento e os totais saem com 1 instituição fora | admin-cli: `network.rs` |

## Economia sem blocos vazios (ADR-0034)

| Garantia | Teste |
|---|---|
| O horário do bloco só avança, e a instituição só vota num horário perto do relógio dela (60 s); relógio 10 anos adiantado não move o horário | economy: `block_time_is_the_proposers_it_only_moves_forward_and_must_be_near_the_voters_clock` |
| A rede de economia ociosa fica na altura 0, uma transação gera um bloco em todos os nós, e depois para | node: `no_block_is_made_on_its_own_the_chain_only_grows_with_transactions` |

## Fase 2, etapa 7: reemissão, revogação e build de produção

| Garantia | Teste |
|---|---|
| **Substituído pelo ADR-0038:** a lista de revogação fora da chain, a `citizens.log` e o `issuer-cli issue/reissue/revoke` não existem mais (testes removidos); o que cobre a identidade agora está na seção "ID cívico na chain (ADR-0038)" abaixo | — |
| A Suite de produção não contém a autoridade/operador de teste (com controle do detector) | `scripts/release-check.sh` (`cargo tree` sem/com `dev-simulation`) |

## Fase 2, etapa 8: cédulas cifradas (ADR-0019)

| Garantia | Teste |
|---|---|
| Cédula válida verifica, ida e volta de bytes, tamanho esperado; limites de opções | ballot-crypto: `a_valid_ballot_verifies_roundtrips_and_has_the_expected_size` |
| Cédula inválida recusada: outro pleito/eleição/chave do eleitor, chave pública errada, nº de opções, prova trocada de lugar, cifrados reordenados, soma adulterada, **duas opções**, **nenhuma**, um "2" com prova 0/1, bytes malformados | ballot-crypto: `invalid_ballots_are_rejected`; `a_proof_is_bound_to_its_exact_ciphertext` |
| Decifração de limiar da soma homomórfica = contagem em claro, com **quaisquer 2 de 3**; 1 parte não basta; índices duplicados/zero recusados | ballot-crypto: `threshold_decryption_of_the_homomorphic_sum_matches_the_plain_count` |
| Cifra nova a cada vez (sem ligação entre cédulas do mesmo voto) | ballot-crypto: proptest `any_choice_verifies_and_changes_the_ciphertext_every_time` |
| Só pontos e escalares canônicos decodificam (achou um decodificador leniente com tag SEC1 inválida) | ballot-crypto: `only_canonical_points_and_scalars_decode` |
| DKG: todos concordam na chave; qualquer limiar reconstrói e decifra com provas; compromisso/parte/prova de posse adulterados, **chave maliciosa (rogue key)**, outra sessão, parâmetros: recusados e apontam o culpado | ballot-crypto: `dkg::tests::*` |
| Cerimônia **distribuída entre processos de nós**: registro na cadeia reverificável, partes salvas, decifração com provas por 2 de 3 | node: `the_key_ceremony_runs_across_the_nodes_and_leaves_a_verifiable_record_on_the_chain` |
| O dispositivo cifra e prova; o corpo assinado não nomeia a opção; provas valem só para esta eleição/pleito/chave; a decifração com a chave de teste mostra a escolha; a aleatoriedade não aparece em resposta; chave ruim/ >16 opções recusadas | device-core: `encrypted::*` |
| Cliente verifica assinatura e provas; ponta a ponta com dispositivo real e apuração pública que **conta mas não vê** | e2e: `an_encrypted_election_the_device_encrypts_the_vote_and_the_public_tally_cannot_see_it` |
| **Apuração:** nenhum voto por opção existe antes do resultado (nem com `live`); parte antes do fechamento recusada; 1 de 3 não revela; duplicada/forjada/de fora recusadas; 2 de 3 revelam totais **iguais** aos votos; incremental = replay | election: `nothing_about_the_distribution_exists_until...` |
| Ballots inválidos (assinatura, prova, em claro numa eleição cifrada, opção nomeada) não tocam as somas; só o válido conta | election: `invalid_encrypted_ballots_are_rejected_and_never_touch_the_sums` |
| Cerimônia forjada ou chave que não é a da eleição não revela nada | election: `a_forged_ceremony_or_a_key_that_is_not_the_elections_reveals_nothing` |
| **Devnet ponta a ponta:** cerimônia, eleição com a chave, reinício dos nós, cédulas cifradas sintéticas com provas reais, parciais invisíveis, 1 validador cai, fechamento, 2 de 3 revelam, **totais == votos plantados pelo simulador** | admin: `encrypted_ballots_stay_hidden_until_the_close_then_two_of_three_validators_reveal_the_exact_totals` |

## Livro-razão genérico (`teko-ledger`, ADR-0020)

| Invariante | Teste |
|---|---|
| Qualquer campo adulterado quebra a assinatura; cadeia errada, repetição e salto de nonce são recusados | ledger: `tampering_any_field_breaks_the_signature`, `rejects_wrong_chain_replay_and_nonce_gap` |
| Bloco com uma transação inválida não muda nada (atômico) | ledger: `a_failing_transaction_rejects_the_whole_block_atomically` |
| Módulos têm espaços de nomes separados; módulo 0 reservado e duplicado recusados | ledger: `modules_have_separate_namespaces`, `genesis_only_before_the_first_block_and_modules_are_validated` |
| Réplicas que aplicam os mesmos blocos chegam à mesma raiz; a ordem das transações muda a raiz | ledger (proptest): `replicas_agree`, `order_of_transactions_matters_for_roots` |
| Prova de inclusão do estado confere e recusa valor errado | ledger: `state_proofs_verify_and_reject_a_wrong_value` |
| Codificação canônica; payload grande recusado; altura errada recusada | ledger: `canonical_bytes_roundtrip_and_reject_garbage`, `oversized_payload_is_rejected`, `wrong_height_is_rejected` |

## Moeda (`teko-coin`, ADR-0021)

| Invariante | Teste |
|---|---|
| Faucet e alocações só na devnet; gênese inválida recusada | coin: `genesis_rules` |
| A recompensa cai pela metade e vai ao proponente; **o teto nunca é ultrapassado** | coin: `reward_halves_and_is_credited_to_the_proposer`, `the_cap_is_never_exceeded` |
| Valores em TEKO/ypy: `parse(format(x)) == x` (inclusive no teto); mais de 6 casas recusadas; a tela do dispositivo mostra o valor exato do `u64` assinado, sem arredondar (ADR-0029) | crypto (proptest): `amount::tests::roundtrip`, `roundtrip_any`, `parses`; device-core `tests.rs` (`Valor: 0,0005 TEKO`…) |
| Equivocação detectada vira um `Slash` por ofensa, enviado pelo próprio nó; o lock do BFT sobrevive ao reinício e não passa de uma altura a outra (ADR-0030) | node: `an_equivocation_becomes_one_slash_transaction_per_offence`, `the_wal_remembers_the_round_and_the_lock_across_a_restart`; bft: `a_restored_lock_survives_a_restart_and_is_dropped_on_a_fresh_height` |
| O explorador reexecuta e verifica a cadeia sozinho: bloco com certificado falso é recusado; saldos e oferta batem com o livro; a API só aceita GET (ADR-0031) | explorer: `a_block_with_a_forged_certificate_is_refused_and_the_index_keeps_its_chain`, `rewards_go_to_the_owner_and_the_supply_matches_the_ledger`; server: `the_api_is_read_only`, `the_explorer_verifies_the_chain_of_three_real_nodes_and_decodes_a_transfer` |
| A Suite não consegue trocar o bem que o dispositivo mostrou: o consentimento cobre o hash dos metadados; registro que não nomeia o bem, inválido ou de outra classe é recusado antes de mostrar; consentimento sobre outro hash é recusado pelo livro; registro exige hash de 32 bytes (ADR-0032) | device: `a_vehicle_consent_shows_plate_and_model_and_refuses_records_that_do_not_match`; assets: `a_consent_over_another_metadata_hash_is_refused`, `registration_needs_a_32_byte_metadata_hash`; client e2e: `selling_and_buying_an_asset_with_my_device_consent_in_the_devnet_world` |
| Esquema de metadados: RENAVAM com dígito verificador, placa, chassi, anos; registro canônico estrito; hash por classe e conteúdo | crypto: `asset_meta::tests::*` (inclui proptest) |
| Cada classe tem o seu órgão; venda bloqueada sem o registro ou com registro que não confere | assets: `a_vehicle_is_registered_by_its_own_organ_with_the_chassis_as_id`, `a_sale_is_blocked_when_the_metadata_record_is_missing_or_does_not_match`; node: `each_asset_class_has_its_own_organ_and_a_vehicle_registers_through_the_nodes` |
| A equivocação simulada penaliza o DONO do validador em `slash_bps` do stake em risco, uma vez por ofensa (cada simulação é outra ofensa) e a oferta continua conservada | devnet-sim: `the_simulated_equivocation_burns_a_tenth_of_the_owners_stake_each_time` |
| A verificação paralela do quadro não muda o resultado: conferir cédulas e registros em várias threads, e contar em ordem, dá exatamente o resultado sequencial (cédulas adulteradas, duplicadas, com prova ou assinatura inválida, veredito de outra chave ou de outro conteúdo é descartado) (ADR-0033) | election: `prechecked_feeding_gives_exactly_the_sequential_result`; admin-cli: `checking_ballots_on_many_threads_gives_the_sequential_result`, `feeding_in_steps_reports_progress_and_ends_at_the_same_state` |
| Reabrir a economia verificada baixa só os blocos novos e chega ao mesmo estado | node: `reopening_the_world_downloads_only_the_new_blocks` |
| Taxa por byte: abaixo do mínimo recusada; paga ao proponente; sem saldo recusada | coin: `fees_are_priced_per_byte_and_paid_to_the_proposer` |
| Transferência não cria nem destrói moeda; saldo insuficiente e valor zero recusados | coin: `transfers_move_funds_and_refuse_overdraft_and_zero` |
| Stake, desvinculação e resgate só após o prazo | coin: `stake_unbonding_and_claim` |
| Equivocação (prova do BFT) queima parte do stake e da desvinculação, **uma vez**; prova inválida, de outra rede ou fora da janela é recusada | coin: `equivocation_burns_stake_and_unbonding_once`, `bad_or_stale_evidence_is_refused` |
| Faucet limitada por intervalo e rotulada; desligada fora da devnet | coin: `faucet_is_rate_limited_labeled_and_devnet_only` |
| **Conservação da oferta** com qualquer sequência de operações (criado − queimado = saldos + stake + desvinculação ≤ teto) | coin (proptest): `supply_is_conserved_and_capped` |
| Réplicas com a moeda chegam à mesma raiz; payload malformado recusado | coin: `replicas_with_the_coin_agree_on_the_state_root`, `malformed_payloads_are_refused` |

## Bens (`teko-assets`, ADR-0022)

| Invariante | Teste |
|---|---|
| Registro só com o quórum do órgão (1 de 2 não basta; a mesma chave duas vezes conta uma; quem não é do órgão não completa) | assets: `registration_needs_the_registrar_quorum`, `unknown_class_is_refused` |
| Transferência exige dono + comprador + quórum; assinatura de estranho não vale como a do dono; um *relayer* pode enviar | assets: `transfer_needs_owner_buyer_and_registrar` |
| **Sem repetição:** aprovações antigas morrem quando o bem muda de versão; aprovações de outra rede não valem | assets: `consents_cannot_be_replayed_after_a_change` |
| Consentimento de `Transfer` não serve para `Swap`; o preço assinado é o preço pago | assets: `a_transfer_consent_cannot_be_used_as_a_swap`, `swap_is_atomic_delivery_versus_payment` |
| Gravame bloqueia a venda; só o órgão coloca e retira | assets: `liens_block_transfers_until_released` |
| **Troca atômica:** sem saldo, nada se move; com saldo, bem e moeda andam juntos e a oferta se conserva | assets: `swap_is_atomic_delivery_versus_payment` |
| O órgão só é trocado pelo quórum atual; os antigos deixam de valer | assets: `registrars_can_be_rotated_only_by_the_current_quorum` |
| Gênese e payload inválidos recusados | assets: `bad_genesis_classes_and_payloads_are_refused` |
| Todo bem tem exatamente um dono, o índice de donos é consistente e a oferta de moeda se conserva sob transferências e trocas aleatórias | assets (proptest): `ownership_and_supply_invariants` |

## Carteira e frase de recuperação (`teko-wallet` + dispositivo, ADR-0023)

| Invariante | Teste |
|---|---|
| BIP-39 e SLIP-10 `nist256p1` conferem com os **vetores oficiais**; entropia → palavras → entropia; checksum pega palavra trocada | wallet: `bip39_official_vectors`, `slip10_nist256p1_vector_1`, `phrase_validation`, proptests `entropy_to_words_to_entropy_roundtrip`, `replacing_one_word_is_caught_or_changes_the_wallet` |
| A passphrase muda a carteira e é validada (ASCII, ≤ 64) | wallet: `passphrase_changes_the_wallet_and_is_validated`; device: `passphrase_is_optional_confirmed_twice_and_changes_the_address` |
| O endereço bech32m detecta erro de digitação e é o mesmo do livro-razão | wallet (proptest): `wallet_addresses_detect_single_typos`; `addresses_are_distinct_per_index_and_match_the_ledger` |
| **A carteira é independente de identidade e voto** | wallet: `the_wallet_is_independent_of_the_identity_derivation`; device: `the_wallet_key_is_unrelated_to_the_identity_and_voter_secrets` |
| **As palavras só passam pela tela**: nenhuma resposta ou mensagem intermediária as contém | device: `creating_a_wallet_shows_24_words_only_on_the_screen`; proptest `no_response_exposes_secrets` agora inclui a chave mestra da carteira |
| Confirmação por 3 posições: palavra errada reinicia a exibição; cancelar ou errar demais não salva nada | device: `a_wrong_word_shows_the_phrase_again_and_nothing_is_saved_meanwhile`, `giving_up_leaves_the_device_without_a_wallet`, `too_many_failed_quizzes_abort_without_saving` |
| **Sem carteira pronta não há ID Cívico nem emissão** (a Suite não pula o backup) | device: `identity_steps_need_the_wallet_backup_first`; e2e: `identity_is_blocked_until_the_wallet_backup_is_confirmed` |
| Restaurar a mesma frase dá o **mesmo endereço**; frase com palavra desconhecida ou checksum errado é recusada; "não confere o endereço" não salva | device: `restore_gives_the_same_address_and_rejects_bad_phrases`; e2e: `a_wallet_restores_on_a_new_device_with_the_same_address` |
| "Verificar backup" confirma a frase certa e recusa outras sem mudar nada | device: `backup_check_confirms_the_right_phrase_and_refuses_others` |
| `GetAddress` só com o dispositivo desbloqueado; a carteira sobrevive ao reinício e o `Wipe` a destrói | device: `get_address_needs_the_unlocked_device_and_returns_public_data`, `the_wallet_survives_a_restart_and_a_wipe_destroys_it` |
| **O dispositivo mostra tudo que assina**: destino, valor, taxa, total e rede aparecem na tela e a assinatura é uma `Tx` válida do livro (qualquer mudança de campo a invalida) | device: `a_transfer_is_shown_on_the_device_and_signs_as_a_valid_ledger_tx` |
| Recusar ou expirar no dispositivo não gera assinatura; pedido inválido (zero, endereço ruim, para si mesmo, estouro, rede vazia) é recusado **antes de qualquer tela** | device: `rejecting_the_transfer_on_the_device_produces_no_signature`, `bad_transfer_requests_are_refused_before_any_prompt` |
| Enviar exige carteira e dispositivo desbloqueado | device: `transfers_need_the_wallet_and_the_unlocked_device` |
| **Ponta a ponta:** assinar no dispositivo, entregar ao livro local, saldos corretos, reinício reproduz, recusa no botão não muda nada | e2e: `sending_coins_is_signed_on_the_device_and_lands_in_the_devnet_economy` |
| A economia local move moedas com taxa, recusa repetição (nonce), saldo insuficiente e taxa baixa sem deixar rastro, e rejeita arquivo adulterado | coin: `devnet::tests::the_local_economy_moves_coins_with_fees_and_history`, `a_tampered_transactions_file_is_rejected_on_open` |
| **Consentimento de bem assinado no dispositivo vale no livro real:** o dono assina no dispositivo, comprador e órgão em software, e a troca move o bem e a moeda com a oferta conservada | device: `a_swap_signed_on_the_device_moves_the_asset_and_the_coin_in_the_real_ledger` |
| O dispositivo só assina se a carteira for parte; recusa antes de qualquer tela tipo desconhecido, mesma parte, venda sem preço, transferência com preço, endereço ruim; recusar no botão não gera consentimento; reconhece o papel de comprador | device: `consent_requests_are_validated_and_need_a_physical_confirmation` |
| Ponta a ponta: consentimento assinado, mostrado na tela e **verificado pelo cliente**; recusa no botão | e2e: `an_asset_consent_is_signed_on_the_device_and_verified_by_the_client` |
| **Bens no mundo de teste:** o órgão simulado registra (quórum 2-de-3), penhora e libera; a venda/compra com a contraparte move bem e moeda; gravame, bem alheio, preço zero e comprador sem saldo são recusados; tudo sobrevive ao reinício e a oferta se conserva | assets: `devnet::the_simulated_registrar_and_counterparty_trade_with_me`, `a_consent_from_the_wrong_person_does_not_move_the_asset` |
| **Ponta a ponta com o dispositivo:** vender e comprar um bem com o consentimento assinado no dispositivo (tela "VENDA DE BEM", papel vendedor/comprador), recusa no botão não move nada | e2e: `selling_and_buying_an_asset_with_my_device_consent_in_the_devnet_world` |
| **Stake assinado no dispositivo vira validador no livro real:** o dispositivo mostra a ação, a assinatura é uma `Tx` válida, travar cria o validador, sair exige a espera e resgatar antes do prazo é recusado | device: `a_stake_is_shown_on_the_device_and_becomes_a_validator_in_the_real_ledger` |
| Pedido de stake inválido (ação desconhecida, valor zero, resgate com valor, estouro, rede ruim) é recusado **antes de qualquer tela**; recusar no botão não assina | device: `bad_stake_requests_are_refused_before_any_prompt_and_rejection_signs_nothing` |
| **Ponta a ponta:** travar, sair, resgatar cedo (recusado), esperar blocos, resgatar; o mundo de teste unificado continua servindo saldo após tráfego de bens e stake; recusa no botão | e2e: `staking_is_signed_on_the_device_and_the_wallet_becomes_a_validator_in_the_devnet_world` |
| Saldo de teste da devnet (arquivo de alocações) | coin: `devnet::tests::allocations_roundtrip_and_balance`, `bad_input_is_refused` |

## Economia sobre BFT e fase de fundador (`teko-coin`, `teko-economy`, `teko-node`, ADR-0028)

| Invariante | Teste |
|---|---|
| A fase de fundador paga as recompensas ao **dono** (não à chave do nó), sem pré-mineração, e **encerra de vez** quando o stake total chega a N; depois o conjunto vem do stake; `Unstake` abaixo do mínimo é recusado | coin: `the_founder_phase_pays_rewards_to_the_owner_and_ends_when_enough_stake_exists` |
| Fundadores de gênese validados e nunca em rede travada; chave de consenso única e religável; evidência da **chave de consenso** pune o stake do **dono**; conjunto respeita mínimo, máximo e ordem | coin: `genesis_founders_are_validated…`, `a_consensus_key_can_be_linked_once_and_relinked`, `equivocation_by_a_consensus_key_slashes_the_stake_of_its_owner`, `the_set_after_the_founder_phase_respects…` |
| Execução de ensaio do bloco prevê a raiz real e não muda nada | ledger: `check_block_is_a_dry_run_that_predicts_the_real_root` |
| O proponente sorteado por hash é proporcional ao poder e **sem sequências longas**; o bloco só é aceito se o proponente declarado é o da rodada | bft: `the_proposer_rotates_in_proportion_to_the_power`; economy: `invalid_proposals_are_not_accepted_by_the_checker` |
| **Fundador sozinho produz blocos** com réplicas que acompanham; o conjunto muda quando o stake chega e um segundo validador entra; ambos decidem juntos e as raízes coincidem | economy: `the_founder_alone_produces_blocks…`, `the_phase_ends_when_the_stake_arrives_and_a_second_validator_joins_by_stake` |
| **Cliente que não confia em nós** reproduz e verifica a cadeia inteira; certificado ausente, bloco trocado, payload adulterado, proponente falso ou outra rede são recusados | economy: `a_client_replays_and_verifies_the_whole_chain_without_trusting_the_node` |
| Equivocação vira penalidade do dono; stake vira poder de voto limitado | economy: `equivocation_evidence_becomes_a_slash_of_the_owner`, `stake_becomes_bounded_voting_power` |
| **Rede real (3 nós em loopback):** só o fundador vota; as recompensas chegam ao dono; um cliente verifica; ao travar stake a fase encerra, os nós 1 e 2 entram por stake e os três decidem; um nó reiniciado reproduz e alcança | node: `the_founder_alone_produces_blocks_and_every_node_and_a_client_verify_the_same_chain`, `staking_ends_the_founder_phase_new_validators_join_and_a_restarted_node_catches_up` |

## O que **não** está coberto por teste automático (honestidade)

- **Economia BFT:** o fundador não ganha com o nó parado e o motor BFT não grava o *lock* em disco (reinício simultâneo de > 1/3 do poder é risco conhecido); a evidência de equívoco é contada mas **não** enviada como `Slash` pelo nó; sem teste de partição de rede entre processos nem de carga; a Suite (JS) com a cadeia do nó só foi exercitada com o mock.

- **Carteira:** a tela de digitação das palavras no terminal do emulador (`TerminalHal`) e a limpeza da tela só foram exercitadas à mão; a etapa e a tela Carteira da Suite (JS) só com o mock no navegador. Sem teste de tempo constante da comparação de palavras (o dispositivo compara com varredura completa, mas a frase não é um segredo de login). Sem auditoria da derivação.

- **Cifra/provas:** sem vetores de teste de terceiros nem auditoria (implementação própria). Sem teste de tempo constante; sem fuzzing dos decodificadores. Sem teste de carga com milhares de cédulas por bloco.
- **Cerimônia real:** só 3 processos em loopback, sem canal autenticado/cifrado nem tratamento de reclamações; sem teste com validador malicioso na rede (só no nível da biblioteca).
- **UI de resultados cifrados (JS):** só exercitada com o mock no navegador.
- **Fluxo do pedido e da instalação do pacote na Suite (JS):** só exercitado com o mock no navegador; o lado do dispositivo e da chain é coberto pelos testes do ADR-0038.
- **Rede em várias máquinas / testnet real, partições de rede e latência:** só loopback, 3 nós em threads. Sem teste de carga nem fuzzing do protocolo de rede.
- **Painel de rede na Suite (JS):** só exercitado com o mock no navegador.
- **Explorador público (Rust + JS):** a conferência de inclusão e o explorador só foram exercitados com o mock no navegador; a prova Merkle em si é testada em `teko-bulletin`. A "conferência" na Suite recalcula contra a raiz do MESMO arquivo: confirma consistência interna, não protege contra um operador que mostre logs diferentes (V19).
- **Anonimato real do registro:** a propriedade de cegamento vem da matemática do RFC (vetor oficial) e o teste confere que o livro e o pedido não contêm a chave; **não há teste estatístico nem de canal/tempo**. No protótipo, autoridade e quadro rodam no mesmo processo.
- **Implementação RFC 9474 própria sem auditoria independente.**

- **Tempo constante da comparação do PIN:** o código usa `subtle::ConstantTimeEq`, mas não há teste de temporização.
- **`--listen` só em loopback:** verificado manualmente (recusa `0.0.0.0`); falta teste automatizado.
- **A tela de Resultados ao vivo (JS):** só exercitada com o mock no navegador (reconciliação de DOM, animação, abas). A apuração que a alimenta é testada; o desenho, não.
- **A janela da Suite (Tauri) e a UI JS:** não há teste automatizado de interface (a tela de votação foi exercitada só com o mock no navegador; ali um laço de re-render travou a página e foi corrigido); a UI foi exercitada com o mock no navegador e o app real só foi aberto até conectar no emulador.
- **`no_std` em alvo embarcado real:** o CI compila o núcleo para `thumbv7em-none-eabihf` (passou em 2026-11-14); não há teste de execução em hardware.
- **Quadro de avisos de operador único:** sem testemunhas independentes, um operador pode mostrar logs diferentes a pessoas diferentes (*split view*). Só um head fixado e comparado por terceiros detecta isso; não há essa comparação implementada.
- **Fuzzing contínuo do parser:** só `proptest`; sem `cargo-fuzz`.
| A gênese dá as primeiras moedas à carteira do fundador (`--alloc`): altura 0 já mostra o saldo, a oferta fecha, e o fundador consegue travar stake sem nenhum bloco antes | node: `the_genesis_gives_the_founder_the_first_coins_so_the_network_can_start_with_no_block_at_all` |

## Caução das instituições e penalidade na rede de votos (ADR-0035)

| Garantia | Teste |
|---|---|
| Equivocação na rede de votos queima 10% da caução, uma vez por ofensa; nada é devolvido a ninguém e o teto não muda | coin: `equivocation_on_the_vote_chain_burns_ten_percent_of_the_bond_once` |
| Evidência de outra rede (inclusive a da própria economia) é recusada, a `Slash` da economia não aceita evidência de votos, e votos iguais ou lixo são recusados | coin: `vote_chain_evidence_must_be_for_the_vote_chain_and_the_two_penalties_do_not_mix` |
| A penalidade recai sobre o dono ao qual a chave da instituição está ligada | coin: `an_institution_is_punished_through_the_owner_its_key_is_linked_to` |
| Sem `vote_chain_id` configurado a penalidade fica desligada; o payload faz ida e volta | coin: `vote_chain_penalties_are_off_...`, `the_vote_slash_payload_round_trips` |
| Ponta a ponta: uma instituição que assina dois prevotes em conflito é detectada ao vivo pelo nó, a evidência vai à economia e 10% da caução é queimada | node: `an_institution_that_equivocates_on_the_vote_chain_loses_ten_percent_of_its_bond_in_the_economy` |

## Bens: negociação direta com janela de veto (ADR-0036)

| Garantia | Teste |
|---|---|
| O horário do bloco chega ao livro e réplicas concordam; o dry run prevê a raiz | ledger: `modules_see_the_block_time_and_replicas_agree_on_it`, `the_selection_of_a_proposer_sees_the_same_time...` |
| Oferta, aceite (preço em escrow) e finalização só depois da janela, por qualquer um; não finaliza duas vezes | assets: `offer_accept_and_finalize_move_the_asset_and_the_money_only_after_the_window` |
| Só o dono oferece; só o comprador nomeado aceita; oferta presa à versão e ao hash dos metadados; uma oferta por bem | assets: `only_the_owner_offers_only_the_named_buyer_accepts_and_stale_offers_are_refused` |
| O aceite leva os termos vistos: cancelar e reofertar não o sequestra | assets: `an_acceptance_binds_the_terms_the_buyer_saw...` |
| Aceitar sem dinheiro falha por inteiro; o dono cancela só antes do aceite | assets: `accepting_without_the_money...`, `the_seller_can_cancel_only_before_the_buyer_accepts` |
| O quórum veta dentro da janela (preço devolvido, bem fica); veto tardio, sem quórum ou para outro motivo é recusado | assets: `the_organ_quorum_vetoes...`, `a_veto_after_the_window_or_for_another_offer_is_refused` |
| Gravame (ou sua liberação) cancela a negociação e devolve o preço; sem gravame o bem volta a poder ser oferecido | assets: `any_change_to_the_asset_cancels...`, `after_the_organ_releases_the_lien...` |
| Moeda e bens se conservam, escrow incluído, em qualquer sequência de ações | assets: `money_and_ownership_are_conserved_under_any_sequence` (proptest) |
| Os bytes que o dispositivo monta são exatamente os que o módulo decodifica | assets: `the_payloads_the_device_builds_are_byte_for_byte...` |
| O dispositivo mostra, pede o botão e assina a transação do dono/comprador; recusa parte errada, janela zero, valores absurdos e metadados que não são do bem | device-core: `an_offer_and_an_acceptance_signed_on_the_device_run_in_the_real_ledger...`, `trade_requests_are_validated...`, `a_vehicle_offer_shows_plate_and_model...` |
| Com o dispositivo real do emulador: oferta e aceite saem da carteira do usuário, a janela é cruzada e o bem muda de dono | client: `selling_and_buying_an_asset_with_my_device_signing_my_own_transactions...` |
| Se eu rejeito o aceite, a oferta da contraparte é retirada (nada fica bloqueado) | assets/devnet: `if_i_reject_the_acceptance_the_counterpartys_offer_is_withdrawn...` |
| Em nós reais: negociação pendente na janela, finalização antes dela recusada na hora (com o motivo) sem bloco vazio, e concluída depois | node: `a_trade_between_wallets_waits_in_the_veto_window_in_real_nodes_and_completes_after_it` |
| O veto simulado do órgão devolve o preço ao comprador em nós reais | devnet-sim: `the_simulated_organ_vetoes_a_pending_trade...` |

## Credenciais na chain (ADR-0037)

| Garantia | Teste |
|---|---|
| Proposta → pendente → ativa → expirada só pelo horário do bloco | credentials: `it_goes_from_proposed_to_pending_to_active_to_expired_by_block_time_only` |
| Só membro propõe/aprova; cada membro conta uma vez | credentials: `only_members_propose_and_approve_and_each_member_counts_once` |
| Veto só dentro da janela e só com o quórum; motivo público obrigatório | credentials: `the_quorum_vetoes_inside_the_window_and_never_after` |
| Revogação só de credencial ativa, por quórum, com motivo | credentials: `an_active_credential_is_revoked_by_the_quorum_with_a_public_reason` |
| Validade por tipo (vitalícia, teto, janela) | credentials: `validity_rules_follow_the_type` |
| Privado guarda só compromisso; público amarra os atributos ao hash | credentials: `private_types_keep_only_a_commitment_and_public_types_bind_their_claims` |
| Nunca ativa na janela nem depois do veto, em qualquer ordem de ações | credentials: `a_credential_is_never_active_inside_the_window_or_after_veto` (proptest) |
| Tipos de credencial na gênese: texto ida e volta e módulo montado | economy: `credential_types_in_the_genesis_survive_the_text_round_trip_and_build_the_module` |
| Chave de credencial por tipo, estável, sem ligação com identidade e carteira | device-core: `the_credential_key_is_per_type_stable_and_unlinkable_to_identity_wallet_and_voter_keys` |
| O dispositivo emite, aprova, veta e revoga e a transação roda no livro real | device-core: `an_organ_member_issues_approves_vetoes_and_revokes_from_the_device_in_the_real_ledger` |
| Pedidos inválidos recusados antes de mostrar; sem botão não assina | device-core: `credential_requests_are_validated_before_anything_is_shown_or_signed` |
| Com o dispositivo real do emulador (TCP) | client: `credential_keys_and_organ_transactions_are_signed_on_the_real_device_and_run_in_the_ledger` |
| Em nós reais: proposta do emissor + cosign dos membros simulados | devnet-sim: `simulated_organ_members_cosign_a_credential_proposed_by_the_issuer_wallet_on_real_nodes` |

## ID cívico na chain (ADR-0038)

| Garantia | Teste |
|---|---|
| O pacote com M membros distintos do órgão do `trust.txt` é aceito e dá o id da credencial | crypto: `a_bundle_with_the_quorum_verifies_and_gives_the_chain_id` |
| Quórum curto, o mesmo membro duas vezes ou membros que o dispositivo não conhece: recusados | crypto: `a_short_repeated_or_foreign_quorum_is_refused` |
| Qualquer mudança nos atributos, no sal, no titular, na assinatura, no payload, na rede, na validade ou no tipo é recusada | crypto: `any_change_to_the_claims_the_salt_the_holder_or_a_transaction_is_refused` |
| Um dispositivo sem órgão no `trust.txt` não instala nada | crypto: `a_device_with_no_trusted_organ_accepts_nothing`; device-core: `a_device_that_trusts_no_civic_organ_installs_nothing` |
| Atributos lidos com rigor; o compromisso não depende da ordem e muda com qualquer atributo ou sal | crypto: `claims_are_parsed_strictly_and_commit_independently_of_their_order` |
| A prova revela um atributo; o compromisso é refeito com os resumos dos demais e comparado com a chain; inativa, outro nonce, outro verificador ou valor trocado: recusado | crypto: `a_presentation_discloses_one_claim_and_the_chain_commitment_still_checks` |
| A política do órgão civil é tudo-ou-nada e os membros são endereços de 32 bytes | attestation: `the_civic_organ_is_all_or_nothing_and_members_are_addresses` |
| O dispositivo recusa pacote de outra chave, com quórum curto, adulterado ou malformado, sem gravar nada | device-core: `credential_for_another_key_or_forged_issuer_is_refused` |
| Prova de idade revela só `age_over_18` e falha se a chain diz que não está ativa | device-core: `attribute_proof_discloses_only_the_predicate` |
| Atributos do órgão: datas, idades derivadas, domicílio e recusas | credentials: `civic::tests` |
| Com o dispositivo real do emulador: pedido, emissão, prova contra a chain, registro anônimo e reemissão com o mesmo cidadão | client: `full_onboarding_and_daily_use`, `election_flow_president_for_all_governor_and_mayor_by_domicile`, `losing_the_device_means_reissue_the_old_credential_is_revoked_on_the_chain_and_the_citizen_stays_one` |
| **Em nós reais:** o dispositivo propõe como membro, o membro simulado aprova, o pacote sai da chain, o dispositivo o instala e a prova só vale após a janela | devnet-sim: `the_civic_credential_is_issued_on_real_nodes_by_the_organ_quorum_and_installed_on_the_device` |

## Recuperar tudo pela carteira (ADR-0039)

| Garantia | Teste |
|---|---|
| Chaves de credencial por tipo, determinísticas, distintas dos endereços e da passphrase; restaurar dá as mesmas | wallet: `credential_keys_are_per_type_deterministic_and_unrelated_to_the_wallet_addresses` |
| Só o destinatário abre; contexto autenticado; qualquer alteração é recusada; cada selo é novo; limites | crypto: `only_the_recipient_opens_it_with_the_same_context`, `any_change_to_the_sealed_bytes_is_refused`, `each_seal_is_fresh_and_oversized_or_invalid_input_is_refused`, `open_inverts_seal` (proptest) |
| Restaurar a carteira num dispositivo novo dá as mesmas chaves de credencial (lote com uma confirmação); `civic` e listas inválidas recusados | device-core: `restoring_the_wallet_on_a_new_device_gives_back_the_same_credential_keys` |
| Chaves de credencial exigem carteira | device-core: `credential_keys_need_the_wallet` |
| O dispositivo sela ao titular, a chain guarda só o cifrado, o dispositivo abre; outra chave, tipo ou blob adulterado não abre | device-core: `an_organ_member_issues_approves_vetoes_and_revokes_from_the_device_in_the_real_ledger` |
| Bem registrado para alguém aparece no histórico dele, não só no de quem assinou | explorer: `an_asset_registered_to_someone_shows_in_their_history_not_only_the_signers` |
| **Em nós reais:** emitir, "perder" o dispositivo, restaurar pelas 24 palavras num novo, reencontrar a credencial pela chave derivada e abrir os atributos; os outros tipos não têm nada | devnet-sim: `restoring_the_wallet_on_a_new_device_recovers_the_credentials_from_the_chain_and_opens_them` |

## Candidatos, resultado certificado, 2º turno e eleito (ADR-0040)

| Garantia | Teste |
|---|---|
| Claims da candidatura canônicos; jurisdição conforme o cargo | credentials: `candidate::tests::claims_round_trip...`, `the_jurisdiction_follows_the_office` |
| Entram só candidaturas ativas, do cargo, jurisdição e eleição, que duram além da votação; ordenadas por número | credentials: `a_contest_takes_the_active_matching_candidates...`, `inactive_expiring_or_foreign_candidacies_do_not_enter` |
| Número ou titular repetido recusa o pleito | credentials: `a_repeated_number_or_holder_refuses_the_contest` |
| Manifesto v3: opção com credencial (32 bytes, coberta pelo hash), `runoff` só em vaga única, 2º turno com 2 opções | election: `candidates_reference_their_credentials_and_rounds_follow_the_office` |
| Maioria absoluta elege; sem ela, 2º turno entre os dois mais votados; empates que a regra não resolve = sem eleito; vaga única só | election: `outcome::tests::*` (incl. proptest: nunca dois vencedores) |
| Mais de 2/3 das instituições têm de assinar os mesmos bytes | election: `more_than_two_thirds_of_the_institutions_must_sign_the_same_bytes`; credentials: `the_winner_gets_a_lifetime_credential...`, `three_institutions_need_all_three` |
| Assinatura sobre outros bytes, estranhos, outra chain, resultado não decidido, candidatura de outro pleito, revogada ou cargo de várias vagas: recusados | credentials: `a_signature_over_other_bytes...`, `only_a_decided_contest...`, `a_revoked_or_vetoed_candidacy...` |
| O mesmo resultado não é reivindicado duas vezes | credentials: `the_winner_gets_a_lifetime_credential...` |
| O eleito seguinte do mesmo cargo sucede o anterior; outro estado não interfere; reeleição cria registro novo | credentials: `the_next_elected_of_the_same_office_supersedes...`, `a_reelected_person_gets_a_new_credential...` |
| O tipo `eleito` não tem órgão: ninguém o propõe | credentials: `the_elected_type_has_no_organ_so_nobody_can_propose_it` |
| Gênese com as instituições da chain de votos e o tipo de sistema ida e volta | economy: `the_vote_institutions_and_the_system_type_survive_the_genesis_text_round_trip` |
| Eleição com candidatos da chain (credencial ligada a cada opção) e 2º turno como revisão assinada que replica | issuer: `candidates_from_the_chain_replace_the_example_list...`, `a_second_round_is_a_signed_revision...` |
| **Em nós reais:** candidaturas emitidas, eleição montada da chain, votos cifrados, fechamento, resultados certificados pelas instituições, `ClaimElected`, credenciais `eleito` ativas e vitalícias, (2º turno se houver) | devnet-sim: `candidates_on_the_chain_become_an_election_whose_certified_results_elect_people_with_lifetime_credentials` |
| Cédula auditada abre na opção pedida; aleatório errado, cédula adulterada ou de outro contexto não abre | ballot-crypto: `an_audited_ballot_opens_to_exactly_the_option_it_encrypts`, `a_wrong_randomness_a_tampered_ballot_or_another_context_does_not_open` |
| Auditar não consome o voto; o compromisso vem antes da escolha; o aleatório não é revelado nem guardado numa cédula votada; recusar tudo cancela | device-core: `the_voter_can_audit_a_ballot_which_is_spoiled_while_the_real_vote_is_still_available`, `what_is_cast_is_exactly_what_was_committed_and_its_randomness_is_never_revealed_or_stored`, `refusing_both_the_cast_and_the_audit_cancels_and_records_nothing` |
| Um dispositivo que cifra outra opção é pego pelo cliente; compromisso incoerente é recusado | client: `a_device_that_encrypted_another_option_is_caught`, `a_commitment_that_does_not_match_its_ballot_is_refused`; e2e `the_voter_audits_a_test_ballot_on_the_device_and_then_casts_the_real_one` |
| O dispositivo confere sozinho certificado > 2/3, bloco, rede e inclusão; recusa certificado fraco, de outra rede, bloco adulterado, folha alheia, prova errada, bloqueado, sem botão ou sem rede de votos confiável | device-core: `the_device_checks_alone_that_its_ballot_is_in_a_certified_block`, `the_device_refuses_what_it_cannot_prove_by_itself`, `the_check_needs_the_button...`, `a_device_that_trusts_no_vote_network_checks_nothing`; client e2e `the_device_checks_alone_that_its_ballot_is_final_on_the_vote_chain` |
| Toda entrada tem bloco certificado e prova verificável pelo dispositivo | chain: `every_entry_has_a_certified_block_and_a_proof_that_a_device_can_check_alone` |
| Veredito por cédula (contada, duplicada, assinatura, fora de fase) | election: `a_watched_ballot_gets_its_own_verdict...`, `a_ballot_outside_the_voting_window...` |
| `trust.txt` com as instituições da chain de votos, tudo ou nada | attestation: `the_vote_chain_institutions_are_all_or_nothing` |
| O simulador de blocos produz exatamente a quantidade pedida nas duas chains e recusa 0 e mais de 1000 | devnet-sim: `the_block_simulator_makes_the_asked_number_of_blocks_on_both_chains` |
| As entradas de teste do simulador (tipo 200) nunca mudam uma contagem | election-admin: `the_block_simulators_filler_entries_never_change_a_count` |
| Todo alvo do Makefile está no menu do `teko-dev` (ou deliberadamente fora); linha de comando só com variáveis preenchidas; valores ruins recusados; menu por estado da devnet; cabeçalho | teko-dev: `every_makefile_target_is_in_the_menu...`, `a_command_line_has_the_target...`, `bad_values_are_refused...`, `the_menu_follows_the_state_of_the_network`, `the_header_says...`, `probing_a_folder_without_a_network...` |
| Branco e nulo: nome fixo, sem número nem credencial, no máximo um de cada, ≥ 2 candidatos, 2º turno com 2 candidatos + especiais, teto de 16 no cifrado; o encode sem `kind` não muda | election: `blank_and_null_are_options_with_fixed_names...`, `a_second_round_has_two_candidates_and_may_keep_the_specials`, `an_encrypted_contest_counts_the_specials_in_its_sixteen_options` |
| A maioria conta só candidatos: com 60 brancos e 40 válidos, 21 elege e 20 vai a 2º turno; só especiais = sem eleito; branco nunca finalista nem vencedor | election/outcome: `blank_and_null_are_outside_the_majority_and_never_win_or_lead`, `a_contest_where_nobody_votes_for_a_candidate_has_no_winner` |
| O ranking ignora os especiais e eles são contados à parte | election/tally: `blank_and_null_are_counted_reported_apart_and_never_ranked` |
| O dispositivo mostra VOTO EM BRANCO/NULO com as próprias palavras, informa `option_kinds` e a auditoria abre a cédula em branco | device-core: `blank_and_null_show_the_devices_own_words_and_can_be_audited` |
| Em nós reais: brancos e nulos plantados saem decifrados no resultado certificado (`blank_votes`, `null_votes`) e só candidatos são eleitos | devnet-sim: `candidates_on_the_chain_become_an_election_whose_certified_results_elect_people_with_lifetime_credentials` |
| A parte do sorteio vale só para o pleito e o empate; quaisquer `t` partes dão o mesmo ponto e a mesma semente; parte adulterada, chave errada ou outro contexto é recusado | ballot-crypto: `the_point_is_valid_deterministic_and_domain_separated`, `any_t_shares_combine_to_the_same_point_and_the_same_seed`, `a_wrong_share_a_wrong_key_or_another_context_is_refused` |
| A entrada do sorteio liga o pleito e o empate exato; a ordem é uma permutação decidida só pela semente | election/draw: `the_input_is_bound_to_the_contest_and_to_the_exact_tie`, `the_order_is_a_permutation_decided_only_by_the_seed` |
| Empate no topo e vaga de finalista são sorteados só entre os empatados (branco/nulo fora); o que a regra decide não usa semente; `valid == 0` continua sem eleito | election/outcome: `a_tie_at_the_top_is_decided_by_the_draw_and_only_among_the_tied`, `a_tie_for_the_finalist_slot_is_drawn_and_what_the_rule_decides_is_untouched`, `with_a_seed_every_decidable_contest_is_decided_among_the_right_candidates` |
| O apurador só aceita partes de pleito encerrado, revelado e empatado, para ESTE empate, com prova válida, uma por validador; a semente sai com o limiar e é igual com qualquer subconjunto | election/tally: `a_tie_is_drawn_from_the_validators_shares_and_any_threshold_of_them_gives_the_same_seed` |
| Um resultado sorteado elege como qualquer outro na economia; empate sem semente segue recusado | credentials: `a_drawn_winner_elects_like_any_other_the_ledger_trusts_the_institutions_signatures` |
| **Em nós reais, com uma instituição fora:** empate de 2, vaga de finalista e empate triplo são sorteados, certificados, refeitos do quadro, o eleito é registrado e o 2º turno abre com os finalistas sorteados | devnet-sim: `ties_are_broken_by_a_draw_nobody_controls_that_anyone_can_redo_and_it_works_with_an_institution_down` |
| Credencial no formato antigo: o dispositivo diz o que fazer | device-core: `a_device_with_a_credential_in_the_old_format_says_so_instead_of_failing_mysteriously` |
| Simular sem pleito aberto falha com a instrução | election-admin: `simulating_with_no_open_contest_fails_and_says_what_to_do_instead_of_reporting_zero` |
| Cédula de 2 vagas: 1 ou 2 candidatos distintos, ou só branco/nulo; recusa 3 candidatos, repetido, candidato+branco, branco+nulo, nada; prova movida de contexto/vagas/adulterada; uma vaga mantém os bytes de hoje | ballot-crypto: `a_one_seat_ballot_keeps_todays_bytes_and_the_multi_api_agrees_with_it`, `a_two_seat_ballot_is_one_or_two_distinct_candidates_or_exactly_blank_or_null`, `a_forged_two_seat_ballot_is_refused_by_the_proofs` |
| Manifesto do Senado: 1 ou 2 vagas explícitas, só Senador, candidatos > vagas, sem 2º turno, 2 vagas exigem eleição cifrada | election/manifest: `a_senate_contest_has_one_or_two_seats_needs_more_candidates_than_seats_and_two_seats_need_encryption` |
| Os N mais votados vencem (branco/nulo fora), corte empatado sorteado, menos de N com voto deixa vagas, totais cabem nas cédulas, proporcional segue pendente | election/outcome: `a_senate_contest_elects_the_n_most_voted_candidates...`, `a_tie_at_the_cut_of_the_seats_is_drawn_and_zero_votes_never_take_a_seat`, `the_totals_of_a_multi_seat_contest_must_fit_the_ballots_that_were_counted`, `a_senate_contest_never_elects_more_than_its_seats_nor_a_candidate_without_votes` |
| O dispositivo recusa escolhas fora da regra, mostra "SEUS VOTOS (k de N vagas)", audita o conjunto | device-core: `a_two_seat_vote_is_up_to_two_distinct_candidates_or_blank_or_null_alone_and_the_screen_lists_them`, `blank_alone_is_a_valid_two_seat_ballot` |
| A auditoria exige o conjunto exato pedido | client: `a_two_seat_audit_must_open_to_exactly_the_set_that_was_asked_for` |
| Um resultado do Senado elege todos os titulares numa reivindicação (tudo ou nada); cada classe (1 ou 2 vagas) só substitui a si mesma; resultados malformados recusados | credentials: `a_senate_result_elects_all_its_winners_in_one_claim_and_each_class_replaces_only_itself`, `malformed_senate_results_are_refused` |
| **Em nós reais:** Senador de 2 vagas com corte empatado sorteado e de 1 vaga sem empate, certificados, refeitos do quadro, 2+1 credenciais `eleito` | devnet-sim: `ties_are_broken_by_a_draw_nobody_controls...` (com `senador-sp` e `senador-rj`) e `candidates_on_the_chain_become_an_election...` |
| A troca precisa de mais de 2/3 do conjunto em vigor, é presa à época, à cadeia e ao conjunto que substitui, e todo campo é assinado; conjunto de 2 a 64 chaves compactas distintas e a ordem importa | bft: `a_change_needs_more_than_two_thirds_of_the_set_in_force`, `a_change_is_bound_to_its_epoch_chain_set_and_every_field`, `the_new_set_must_be_two_to_sixty_four_distinct_valid_keys_and_the_order_matters` |
| A troca vale a partir da altura, blocos antigos seguem válidos e uma réplica refaz as épocas; todas as regras quebradas invalidam o bloco | chain: `an_approved_change_takes_effect_at_its_height_old_blocks_stay_valid_and_a_replica_rebuilds_the_epochs`, `a_set_change_that_breaks_a_rule_makes_the_block_invalid` |
| A rotação só é segura com todos os pleitos encerrados, decifrados e sem sorteio pendente | election/tally: `the_institutions_may_rotate_only_when_every_contest_is_over` e as asserções de `a_tie_is_drawn_from_the_validators_shares...` |
| A economia troca o conjunto guardado e `ClaimElected` passa a exigir o novo | credentials: `a_rotation_approved_by_more_than_two_thirds_changes_who_can_certify_a_result` |
| O dispositivo segue as trocas elo por elo, recusa cada elo quebrado, não volta de época nem aceita outro ramo, persiste a catraca e o `wipe` a zera | device-core: `the_device_follows_the_chain_of_institution_changes_and_refuses_every_broken_link`, `the_ratchet_never_goes_back_and_never_accepts_another_branch_of_changes` |
| **Em nós reais:** uma instituição é trocada com a chain rodando; réplica segue as épocas; cerimônia de chave nova com o conjunto novo; eleição inteira decidida e certificada por ele; a economia a aceita; a proposta de nova troca é recusada com eleição aberta | devnet-sim: `an_institution_is_replaced_while_the_chain_runs_and_the_next_election_is_decided_by_the_new_set` |
