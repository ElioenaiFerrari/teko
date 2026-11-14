/* Somente desenvolvimento: simula o backend para pré-visualizar a UI num navegador comum.
   Só ativa com `?mock=none|virgin|locked|ready` e sem o runtime do Tauri. */
(() => {
  const q = new URLSearchParams(location.search).get('mock');
  if (window.__TAURI__ || !q) return;

  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const listeners = {};
  const emit = (ev, payload) => (listeners[ev] || []).forEach((cb) => cb({ payload }));
  const ID = 'teko1qpzry9x8gf2tvdw0s3jn54khce6mua7lq9xk4m';
  const FP = 'a1b2c3d4e5f60718';
  const dev = {
    connected: q !== 'none',
    state: q === 'virgin' ? 'virgin' : q === 'wallet' ? 'initialized' : q === 'ready' || q === 'locked' ? 'credentials_issued' : 'virgin',
    locked: q === 'locked',
    fw: q === 'virgin' || q === 'none' ? 0 : 1,
    election: false,
    wallet: q === 'ready' || q === 'locked',
    retries: Number(new URLSearchParams(location.search).get('retries') || 8),
  };
  const WADDR = 'tekow1qpzry9x8gf2tvdw0s3jn54khce6mua7lqpzry9x8gf2tvdw0s3jn54khce6muaw9z3k';
  const status = () => ({
    connected: dev.connected, busy: false, addr: '127.0.0.1:7878', trust_ok: true, error: null,
    status: dev.connected ? { fw_version: dev.fw, state: dev.state, locked: dev.locked, retries_remaining: dev.retries, label: 'Teko Emulator', has_credential: dev.state === 'credentials_issued', wallet_ready: dev.wallet, capabilities: [] } : null,
  });
  const prompt = async (kind, ms = 900) => { emit('device-prompt', kind); await wait(ms); };
  const offer = (v) => ({ version: v, size: 49152, fingerprint: v === 1 ? 'a1b2 c3d4 e5f6 0718' : '9f8e 7d6c 5b4a 3921', valid_signers: 2, threshold: 2, total_signers: 3, installed_version: dev.fw, is_downgrade: v < dev.fw, verdict: v < dev.fw ? 'Versão anterior à instalada: recusado (anti-downgrade)' : v === dev.fw ? 'Já instalado' : dev.fw ? 'Atualização disponível' : 'Pronto para instalar', installable: v > dev.fw });

  const NOW = Math.floor(Date.now() / 1000), DAY = 86400;
  const voted = {};
  const contests = () => [
    { contest_id: 'presidente', title: 'Presidente da República', phase: 'voting_open', phase_code: 3, scope: 'national', scope_code: '', option_names: ['Candidata Onça-Pintada', 'Candidato Boto-Cor-de-Rosa', 'Candidata Arara-Azul'], eligible: true, registration_required: true, voting_opens: NOW - DAY, voting_closes: NOW + 3 * DAY },
    { contest_id: 'governador-sp', title: 'Governador de São Paulo', phase: 'registration_open', phase_code: 2, scope: 'state', scope_code: 'SP', option_names: ['Candidato Guará', 'Candidata Vitória-Régia'], eligible: true, registration_required: true, voting_opens: NOW + DAY, voting_closes: NOW + 5 * DAY },
    { contest_id: 'governador-rj', title: 'Governador do Rio de Janeiro', phase: 'voting_open', phase_code: 3, scope: 'state', scope_code: 'RJ', option_names: ['Candidato Mico-Leão-Dourado', 'Candidata Capivara'], eligible: false, registration_required: true, voting_opens: NOW - DAY, voting_closes: NOW + 2 * DAY },
    { contest_id: 'prefeito-3550308', title: 'Prefeito de São Paulo', phase: 'candidates_published', phase_code: 1, scope: 'municipality', scope_code: '3550308', option_names: ['Candidata Jaguatirica', 'Candidato Tucano'], eligible: true, registration_required: true, voting_opens: NOW + 3 * DAY, voting_closes: NOW + 4 * DAY },
  ].map((c) => ({ ...c, voted: !!voted[c.contest_id] }));
  const electionSummary = () => ({ election_id: 'eleicoes-exemplo-2026', title: 'Eleições de exemplo 2026 (protótipo)', revision: 1, seq: 6, contests: contests() });

  let published = q === 'ready' && false;
  let headExtra = 0;
  const mk = (id, title, office, scope, code, phase, names, first, base) => ({ id, title, office, scope, code, phase, names, first, votes: names.map((_, i) => Math.round(base / (i + 1) ** 1.2)) });
  const live = Object.fromEntries([
    mk('presidente', 'Presidente da República', 'president', 'national', '', 'voting_open', ['Candidata Onça-Pintada', 'Candidato Boto-Cor-de-Rosa', 'Candidata Arara-Azul'], 10, 4200),
    mk('governador-sp', 'Governador de São Paulo', 'governor', 'state', 'SP', 'voting_open', ['Candidato Guará', 'Candidata Vitória-Régia', 'Candidato Lobo-Guará', 'Candidata Arara-Canindé', 'Candidato Tamanduá-Bandeira', 'Candidata Capivara', 'Candidato Jacaré-do-Papo-Amarelo', 'Candidata Onça-Parda', 'Candidato Mico-Leão-Dourado', 'Candidata Ararinha-Azul', 'Candidato Tatu-Canastra', 'Candidata Sabiá-Laranjeira'], 20, 900),
    mk('governador-rj', 'Governador do Rio de Janeiro', 'governor', 'state', 'RJ', 'voting_open', ['Candidato Mico-Leão-Dourado', 'Candidata Capivara', 'Candidato Boto-Tucuxi'], 40, 380),
    mk('governador-mg', 'Governador de Minas Gerais', 'governor', 'state', 'MG', 'voting_open', ['Candidato Lobo-Guará', 'Candidata Onça-Parda', 'Candidato Tamanduá-Bandeira', 'Candidata Seriema'], 50, 420),
    mk('governador-ba', 'Governador da Bahia', 'governor', 'state', 'BA', 'voting_open', ['Candidata Arara-Vermelha', 'Candidato Tartaruga-Marinha', 'Candidata Baleia-Jubarte'], 60, 300),
    mk('governador-rs', 'Governador do Rio Grande do Sul', 'governor', 'state', 'RS', 'voting_open', ['Candidato Quero-Quero', 'Candidata Graxaim', 'Candidato Zorrilho', 'Candidata Ema'], 70, 240),
    mk('prefeito-3550308', 'Prefeito de São Paulo', 'mayor', 'municipality', '3550308', 'candidates_published', ['Candidata Jaguatirica', 'Candidato Tucano'], 30, 0),
  ].map((c) => [c.id, c]));
  const tick = () => { for (const c of Object.values(live)) { if (c.phase !== 'voting_open') continue; const n = Math.ceil(Math.random() * 4); for (let k = 0; k < n; k++) { const r = Math.random() ** 2.2; c.votes[Math.min(c.names.length - 1, Math.floor(r * c.names.length))] += 1; headExtra += 1; } } };

  const H = {
    device_status: async () => status(),
    dev_info: async () => ({ dev_dir: 'dev', addr: '127.0.0.1:7878', authority_root: '02ab' }),
    verify_authenticity: async () => { await wait(600); return { genuine: true, batch_id: 'lote-dev-001', fw_version: dev.fw, reason: null }; },
    firmware_catalog: async () => [offer(1), offer(2)],
    install_firmware: async ({ version }) => { await prompt('firmware_install'); for (let i = 1; i <= 5; i++) { emit('fw-progress', [i * 10, 50]); await wait(150); } dev.fw = version; if (dev.state === 'virgin') dev.state = 'firmware_installed'; return version; },
    setup_pin: async () => { await prompt('pin_entry', 1200); dev.state = 'initialized'; },
    wallet_setup: async () => { await prompt('wallet_create', 900); for (let i = 0; i < 4; i++) await prompt('seed_words', 700); await prompt('wallet_confirm', 1500); dev.wallet = true; return { address: WADDR, has_passphrase: false }; },
    wallet_restore: async () => { await prompt('wallet_restore', 1800); dev.wallet = true; return { address: WADDR, has_passphrase: false }; },
    wallet_check_backup: async () => { await prompt('wallet_check', 1800); },
    wallet_address: async () => ({ address: WADDR, public_key: '02' + 'cd'.repeat(32) }),
    wallet_balance: async () => ({ available: true, amount: 4000, note: 'Alocação de gênese de TESTE da devnet (mock).' }),
    unlock: async () => { await prompt('pin_entry', 1000); dev.locked = false; return { result: 'unlocked' }; },
    civic_id: async () => { if (dev.state === 'initialized') await prompt('confirm_civic_id'); return { id: ID, public_key: '02' + 'ab'.repeat(32) }; },
    dev_mode: async () => true,
    revoke_credential: async () => 1,
    issue_credential: async () => { await prompt('issue_identity'); await prompt('load_credential'); dev.state = 'credentials_issued'; },
    reveal_identity: async () => { for (let i = 0; i < 4; i++) await prompt('prove_attribute', 500); return [['name', 'Maria da Silva'], ['birth_date', '1990-05-17'], ['nationality', 'BR'], ['registry_id', 'TK-9A41C07E']]; },
    pairwise_id: async ({ service }) => { await prompt('pairwise_id'); return 'tekop1' + btoa(service).replace(/[^a-z0-9]/gi, '').toLowerCase().slice(0, 32); },
    hash_file: async () => '3a1f9c0d77be42e18a5c6d09f4b2e7138c5d6a90b1e2f3748596a7b8c9d0e1f2',
    sign_document: async ({ summary }) => { await prompt('sign_document', 1400); return { format: 'teko-signed-document/1', hash_sha3_256: '3a1f9c0d77be42e18a5c6d09f4b2e7138c5d6a90b1e2f3748596a7b8c9d0e1f2', summary, civic_id: ID, public_key: '02' + 'ab'.repeat(32), signature: 'cd'.repeat(64), scheme: 'ECDSA-P256' }; },
    save_signed_document: async () => '/Users/voce/Downloads/contrato.teko-signature.json',
    verify_document: async () => ({ signature_valid: true, civic_id: ID, matches_file: true }),
    prove_attribute: async ({ kind, arg, name }) => { await prompt('prove_attribute', 1200); return { verified: true, issuer: 'Autoridade Cívica (simulada)', disclosed: kind === 'reveal' ? [[name, 'BR']] : [[`age_over_${arg}`, 'true']], nonce: 'aa' }; },
    election_load: async () => { await prompt('load_election', 900); dev.election = true; return electionSummary(); },
    election_get: async () => { if (!dev.election) throw { code: 'InvalidState', message: 'nenhuma eleição carregada', retry_after_ms: 0, remaining: 0 }; return electionSummary(); },
    election_vote: async ({ contestId, optionIndex }) => {
      const c = contests().find((x) => x.contest_id === contestId);
      if (!c.eligible) throw { code: 'NotEligible', message: 'fora do seu domicílio', retry_after_ms: 0, remaining: 0 };
      if (c.phase !== 'voting_open') throw { code: 'WrongPhase', message: 'votação fechada', retry_after_ms: 0, remaining: 0 };
      if (c.registration_required) await prompt('register', 1200);
      await prompt('sign_ballot', 1500); voted[contestId] = true;
      const rc = live[contestId]; if (rc) rc.votes[optionIndex] += 1;
      return { ballot: { election_id: 'eleicoes-exemplo-2026', contest_id: contestId, contest_title: c.title, option_index: optionIndex, option_name: c.option_names[optionIndex], voter_key: '03' + 'cd'.repeat(32), signature: 'ef'.repeat(64) }, submitted: true, board_index: 4242, note: null };
    },
    explorer_snapshot: async () => {
      if (!published) return { available: false, reason: 'O quadro de avisos ainda está vazio: publique a eleição.' };
      const cs = contests().map((c) => ({ contest_id: c.contest_id, title: c.title, office: 'governor', scope: c.scope, scope_code: c.scope_code, phase: c.phase, registration_opens: c.voting_opens - 2 * DAY, voting_opens: c.voting_opens, voting_closes: c.voting_closes, registration_required: true, options: c.option_names.map((n, i) => ({ number: String(10 + i), name: n })) }));
      return { available: true, reason: null, election_id: 'eleicoes-exemplo-2026', title: 'Eleições de exemplo (protótipo, dados fictícios)', revision: 1, head_size: 12, head_root: 'ab12cd34ef56' + '00'.repeat(26), contests: cs,
        ceremony: { threshold: 2, validators: 3, election_key: '02' + 'ab'.repeat(32), matches_manifest: true },
        governance: [{ seq: 1, contest_id: 'presidente', from: 'draft', to: 'candidates_published', at: NOW - 6 * DAY, signatures: 2, board_index: 1 }, { seq: 2, contest_id: 'presidente', from: 'candidates_published', to: 'registration_open', at: NOW - 5 * DAY, signatures: 2, board_index: 2 }] };
    },
    verify_inclusion: async ({ index }) => ({ valid: true, kind: index === 0 ? 'manifesto' : 'decisão da governança', index, size: 12, root: 'ab12cd34ef56', proof_len: 4 }),
    network_status: async () => {
      if (!published) return { mode: 'file', name: 'arquivo local', chain_id: '', threshold: 1, validators: 1, height: 0, nodes: [], blocks: [], error: null };
      const h = 7 + headExtra;
      return { mode: 'net', name: 'devnet', chain_id: 'teko-devnet-1', threshold: 2, validators: 3, height: h,
        nodes: [0, 1, 2].map((i) => ({ index: i, addr: `127.0.0.1:${7901 + i}`, up: i !== 2, height: i === 2 ? h - 2 : h, mempool: 0, peers_online: i === 2 ? 0 : 1, equivocations: 0 })),
        blocks: Array.from({ length: h }, (_, k) => h - 1 - k).map((b) => ({ height: b, entries: 1 + (b * 7) % 5, signatures: b % 4 === 3 ? 3 : 2, round: b === 4 ? 1 : 0, timestamp: Math.floor(Date.now() / 1000) - (h - b) * 2, proposer: (b + (b === 4 ? 1 : 0)) % 3, hash: 'ab12cd34ef56' + String(b).padStart(2, '0').repeat(26) })), error: null };
    },
    board_publish: async () => { published = true; return 12; },
    results_snapshot: async () => {
      if (!published) return { available: false, reason: 'O quadro de avisos ainda está vazio: publique a eleição.', election_id: '', title: '', revision: 0, visibility: 'unspecified', simulated: false, head_size: 0, head_root: '', ignored_governance: 0, contests: [] };
      tick();
      const cs = Object.values(live).map((c) => {
        const total = c.votes.reduce((a, b) => a + b, 0);
        const ranked = c.names.map((n, i) => ({ index: i, name: n, number: String(c.first + i), votes: c.votes[i] })).sort((a, b) => b.votes - a.votes || a.index - b.index);
        const top = ranked.slice(0, 10), rest = ranked.slice(10);
        return { contest_id: c.id, title: c.title, office: c.office, scope: c.scope, scope_code: c.code, phase: c.phase, accepted: total, registered: total, registration_required: true, encrypted: true, revealed: c.phase !== 'voting_open', rejected: 0, distribution_visible: c.phase !== 'voting_open', top: c.phase !== 'voting_open' ? top : [], others_votes: rest.reduce((a, b) => a + b.votes, 0), others_count: rest.length };
      });
      return { available: true, reason: null, election_id: 'eleicoes-exemplo-2026', title: 'Eleições de exemplo (protótipo, dados fictícios)', revision: 1, visibility: 'live', simulated: true, head_size: 12 + headExtra, head_root: 'ab12cd34ef56' + '00'.repeat(26), ignored_governance: 0, contests: cs };
    },
    wipe: async () => { await prompt('wipe'); dev.state = 'firmware_installed'; dev.locked = false; },
  };

  window.__TAURI__ = {
    core: { invoke: (cmd, args) => (H[cmd] ? H[cmd](args || {}) : Promise.reject({ code: 'Internal', message: `sem mock: ${cmd}` })) },
    event: { listen: async (ev, cb) => { (listeners[ev] = listeners[ev] || []).push(cb); return () => {}; } },
  };
})();
