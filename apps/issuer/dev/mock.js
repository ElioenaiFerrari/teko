/* Somente desenvolvimento: simula o backend do Emissor para pré-visualizar a UI num navegador comum.
   Só ativa com `?mock=ready|locked|none` e sem o runtime do Tauri. */
(() => {
  const q = new URLSearchParams(location.search).get('mock');
  if (window.__TAURI__ || !q) return;
  const wait = (ms) => new Promise((r) => setTimeout(r, ms));
  const listeners = {};
  const emit = (ev, payload) => (listeners[ev] || []).forEach((cb) => cb({ payload }));
  const now = Math.floor(Date.now() / 1000);
  const dev = { connected: q !== 'none', locked: q === 'locked' };
  const status = () => ({
    connected: dev.connected, busy: false, addr: '127.0.0.1:7878', error: dev.connected ? null : 'conexão recusada',
    status: dev.connected ? { fw_version: 1, state: 'credentials_issued', locked: dev.locked, retries_remaining: 8, label: 'Teko Emulator', has_credential: true, wallet_ready: true, capabilities: [] } : null,
  });
  const H = (c) => c.repeat(64).slice(0, 64);
  const cred = (o) => ({ ctype: 'cnh-b', subject: '02' + H('ab').slice(0, 64), status: 'active', approvals: 2, threshold: 2, approved_by_me: true, valid_until: now + 86400 * 900, active_at: now - 3600, reason: '', claims: null, can_approve: false, can_veto: false, can_revoke: false, ...o });
  let orgs = [
    { ctype: 'civic', public: false, threshold: 2, members: 3, contest_secs: 1200, validity_secs: 315360000, is_member: true, credentials: [cred({ id: H('c'), ctype: 'civic', claims: 'name=Maria da Silva;registry_id=TK-9A41C07E', status: 'pending', active_at: now + 600 })] },
    { ctype: 'cnh-a', public: false, threshold: 2, members: 3, contest_secs: 1200, validity_secs: 315360000, is_member: true, credentials: [] },
    { ctype: 'cnh-b', public: false, threshold: 2, members: 3, contest_secs: 1200, validity_secs: 315360000, is_member: true, credentials: [
      cred({ id: H('1'), claims: 'nome=Ana Souza;categoria=B', can_revoke: true }),
      cred({ id: H('2'), status: 'proposed', approvals: 1, approved_by_me: false, can_approve: true, active_at: 0 }),
      cred({ id: H('3'), status: 'pending', active_at: now + 900, can_veto: true }),
      cred({ id: H('4'), status: 'vetoed', reason: 'fraude documental' }),
    ] },
    { ctype: 'certidao-casamento', public: false, threshold: 2, members: 3, contest_secs: 1200, validity_secs: 0, is_member: true, credentials: [] },
    { ctype: 'certidao-divorcio', public: false, threshold: 2, members: 3, contest_secs: 1200, validity_secs: 0, is_member: true, credentials: [] },
    { ctype: 'candidato', public: true, threshold: 2, members: 3, contest_secs: 1200, validity_secs: 31536000, is_member: false, credentials: [
      cred({ id: H('5'), ctype: 'candidato', claims: 'cargo=governador;uf=SP;numero=13;nome=Ana Souza', status: 'pending', active_at: now + 600 }),
    ] },
  ];
  const prompt = async (kind, ms = 800) => { emit('device-prompt', kind); await wait(ms); };
  const marriages = [{ matricula: '2020.0145.77', data: '2020-02-29', regime: 'comunhao-parcial', cartorio: '1º Ofício de Registro Civil de São Paulo', active: true, spouses: [{ nome: 'Ana Souza', conjuge: 'Beto Lima', id: H('a'), status: 'active' }, { nome: 'Beto Lima', conjuge: 'Ana Souza', id: H('b'), status: 'active' }] }];
  const handlers = {
    civil_marriages: async () => marriages,
    civil_marriage_issue: async ({ form }) => { await prompt('sign_credential_op', 900); await prompt('sign_credential_op', 900); return [{ nome: form.a.nome, id: H('c'), error: '' }, { nome: form.b.nome, id: H('d'), error: '' }]; },
    civil_divorce_issue: async () => { await prompt('sign_credential_op', 900); return [{ nome: 'Ana Souza', id: H('e'), error: '' }, { nome: 'Beto Lima', id: H('f'), error: '' }]; },
    device_status: async () => status(),
    unlock: async () => { await prompt('pin_entry'); dev.locked = false; return { result: 'unlocked' }; },
    overview: async () => {
      if (dev.locked) {/* the real app returns the read-only view */}
      return { chain_id: 'teko-devnet-1-economy', now, wallet: dev.locked ? '' : 'tekow1qpzry9x8gf2tvdw0s3jn54khce6mua7lqpzry9x8gf2tvdw0s3jn54khce6muaw9z3k', wallet_note: '', orgs: orgs.map((o) => ({ ...o, is_member: o.is_member && !dev.locked })) };
    },
    holder_key: async () => { await prompt('credential_key'); return '03' + H('cd'); },
    issue: async ({ form }) => { await prompt('sign_credential_op'); const id = H('9'); return { id, bundle: JSON.stringify({ ctype: form.ctype, id, claims: form.claims, salt: H('7').slice(0, 32) }, null, 2), bundle_path: 'dev/issued/' + id + '.json' }; },
    vote: async () => { await prompt('sign_credential_op'); return 'ok'; },
    civic_requests: async () => [{ file: 'civic-request-1a2b3c4d.json', civic_id: 'teko1qpzry9x8gf2tvdw0s3jn54khce6mua7l', verified: true, error: '' }],
    civic_issue: async () => { await prompt('sign_credential_op'); return { id: H('8'), bundle: JSON.stringify({ ctype: 'civic' }), bundle_path: 'dev/issued/x.json' }; },
    civic_deliver: async () => 'dev/inbox/civic-1a2b3c4d.bundle',
  };
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => { const h = handlers[cmd]; if (!h) throw { code: 'Internal', message: 'comando sem mock: ' + cmd }; return h(args || {}); } },
    event: { listen: async (ev, cb) => { (listeners[ev] = listeners[ev] || []).push(cb); return () => {}; } },
  };
})();
