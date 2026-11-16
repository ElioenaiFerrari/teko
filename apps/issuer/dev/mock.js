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
    { ctype: 'cnh-a', public: false, threshold: 2, members: 3, contest_secs: 1200, validity_secs: 157680000, is_member: true, credentials: [] },
    { ctype: 'cnh-b', public: false, threshold: 2, members: 3, contest_secs: 1200, validity_secs: 157680000, is_member: true, credentials: [
      cred({ id: H('1'), claims: 'nome=Ana Souza;categoria=B', can_revoke: true }),
      cred({ id: H('2'), status: 'proposed', approvals: 1, approved_by_me: false, can_approve: true, active_at: 0 }),
      cred({ id: H('3'), status: 'pending', active_at: now + 900, can_veto: true }),
      cred({ id: H('4'), status: 'vetoed', reason: 'fraude documental' }),
    ] },
    { ctype: 'candidato', public: true, threshold: 2, members: 3, contest_secs: 1200, validity_secs: 31536000, is_member: false, credentials: [
      cred({ id: H('5'), ctype: 'candidato', claims: 'cargo=governador;uf=SP;numero=13;nome=Ana Souza', status: 'pending', active_at: now + 600 }),
    ] },
  ];
  const prompt = async (kind, ms = 800) => { emit('device-prompt', kind); await wait(ms); };
  const handlers = {
    device_status: async () => status(),
    unlock: async () => { await prompt('pin_entry'); dev.locked = false; return { result: 'unlocked' }; },
    overview: async () => {
      if (dev.locked) {/* the real app returns the read-only view */}
      return { chain_id: 'teko-devnet-1-economy', now, wallet: dev.locked ? '' : 'tekow1qpzry9x8gf2tvdw0s3jn54khce6mua7lqpzry9x8gf2tvdw0s3jn54khce6muaw9z3k', wallet_note: '', orgs: orgs.map((o) => ({ ...o, is_member: o.is_member && !dev.locked })) };
    },
    holder_key: async () => { await prompt('credential_key'); return '03' + H('cd'); },
    issue: async ({ form }) => { await prompt('sign_credential_op'); const id = H('9'); return { id, bundle: JSON.stringify({ ctype: form.ctype, id, claims: form.claims, salt: H('7').slice(0, 32) }, null, 2), bundle_path: 'dev/issued/' + id + '.json' }; },
    vote: async () => { await prompt('sign_credential_op'); return 'ok'; },
  };
  window.__TAURI__ = {
    core: { invoke: async (cmd, args) => { const h = handlers[cmd]; if (!h) throw { code: 'Internal', message: 'comando sem mock: ' + cmd }; return h(args || {}); } },
    event: { listen: async (ev, cb) => { (listeners[ev] = listeners[ev] || []).push(cb); return () => {}; } },
  };
})();
