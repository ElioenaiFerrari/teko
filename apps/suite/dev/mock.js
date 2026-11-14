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
    state: q === 'virgin' ? 'virgin' : q === 'ready' || q === 'locked' ? 'credentials_issued' : 'virgin',
    locked: q === 'locked',
    fw: q === 'virgin' || q === 'none' ? 0 : 1,
    retries: 8,
  };
  const status = () => ({
    connected: dev.connected, busy: false, addr: '127.0.0.1:7878', trust_ok: true, error: null,
    status: dev.connected ? { fw_version: dev.fw, state: dev.state, locked: dev.locked, retries_remaining: dev.retries, label: 'Teko Emulator', has_credential: dev.state === 'credentials_issued', capabilities: [] } : null,
  });
  const prompt = async (kind, ms = 900) => { emit('device-prompt', kind); await wait(ms); };
  const offer = (v) => ({ version: v, size: 49152, fingerprint: v === 1 ? 'a1b2 c3d4 e5f6 0718' : '9f8e 7d6c 5b4a 3921', valid_signers: 2, threshold: 2, total_signers: 3, installed_version: dev.fw, is_downgrade: v < dev.fw, verdict: v < dev.fw ? 'Versão anterior à instalada: recusado (anti-downgrade)' : v === dev.fw ? 'Já instalado' : dev.fw ? 'Atualização disponível' : 'Pronto para instalar', installable: v > dev.fw });

  const H = {
    device_status: async () => status(),
    dev_info: async () => ({ dev_dir: 'dev', addr: '127.0.0.1:7878', authority_root: '02ab' }),
    verify_authenticity: async () => { await wait(600); return { genuine: true, batch_id: 'lote-dev-001', fw_version: dev.fw, reason: null }; },
    firmware_catalog: async () => [offer(1), offer(2)],
    install_firmware: async ({ version }) => { await prompt('firmware_install'); for (let i = 1; i <= 5; i++) { emit('fw-progress', [i * 10, 50]); await wait(150); } dev.fw = version; if (dev.state === 'virgin') dev.state = 'firmware_installed'; return version; },
    setup_pin: async () => { await prompt('pin_entry', 1200); dev.state = 'initialized'; },
    unlock: async () => { await prompt('pin_entry', 1000); dev.locked = false; return { result: 'unlocked' }; },
    civic_id: async () => { if (dev.state === 'initialized') await prompt('confirm_civic_id'); return { id: ID, public_key: '02' + 'ab'.repeat(32) }; },
    issue_credential: async () => { await prompt('issue_identity'); await prompt('load_credential'); dev.state = 'credentials_issued'; },
    reveal_identity: async () => { for (let i = 0; i < 4; i++) await prompt('prove_attribute', 500); return [['name', 'Maria da Silva'], ['birth_date', '1990-05-17'], ['nationality', 'BR'], ['registry_id', 'TK-9A41C07E']]; },
    pairwise_id: async ({ service }) => { await prompt('pairwise_id'); return 'tekop1' + btoa(service).replace(/[^a-z0-9]/gi, '').toLowerCase().slice(0, 32); },
    hash_file: async () => '3a1f9c0d77be42e18a5c6d09f4b2e7138c5d6a90b1e2f3748596a7b8c9d0e1f2',
    sign_document: async ({ summary }) => { await prompt('sign_document', 1400); return { format: 'teko-signed-document/1', hash_sha3_256: '3a1f9c0d77be42e18a5c6d09f4b2e7138c5d6a90b1e2f3748596a7b8c9d0e1f2', summary, civic_id: ID, public_key: '02' + 'ab'.repeat(32), signature: 'cd'.repeat(64), scheme: 'ECDSA-P256' }; },
    save_signed_document: async () => '/Users/voce/Downloads/contrato.teko-signature.json',
    verify_document: async () => ({ signature_valid: true, civic_id: ID, matches_file: true }),
    prove_attribute: async ({ kind, arg, name }) => { await prompt('prove_attribute', 1200); return { verified: true, issuer: 'Autoridade Cívica (simulada)', disclosed: kind === 'reveal' ? [[name, 'BR']] : [[`age_over_${arg}`, 'true']], nonce: 'aa' }; },
    cast_vote: async () => { await prompt('sign_ballot', 1000); throw { code: 'NotImplemented', message: 'stub', retry_after_ms: 0, remaining: 0 }; },
    wipe: async () => { await prompt('wipe'); dev.state = 'firmware_installed'; dev.locked = false; },
  };

  window.__TAURI__ = {
    core: { invoke: (cmd, args) => (H[cmd] ? H[cmd](args || {}) : Promise.reject({ code: 'Internal', message: `sem mock: ${cmd}` })) },
    event: { listen: async (ev, cb) => { (listeners[ev] = listeners[ev] || []).push(cb); return () => {}; } },
  };
})();
