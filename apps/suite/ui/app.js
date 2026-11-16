/* Teko Suite — UI. Sem framework: um estado, uma função render(), ações por data-act.
   A Suite NUNCA vê o PIN: ele é digitado no dispositivo (emulador). */
(() => {
  'use strict';

  const TAURI = window.__TAURI__;
  const invoke = (cmd, args) => TAURI.core.invoke(cmd, args);
  const listen = (ev, cb) => TAURI.event.listen(ev, cb);

  // ───────────────────────── ícones (SVG inline, sem emoji) ─────────────────────────
  const P = {
    usb: '<rect x="7" y="3" width="10" height="7" rx="1.5"/><path d="M9.5 10v2.5M14.5 10v2.5"/><rect x="6" y="12.5" width="12" height="8" rx="2"/><path d="M10 16.5h4"/>',
    shield: '<path d="M12 3l7 3v5c0 5-3.5 8.5-7 10-3.5-1.5-7-5-7-10V6l7-3z"/><path d="M9 12l2 2 4-4"/>',
    key: '<circle cx="8" cy="15" r="4"/><path d="M11 12l9-9M16 7l3 3"/>',
    wallet: '<rect x="3" y="6" width="18" height="13" rx="2"/><path d="M3 10h18M16 14.5h2"/>',
    home: '<path d="M3 11l9-7 9 7v9H3z"/><path d="M9 20v-6h6v6"/>',
    stake: '<path d="M12 3l8 4v5c0 4.5-3.2 7.5-8 9-4.8-1.5-8-4.5-8-9V7z"/><path d="M9 12l2 2 4-4"/>',
    pen: '<path d="M4 20h16"/><path d="M5 16l9-9 3 3-9 9H5v-3z"/>',
    badge: '<circle cx="12" cy="12" r="9"/><path d="M8 12l3 3 5-6"/>',
    ballot: '<rect x="4" y="9" width="16" height="11" rx="2"/><path d="M8 9V5h8v4M9 14h6"/>',
    globe: '<circle cx="12" cy="12" r="9"/><path d="M3 12h18"/><path d="M12 3c3 3 3 15 0 18-3-3-3-15 0-18z"/>',
    lock: '<rect x="5" y="11" width="14" height="9" rx="2"/><path d="M8 11V8a4 4 0 0 1 8 0v3"/>',
    unlock: '<rect x="5" y="11" width="14" height="9" rx="2"/><path d="M8 11V8a4 4 0 0 1 7-2.6"/>',
    alert: '<path d="M12 3l10 18H2L12 3z"/><path d="M12 10v5M12 18h.01"/>',
    check: '<path d="M5 13l4 4L19 7"/>',
    x: '<path d="M6 6l12 12M18 6L6 18"/>',
    copy: '<rect x="9" y="9" width="11" height="11" rx="2"/><path d="M5 15V6a2 2 0 0 1 2-2h8"/>',
    upload: '<path d="M12 16V4M7 9l5-5 5 5M4 20h16"/>',
    file: '<path d="M6 3h8l4 4v14H6z"/><path d="M14 3v4h4"/>',
    eye: '<path d="M2 12s4-7 10-7 10 7 10 7-4 7-10 7S2 12 2 12z"/><circle cx="12" cy="12" r="3"/>',
    sun: '<circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M2 12h2M20 12h2M5 5l1.5 1.5M17.5 17.5L19 19M5 19l1.5-1.5M17.5 6.5L19 5"/>',
    moon: '<path d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z"/>',
    monitor: '<rect x="3" y="4" width="18" height="12" rx="2"/><path d="M8 20h8M12 16v4"/>',
    spin: '<path d="M12 3a9 9 0 1 0 9 9"/>',
    id: '<rect x="3" y="5" width="18" height="14" rx="2"/><circle cx="9" cy="11" r="2"/><path d="M6 16c.5-2 5.5-2 6 0M14 10h4M14 14h4"/>',
    list: '<path d="M8 6h12M8 12h12M8 18h12M4 6h.01M4 12h.01M4 18h.01"/>',
    users: '<circle cx="9" cy="8" r="3"/><path d="M3 20c0-3.3 2.7-6 6-6s6 2.7 6 6"/><path d="M16 5a3 3 0 0 1 0 6M18 14c1.7.7 3 2.5 3 5"/>',
    chart: '<path d="M4 20V4M4 20h16"/><path d="M8 16v-4M12 16V8M16 16v-6"/>',
    link: '<path d="M10 14a4 4 0 0 0 5.7 0l3-3a4 4 0 0 0-5.7-5.7l-1 1"/><path d="M14 10a4 4 0 0 0-5.7 0l-3 3A4 4 0 0 0 11 18.7l1-1"/>',
    trash: '<path d="M4 7h16M9 7V4h6v3M6 7l1 13h10l1-13"/>',
  };
  const ico = (n, c = '') => `<svg class="i ${c}" viewBox="0 0 24 24" aria-hidden="true">${P[n] || ''}</svg>`;

  // ───────────────────────── util ─────────────────────────
  // ───────────────────────── seleção com busca ─────────────────────────
  const G = window.TEKO_GEO || { ufs: [], municipios: {}, paises: [] };
  const geoCountries = () => G.paises.map(([v, n]) => ({ value: v, label: n, hint: v }));
  const geoStates = () => G.ufs.map(([v, n]) => ({ value: v, label: n, hint: v }));
  const geoCities = (uf) => (G.municipios[uf] || []).map(([v, n]) => ({ value: String(v), label: n, hint: uf }));
  const fold = (s) => String(s).normalize('NFD').replace(/[\u0300-\u036f]/g, '').toLowerCase();
  const comboData = {};   // id -> opções; só a escolha feita NA LISTA vale (nada de texto livre)
  function combo(id, items, value, placeholder) {
    comboData[id] = items;
    const cur = items.find((i) => i.value === value);
    return `<div class="combo" data-combo="${id}" data-value="${esc(cur ? cur.value : '')}">
      <input class="input" id="${id}" role="combobox" aria-expanded="false" aria-controls="${id}-list" aria-autocomplete="list" autocomplete="off" spellcheck="false" placeholder="${esc(placeholder)}" value="${esc(cur ? cur.label : '')}">
      <ul class="combo__list" id="${id}-list" role="listbox" hidden></ul></div>`;
  }
  const comboValue = (id) => { const w = document.querySelector(`[data-combo="${id}"]`); return w ? w.dataset.value : ''; };
  function comboOpen(w, query) {
    const id = w.dataset.combo, list = $('.combo__list', w), input = $('input', w);
    const q = fold(query || '');
    const hits = (comboData[id] || []).filter((i) => !q || fold(i.label).includes(q) || fold(i.hint).startsWith(q)).slice(0, 60);
    list.innerHTML = hits.length
      ? hits.map((i, k) => `<li class="combo__opt" role="option" data-v="${esc(i.value)}" aria-selected="${k === 0}"><span>${esc(i.label)}</span><small>${esc(i.hint)}</small></li>`).join('')
      : '<li class="combo__empty">Nada encontrado</li>';
    list.hidden = false; input.setAttribute('aria-expanded', 'true');
  }
  function comboClose(w) { $('.combo__list', w).hidden = true; $('input', w).setAttribute('aria-expanded', 'false'); }
  function comboPick(w, value) {
    const id = w.dataset.combo, item = (comboData[id] || []).find((i) => i.value === value);
    if (!item) return;
    w.dataset.value = item.value; $('input', w).value = item.label; comboClose(w);
    if (id === 'f-uf') {   // o município depende do estado
      const m = document.querySelector('[data-combo="f-mun"]');
      if (m) { comboData['f-mun'] = geoCities(item.value); m.dataset.value = ''; $('input', m).value = ''; $('input', m).focus(); comboOpen(m, ''); }
    }
  }
  document.addEventListener('input', (e) => { const w = e.target.closest && e.target.closest('.combo'); if (w) { w.dataset.value = ''; comboOpen(w, e.target.value); } });
  document.addEventListener('focusin', (e) => { const w = e.target.closest && e.target.closest('.combo'); if (w && e.target.tagName === 'INPUT') { e.target.select(); comboOpen(w, ''); } });
  document.addEventListener('mousedown', (e) => {
    const opt = e.target.closest && e.target.closest('.combo__opt');
    if (opt) { e.preventDefault(); comboPick(opt.closest('.combo'), opt.dataset.v); return; }
    document.querySelectorAll('.combo').forEach((w) => { if (!w.contains(e.target)) comboClose(w); });
  });
  document.addEventListener('focusout', (e) => {
    const w = e.target.closest && e.target.closest('.combo'); if (!w) return;
    setTimeout(() => {   // saiu do campo sem escolher: volta à última escolha válida
      if (w.contains(document.activeElement)) return;
      comboClose(w);
      const item = (comboData[w.dataset.combo] || []).find((i) => i.value === w.dataset.value);
      $('input', w).value = item ? item.label : '';
    }, 0);
  });
  document.addEventListener('keydown', (e) => {
    const w = e.target.closest && e.target.closest('.combo'); if (!w) return;
    const list = $('.combo__list', w), opts = [...list.querySelectorAll('.combo__opt')];
    const at = opts.findIndex((o) => o.getAttribute('aria-selected') === 'true');
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      e.preventDefault(); if (list.hidden) comboOpen(w, ''); else if (opts.length) {
        const n = (at + (e.key === 'ArrowDown' ? 1 : -1) + opts.length) % opts.length;
        opts.forEach((o, k) => o.setAttribute('aria-selected', k === n)); opts[n].scrollIntoView({ block: 'nearest' });
      }
    } else if (e.key === 'Enter') { if (!list.hidden && opts[at]) { e.preventDefault(); comboPick(w, opts[at].dataset.v); } }
    else if (e.key === 'Escape') { comboClose(w); }
  });

  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const $ = (sel, root = document) => root.querySelector(sel);
  const groups = (hex, n = 4) => (hex.match(new RegExp(`.{1,${n}}`, 'g')) || []).map((g) => `<span>${esc(g)}</span>`).join('');
  const short = (s, a = 10, b = 8) => (s.length > a + b + 3 ? `${s.slice(0, a)}…${s.slice(-b)}` : s);
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  const asErr = (e) => (e && typeof e === 'object' && 'message' in e ? e : { code: 'Internal', message: String(e), retry_after_ms: 0, remaining: 0 });

  function toast(msg, kind = 'bad') {
    const el = document.createElement('div');
    el.className = `toast toast--${kind}`;
    el.setAttribute('role', kind === 'bad' ? 'alert' : 'status');
    el.innerHTML = `${ico(kind === 'ok' ? 'check' : 'alert')}<div>${esc(msg)}</div>`;
    $('#toasts').appendChild(el);
    setTimeout(() => el.remove(), kind === 'ok' ? 4500 : 8000);
  }

  // ───────────────────────── estado ─────────────────────────
  const S = {
    route: null,
    userPicked: false,
    dev: { connected: false, busy: false, status: null, trust_ok: true, addr: '' },
    info: null,
    genuine: null,       // AuthenticityReport
    authAck: false,
    authRunning: false,
    fw: null,
    fwProgress: null,
    civic: null,
    identity: null,      // dados revelados
    pairwise: null,
    signed: null,        // resultado da assinatura
    file: null,          // { name, bytes, hash }
    verifyRes: null,
    prove: { choice: 'age18', verifier: 'Loja Exemplo', result: null, error: null },
    election: null,      // resumo vindo do dispositivo
    elecLoading: false,
    results: null,        // última resposta de results_snapshot
    resultsAt: 0,
    resultsOffice: null,  // aba de cargo selecionada
    resultsErr: null,
    resultsShape: null,
    elecChecked: false,
    explorer: null, explorerTried: false, inclusion: null, net: null,
    devMode: true,   // false numa versão de produção: sem autoridade/operador de teste
    idTried: false,       // evitam laços de re-tentativa em loadId/loadFirmware
    fwTried: false,
    fwLoading: false,   // já perguntei ao dispositivo por uma eleição nesta visita?
    picks: {},           // contest_id -> índice escolhido
    ballots: {},         // contest_id -> cédula votada (só rastreador; a opção não é guardada)
    audits: {},          // contest_id -> última cédula de teste auditada
    checks: {},          // contest_id -> conferência da própria cédula
    lockMsg: null,
    lastAck: false,
    wiped: false,
    prevState: null,
    expectWipe: false,
    wipeArmed: false,
    busy: false,
    prompt: null,
    promptTimer: null,
    lastKey: '',
    // formulário de registro de bem de teste (as regras de validação ficam no Rust)
    reg: { class: 'veiculo', form: null, values: {}, error: '', loading: false },
  };

  // Moeda TEKO (ADR-0029): o livro guarda inteiros em ypy (1 TEKO = 1.000.000 ypy);
  // aqui só aritmética inteira (exata abaixo de 2^53; o teto é 2,1e13), nunca ponto flutuante.
  const YPY = 1000000, DECIMALS = 6;
  const INT_FMT = new Intl.NumberFormat('pt-BR', { maximumFractionDigits: 0 });
  // 1234500000 -> "1.234,5" (sem zeros à direita, sem arredondar)
  const money = (n) => {
    const v = Math.max(0, Math.trunc(Number(n) || 0));
    const whole = Math.floor(v / YPY), frac = v % YPY;
    const f = String(frac).padStart(DECIMALS, '0').replace(/0+$/, '');
    return INT_FMT.format(whole) + (f ? ',' + f : '');
  };
  // O que a pessoa digitou -> ypy (inteiro) ou null. Aceita 1234, 1234,5, 1.234,5, 1234.5.
  const parseTeko = (text) => {
    let t = String(text || '').trim().replace(/\s*TEKO$/i, '');
    if (!/^[0-9.,]+$/.test(t)) return null;
    let ip, fp = '';
    if (t.includes(',')) {
      const [i, f, ...rest] = t.split(',');
      if (rest.length || f.includes('.')) return null;
      ip = i; fp = f;
      if (ip.includes('.') && !/^\d{1,3}(\.\d{3})+$/.test(ip)) return null;
      ip = ip.replace(/\./g, '');
    } else if ((t.match(/\./g) || []).length === 1 && !/^[1-9]\d{0,2}\.\d{3}$/.test(t)) {
      [ip, fp] = t.split('.');
    } else if (t.includes('.')) {
      if (!/^\d{1,3}(\.\d{3})+$/.test(t)) return null;
      ip = t.replace(/\./g, '');
    } else ip = t;
    if (!ip && !fp) return null;
    if (fp.length > DECIMALS || ip.length > 14) return null;
    const v = Number(ip || '0') * YPY + Number(fp.padEnd(DECIMALS, '0'));
    return Number.isSafeInteger(v) ? v : null;
  };
  const AMOUNT_HELP = 'Em TEKO, até 6 casas decimais (1 TEKO = 1.000.000 ypy). Ex.: 12,5 ou 0,000001.';

  const NAV = [
    { id: 'onboarding', label: 'Configurar dispositivo', icon: 'usb', need: 'onboarding' },
    { id: 'home', label: 'Início', icon: 'chart', need: 'ready' },
    { id: 'identity', label: 'Minha identidade', icon: 'id', need: 'ready' },
    { id: 'wallet', label: 'Carteira', icon: 'wallet', need: 'ready' },
    { id: 'assets', label: 'Bens', icon: 'home', need: 'ready' },
    { id: 'credentials', label: 'Credenciais', icon: 'badge', need: 'ready' },
    { id: 'stake', label: 'Validadores', icon: 'stake', need: 'ready' },
    { id: 'sign', label: 'Assinar documento', icon: 'pen', need: 'ready' },
    { id: 'prove', label: 'Provar atributo', icon: 'badge', need: 'ready' },
    { id: 'vote', label: 'Votar', icon: 'ballot', need: 'ready' },
  ];

  function phase() {
    const d = S.dev;
    if (!d.connected || !d.status) return 'none';
    const st = d.status;
    if (st.locked) return 'locked';
    if (st.state === 'credentials_issued') return 'ready';
    // initialized with a confirmed wallet: money, assets, credentials and stake work before the civic ID
    if (st.state === 'initialized' && st.wallet_ready) return 'wallet';
    return 'onboarding';
  }

  function defaultRoute() {
    switch (phase()) {
      case 'none': return 'explorer';
      case 'locked': return 'unlock';
      case 'onboarding':
      case 'wallet': return 'onboarding';
      default: return 'home';
    }
  }

  function routeAllowed(r) {
    const p = phase();
    if (r === 'explorer' || r === 'results') return true;
    if (r === 'unlock') return p === 'locked';
    if (r === 'onboarding') return p === 'onboarding' || p === 'wallet';
    if (p === 'wallet') return ['wallet', 'assets', 'credentials', 'stake'].includes(r);
    return p === 'ready';
  }

  // ───────────────────────── chamadas ao backend ─────────────────────────
  const PROMPTS = {
    pin_entry: ['Digite o PIN no dispositivo', 'Olhe o terminal do emulador. Esta janela nunca vê o que você digita.'],
    firmware_install: ['Confirme a instalação no dispositivo', 'Compare a impressão digital abaixo com a da tela do dispositivo.'],
    confirm_civic_id: ['Confira o ID Cívico no dispositivo', 'O ID exibido no dispositivo é a referência. Se for diferente, recuse.'],
    sign_document: ['Confirme a assinatura no dispositivo', 'Compare o hash abaixo com o da tela do dispositivo. Só assine se forem idênticos.'],
    prove_attribute: ['Confirme a prova no dispositivo', 'O dispositivo mostra exatamente o que será revelado e a quem.'],
    pairwise_id: ['Confirme no dispositivo', 'Será gerado um identificador exclusivo para este serviço.'],
    issue_identity: ['Autorize a emissão no dispositivo', 'O dispositivo vai provar à autoridade que é genuíno e que possui a chave.'],
    load_credential: ['Confirme a instalação da credencial', 'Confira o emissor e os dados mostrados no dispositivo.'],
    sign_ballot: ['Confirme o voto no dispositivo', 'Confira o cargo, a jurisdição e o NOME do candidato que o dispositivo mostra. Só confirme se for o seu voto.'],
    load_election: ['Confirme a eleição no dispositivo', 'O dispositivo verificou as assinaturas das instituições. Confira o título e os pleitos.'],
    register: ['Confirme o registro anônimo no dispositivo', 'A autoridade confere QUEM você é, mas assina às cegas: ela não vê a sua chave de eleitor. Depois o dispositivo pedirá a confirmação do voto.'],
    voter_key: ['Confirme no dispositivo', 'Será exportada a chave PÚBLICA de eleitor deste pleito.'],
    export_commitment: ['Confirme no dispositivo', 'Exportar o compromisso de eleitor.'],
    wipe: ['Confirme o apagamento no dispositivo', 'No dispositivo, digite APAGAR para confirmar. Isso é irreversível.'],
    wallet_create: ['Crie a carteira no dispositivo', 'O dispositivo vai mostrar 24 palavras NA TELA DELE. Elas nunca passam por este computador. Tenha papel e caneta.'],
    seed_words: ['Anote as palavras no dispositivo', 'Escreva cada palavra, na ordem, em papel. Não fotografe nem digite em nenhum computador. Confirme cada página no dispositivo.'],
    wallet_confirm: ['Confirme a sua anotação no dispositivo', 'O dispositivo vai pedir palavras de posições sorteadas. Digite-as no dispositivo, lendo do seu papel.'],
    wallet_passphrase: ['Escolha no dispositivo: usar passphrase?', 'Uma senha extra protege a frase, mas esquecê-la significa perder os fundos. Decida no dispositivo.'],
    wallet_restore: ['Restaure a carteira no dispositivo', 'Digite as 24 palavras NO DISPOSITIVO e confira o endereço mostrado. Esta janela nunca vê as palavras.'],
    sign_stake: ['Confirme o stake no dispositivo', 'Confira a ação, o valor e a taxa que o dispositivo mostra. Quem trava stake pode ser penalizado se votar duas vezes.'],
    sign_asset_offer: ['Confirme a sua OFERTA no dispositivo', 'Confira o BEM, o comprador, o preço, a janela de veto e a versão que o dispositivo mostra. Esta é a SUA transação, assinada na sua carteira. Só confirme se for exatamente o negócio combinado.'],
    sign_asset_accept: ['Confirme o ACEITE no dispositivo', 'Confira o BEM, o vendedor, o preço e a janela de veto. O preço sai da sua carteira para um escrow e só volta se o órgão vetar. Só confirme se for exatamente o negócio combinado.'],
    credential_key: ['Confirme a CHAVE DE CREDENCIAL no dispositivo', 'Só a chave pública deste tipo de credencial será mostrada ao órgão que vai emitir. Ela é derivada da sua carteira (volta com a frase de 24 palavras) e não se liga publicamente à sua identidade, ao seu endereço nem ao seu voto.'],
    ballot_cast: ['Confirme no dispositivo: VOTAR ou AUDITAR', 'Confirmar = esta cédula cifrada é o seu voto. Recusar = voltar e, se quiser, auditar esta cédula de teste (ela é descartada).'],
    ballot_audit: ['Confirme no dispositivo: auditar a cédula', 'O dispositivo mostra ao computador em quem esta cédula de teste votava e como a cifrou. Ela NÃO é usada como voto.'],
    check_ballot: ['Confirme: conferir a minha cédula', 'O dispositivo confere, sozinho, que a sua cédula está num bloco certificado pelas instituições. O seu voto não é mostrado.'],
    open_credential: ['Confirme: abrir os atributos da credencial', 'O dispositivo decifra, com a chave derivada da sua carteira, os atributos guardados na chain e mostra o que vai entregar à Suite.'],
    sign_credential_op: ['Confirme a operação do ÓRGÃO no dispositivo', 'Confira o tipo, o titular, os atributos, a validade e o motivo na tela do dispositivo.'],
    sign_transfer: ['Confirme o envio no dispositivo', 'Confira o ENDEREÇO de destino, o valor e a taxa que o dispositivo mostra. Só confirme se for exatamente o que você quer enviar.'],
    wallet_check: ['Verifique o backup no dispositivo', 'Digite a frase do seu papel no dispositivo. Ele só diz se confere ou não.'],
    unknown: ['Aguardando o dispositivo', 'Olhe a tela do dispositivo.'],
  };

  // Atualiza o modal NO LUGAR: recriar o HTML reinicia as animações (piscada).
  function renderOverlay() {
    const ov = $('#overlay');
    const kind = S.prompt;
    const [title, body] = kind ? PROMPTS[kind] || PROMPTS.unknown : ['Falando com o dispositivo…', 'Aguarde um instante.'];
    const expect = S.opts && S.opts.expect;
    if (!$('.overlay__card', ov)) {
      ov.innerHTML = `
        <div class="overlay__card">
          <div class="row">
            <div class="pulse" id="ov-icon"></div>
            <div><h2 id="ov-title"></h2><p class="muted" id="ov-body"></p></div>
          </div>
          <div class="screen" id="ov-expect" hidden><div class="screen__label"></div><div class="blocks"></div></div>
          <div class="callout callout--info">${ico('shield')}<div><strong>A Suite não confia neste computador.</strong>
            Quem decide é o dispositivo: PIN e confirmações acontecem só nele.</div></div>
        </div>`;
    }
    const set = (el, key, html) => { if (el.dataset[key] !== html) { el.dataset[key] = html; el.innerHTML = html; } };
    const icon = $('#ov-icon');
    const state = kind ? 'wait' : 'spin';
    if (icon.dataset.state !== state) { icon.dataset.state = state; icon.innerHTML = ico(kind ? 'usb' : 'spin', kind ? '' : 'spin'); }
    const t = $('#ov-title'); if (t.textContent !== title) t.textContent = title;
    const b = $('#ov-body'); if (b.textContent !== body) b.textContent = body;
    const box = $('#ov-expect');
    box.hidden = !expect;
    if (expect) {
      const label = $('.screen__label', box); if (label.textContent !== expect.label) label.textContent = expect.label;
      set($('.blocks', box), 'v', groups(expect.value));
    }
  }

  async function call(cmd, args = {}, opts = {}) {
    S.busy = true;
    S.opts = opts;
    S.prompt = null;
    const t = setTimeout(() => { renderOverlay(); $('#overlay').hidden = false; }, opts.delay ?? 200);
    try {
      return await invoke(cmd, args);
    } catch (e) {
      throw asErr(e);
    } finally {
      clearTimeout(t);
      $('#overlay').hidden = true;
      S.busy = false;
      S.prompt = null;
      S.opts = null;
    }
  }

  async function guarded(fn, { silent = false } = {}) {
    try {
      return await fn();
    } catch (e) {
      if (!silent) toast(friendly(e));
      if (e.code === 'DeviceWiped') { resetSession(); S.wiped = true; await poll(); }
      return undefined;
    }
  }

  function friendly(e) {
    switch (e.code) {
      case 'UserRejected': return 'Você recusou no dispositivo. Nada foi feito.';
      case 'Timeout': return 'Tempo esgotado no dispositivo. Tente de novo.';
      case 'PinLocked': return `Muitas tentativas. Aguarde ${Math.ceil((e.retry_after_ms || 0) / 1000)} s antes de tentar de novo.`;
      case 'DeviceWiped': return 'Excesso de tentativas: o dispositivo foi apagado. É preciso reemissão presencial.';
      case 'Transport': return 'Perdi a conexão com o dispositivo. Verifique se o emulador está aberto.';
      default: return e.message;
    }
  }

  function resetSession() {
    S.genuine = null; S.authAck = false; S.fw = null; S.civic = null; S.civicReq = null; S.recovery = null; S.identity = null;
    S.pairwise = null; S.signed = null; S.file = null; S.verifyRes = null; S.prove.result = null;
    S.election = null; S.elecChecked = false; S.idTried = false; S.fwTried = false; S.picks = {}; S.ballots = {}; S.wipeArmed = false; S.lastKey = '';
  }

  // ───────────────────────── polling ─────────────────────────
  async function poll() {
    if (S.busy) return;
    try {
      const r = await invoke('device_status');
      if (r.busy) return;
      const was = S.dev;
      S.dev = r;
      if (was.connected && !r.connected) { resetSession(); toast('Dispositivo desconectado.'); }
      const stNow = r.connected && r.status ? r.status.state : null;
      const had = ['initialized', 'credentials_issued'].includes(S.prevState);
      if (had && (stNow === 'virgin' || stNow === 'firmware_installed') && !S.expectWipe) { resetSession(); S.wiped = true; }
      if (stNow) S.prevState = stNow;
      if (stNow === 'initialized' || stNow === 'credentials_issued') S.wiped = false;
      if (r.connected && r.status && r.status.state === 'virgin' && S.civic) resetSession();
    } catch { /* ignora: tenta no próximo ciclo */ }
    render();
  }

  // ───────────────────────── renderização ─────────────────────────
  function render() {
    if (!S.route || !routeAllowed(S.route) || (!S.userPicked && S.route !== defaultRoute())) {
      S.route = routeAllowed(S.route) && S.userPicked ? S.route : defaultRoute();
    }
    renderSide();
    const st = S.dev.status;
    const key = [S.route, phase(), st && st.state, st && st.locked, S.dev.connected, onboardStep()].join('|');
    if (key !== S.lastKey) { S.lastKey = key; renderMain(); }
  }

  function chip(text, kind = '', icon = '') {
    return `<span class="chip ${kind ? `chip--${kind}` : ''}">${icon ? ico(icon) : ''}${esc(text)}</span>`;
  }

  function renderSide() {
    const d = S.dev, st = d.status, p = phase();
    let title = 'Nenhum dispositivo', sub = `Procurando em ${d.addr || '127.0.0.1:7878'}`, icon = 'usb';
    const chips = [];
    if (p !== 'none') {
      title = st.label || 'Dispositivo Teko';
      sub = `Firmware ${st.fw_version ? `v${st.fw_version}` : 'não instalado'}`;
      chips.push(chip('Conectado', 'ok', 'check'));
      if (S.genuine && S.genuine.genuine) chips.push(chip('Genuíno', 'ok', 'shield'));
      if (S.genuine && !S.genuine.genuine) chips.push(chip('Falhou', 'bad', 'alert'));
      if (st.state !== 'virgin' && st.state !== 'firmware_installed') chips.push(st.locked ? chip('Bloqueado', 'warn', 'lock') : chip('Desbloqueado', 'info', 'unlock'));
      if (st.state !== 'virgin' && st.state !== 'firmware_installed' && st.retries_remaining < 8) chips.push(chip(`${st.retries_remaining} ${st.retries_remaining === 1 ? 'tentativa' : 'tentativas'}`, st.retries_remaining <= 3 ? 'bad' : 'warn'));
    } else {
      chips.push(chip('Modo leitura', 'pink', 'globe'));
    }
    $('#devcard').innerHTML = `
      <div class="devcard__row">
        <div class="devcard__icon">${ico(icon)}</div>
        <div><div class="devcard__title">${esc(title)}</div><div class="devcard__sub">${esc(sub)}</div></div>
      </div>
      <div class="devcard__chips">${chips.join('')}</div>`;

    const items = NAV.map((n) => {
      const ok = routeAllowed(n.id);
      if (n.id === 'onboarding' && p === 'ready') return '';
      return navBtn(n.id, n.label, n.icon, ok);
    }).join('');
    const extra = p === 'locked' ? navBtn('unlock', 'Desbloquear', 'lock', true) : '';
    $('#nav').innerHTML = `${extra}${items}<div class="nav__label">Público</div>${navBtn('results', 'Resultados ao vivo', 'chart', true)}${navBtn('explorer', 'Explorador público', 'globe', true)}`;
    $('#addr').textContent = d.addr ? `emulador ${d.addr}` : '';
    document.querySelectorAll('[data-theme-set]').forEach((b) => {
      const v = b.dataset.themeSet;
      b.innerHTML = ico(v === 'light' ? 'sun' : v === 'dark' ? 'moon' : 'monitor');
      b.setAttribute('aria-pressed', String(themePref() === v));
    });
  }

  function navBtn(id, label, icon, ok) {
    return `<button class="nav__item" data-act="nav" data-id="${id}" ${S.route === id ? 'aria-current="page"' : ''} ${ok ? '' : 'aria-disabled="true"'}>
      ${ico(icon)}<span>${esc(label)}</span>${ok ? '' : `<span class="nav__lock">${ico('lock')}</span>`}</button>`;
  }

  function renderMain() {
    const m = $('#main');
    const views = { home: vHome, results: vResults, explorer: vExplorer, onboarding: vOnboarding, unlock: vUnlock, identity: vIdentity, wallet: vWallet, assets: vAssets, credentials: vCreds, stake: vStake, sign: vSign, prove: vProve, vote: vVote };
    m.innerHTML = `<div class="view">${(views[S.route] || vExplorer)()}</div>`;
    afterRender();
  }

  function head(eyebrow, title, desc) {
    return `<header class="pagehead"><div class="eyebrow">${esc(eyebrow)}</div><h1>${esc(title)}</h1>${desc ? `<p>${esc(desc)}</p>` : ''}</header>`;
  }
  const callout = (kind, icon, title, body) => `<div class="callout callout--${kind}">${ico(icon)}<div>${title ? `<strong>${esc(title)}</strong>` : ''}${body}</div></div>`;

  // ───────────────────────── explorador (sem dispositivo) ─────────────────────────
  // Progresso da verificação do quadro (a Suite verifica as entradas em segundo plano).
  function verifyBar(done, total) {
    const pct = total ? Math.floor((done * 100) / total) : 0;
    return `<article class="card card--flat" role="status" aria-live="polite"><div class="row">${ico('spin', 'spin')}<strong>Verificando o quadro</strong>
      <span class="muted">${nf.format(done)} de ${nf.format(total)} entradas (${pct}%): os números abaixo são os do trecho já verificado.</span></div>
      <progress class="bar" max="${total}" value="${done}" aria-label="Verificação do quadro"></progress></article>`;
  }

  function explorerBody() {
    const x = S.explorer;
    if (!x) return `<article class="card"><p class="muted">${S.explorerTried ? 'Carregando o quadro de avisos…' : ''}</p></article>`;
    if (!x.available) return callout('info', 'link', 'Quadro ainda vazio', `<p>${esc(x.reason || 'Publique a eleição no quadro (make publish).')}</p>`);
    const verifying = x.verified < x.total ? verifyBar(x.verified, x.total) : '';
    const head = `${verifying}<article class="card">
      <div class="card__head"><div class="card__ico card__ico--arara">${ico('globe')}</div><div><h2>${esc(x.title)}</h2>
        <p class="muted">Eleição <span class="mono">${esc(x.election_id)}</span> · revisão ${x.revision}</p></div></div>
      <p class="muted">Quadro de avisos: <strong>${nf.format(x.head_size)}</strong> entradas · raiz Merkle <span class="mono">${esc(short(x.head_root, 12, 10))}</span></p>
      <div class="row"><label class="muted" for="incl-idx">Conferir a posição nº</label>
        <input class="input" id="incl-idx" type="number" min="0" max="${Math.max(0, x.head_size - 1)}" style="max-width:140px" value="${S.inclusion ? S.inclusion.index : 0}">
        <button class="btn" data-act="checkincl">${ico('check')}Conferir inclusão</button></div>
      ${S.inclusion ? callout(S.inclusion.valid ? 'ok' : 'bad', S.inclusion.valid ? 'check' : 'alert', S.inclusion.valid ? 'Está no quadro' : 'NÃO confere',
        `<p>A entrada nº ${S.inclusion.index} (${esc(S.inclusion.kind)}) ${S.inclusion.valid ? 'pertence ao log' : 'não bate com a raiz atual'}: prova Merkle de ${S.inclusion.proof_len} passos recalculada aqui contra a raiz de ${nf.format(S.inclusion.size)} entradas.</p>`) : ''}
    </article>`;
    const contests = x.contests.map((c) => {
      const [pl, pk] = PHASES[c.phase] || [c.phase, ''];
      return `<article class="card">
        <div class="card__head"><div class="card__ico card__ico--${c.scope === 'national' ? 'arara' : c.scope === 'state' ? 'onca' : 'boto'}">${ico(c.scope === 'national' ? 'globe' : 'ballot')}</div>
          <div><h3>${esc(c.title)}</h3><div class="row" style="gap:6px;margin-top:6px">${chip(scopeLabel(c), 'info')}${chip(pl, pk)}${c.registration_required ? chip('Registro anônimo', 'info', 'shield') : ''}</div></div></div>
        <p class="muted">Registro: ${esc(fmtDate(c.registration_opens))} · Votação: ${esc(fmtDate(c.voting_opens))} até ${esc(fmtDate(c.voting_closes))}</p>
        <ol class="muted" style="margin:0;padding-left:1.2em">${c.options.map((o) => `<li><span class="mono">${esc(o.number)}</span> ${esc(o.name)}</li>`).join('')}</ol>
      </article>`;
    }).join('');
    const gov = x.governance.length ? `<article class="card">
      <div class="card__head"><div class="card__ico card__ico--onca">${ico('shield')}</div><h3>Decisões da governança (em ordem do log)</h3></div>
      <p class="muted">Cada decisão exige a assinatura de instituições independentes (M de N) e fica no quadro, a prova de que ninguém abriu ou fechou um pleito sozinho.</p>
      <ol class="muted" style="margin:0;padding-left:1.2em">${x.governance.map((g) => `<li>nº ${g.seq} · <strong>${esc(g.contest_id)}</strong>: ${esc((PHASES[g.from] || [g.from])[0])} → ${esc((PHASES[g.to] || [g.to])[0])} · ${esc(fmtDate(g.at))} · ${g.signatures} assinaturas · posição #${g.board_index}</li>`).join('')}</ol>
    </article>` : '';
    const cer = x.ceremony ? `<article class="card">
      <div class="card__head"><div class="card__ico card__ico--onca">${ico('shield')}</div><div><h3>Cerimônia da chave da eleição</h3>
        <p class="muted">${x.ceremony.validators} validadores geraram a chave juntos: <strong>${x.ceremony.threshold} de ${x.ceremony.validators}</strong> precisam se juntar para decifrar os totais. Ninguém guarda a chave inteira. Verificado aqui (provas de posse e chave recalculadas).</p></div></div>
      <p class="muted">Chave pública: <span class="mono">${esc(short(x.ceremony.election_key, 14, 10))}</span> ${x.ceremony.matches_manifest ? chip('igual à do manifesto assinado', 'ok', 'check') : chip('DIFERENTE da do manifesto', 'bad', 'alert')}</p>
    </article>` : '';
    return `${head}${networkCard()}${cer}<div class="grid grid--3">${contests}</div>${gov}`;
  }

  function networkCard() {
    const n = S.net;
    if (!n) return '';
    if (n.mode !== 'net') return `<article class="card"><div class="card__head"><div class="card__ico">${ico('link')}</div><div><h3>Quadro: arquivo local</h3><p class="muted">Um único operador, sem rede. Rode <code>make devnet</code> para ver a blockchain de teste com 3 validadores.</p></div></div></article>`;
    const nodes = n.nodes.map((x) => `<li class="node ${x.up ? 'node--up' : 'node--down'}" data-tip="Validador ${x.index}|${x.up ? `altura ${x.height} · mempool ${x.mempool} · ${x.peers_online} pares${x.equivocations ? ` · ${x.equivocations} equívoco(s)!` : ''}` : 'fora do ar'}"><span class="node__dot"></span>V${x.index}<small>${x.up ? `#${x.height}` : 'off'}</small></li>`).join('');
    const blocks = n.blocks.map((b) => `<li class="blk" data-tip="Bloco ${b.height}|${esc(short(b.hash, 10, 8))} · proposto por V${b.proposer}">
        <span class="blk__h">#${b.height}</span><span>${b.entries} entradas</span>
        <span class="blk__sig">${'●'.repeat(Math.min(b.signatures, n.validators))}${'○'.repeat(Math.max(0, n.validators - b.signatures))} ${b.signatures}/${n.validators}</span>
        <span class="muted">V${b.proposer}${b.round ? ' · troca de turno' : ''} · ${esc(fmtDate(b.timestamp))}</span></li>`).join('');
    return `<article class="card">
      <div class="card__head"><div class="card__ico card__ico--arara">${ico('link')}</div><div><h3>Rede ${esc(n.name)} <span class="muted mono">${esc(n.chain_id)}</span></h3>
        <p class="muted">${n.threshold} de ${n.validators} validadores assinam cada bloco · verificado aqui, bloco a bloco</p></div></div>
      <ul class="nodes">${nodes}</ul>
      ${n.error ? callout('bad', 'alert', 'Réplica não verificada', `<p>${esc(n.error)}</p>`) : ''}
      <ol class="blocks" reversed>${blocks || '<li class="muted">Nenhum bloco ainda: publique a eleição.</li>'}</ol>
    </article>`;
  }

  async function loadExplorer() {
    // UMA tentativa por visita (mesma regra dos outros carregamentos: sem laço de render).
    if (S.explorerTried) return;
    S.explorerTried = true;
    try { S.net = await invoke('network_status'); } catch { S.net = null; }
    try { S.explorer = await invoke('explorer_snapshot'); } catch { S.explorer = { available: false, reason: 'Não foi possível ler o quadro.' }; }
    S.lastKey = ''; render();
    // enquanto a Suite ainda verifica o quadro, pede de novo logo em seguida (sem laço de render)
    if (S.route === 'explorer' && S.explorer && S.explorer.available && S.explorer.verified < S.explorer.total) {
      S.explorerTried = false; setTimeout(loadExplorer, 150);
    }
  }

  function vExplorer() {
    const connected = phase() !== 'none';
    return `
      <section class="hero">
        <div class="eyebrow" style="color:#F5C22A">${connected ? 'Dispositivo conectado' : 'Modo somente leitura'}</div>
        <h1>${connected ? 'Explorador público' : 'Acompanhe a democracia, sem precisar se identificar'}</h1>
        <p>${connected
          ? 'Você está vendo a parte pública. Suas ações de identidade ficam no menu ao lado.'
          : 'Veja eleições, candidatos e resultados apurados. Para assinar, provar quem você é ou votar, conecte seu dispositivo Teko.'}</p>
        ${connected ? '' : `<div class="row"><button class="btn btn--gold" data-act="rescan">${ico('usb')}Procurar dispositivo</button></div>`}
      </section>

      ${explorerBody()}

      ${connected ? '' : `
      <article class="card">
        <div class="card__head"><div class="card__ico card__ico--boto">${ico('usb')}</div><h3>Como conectar o dispositivo (emulador)</h3></div>
        <p class="muted">Em um terminal, na pasta do projeto:</p>
        <div class="codeblock"><span class="dim"># 1) uma vez: cria as chaves de teste, firmware e fábrica</span><br>cargo run -p issuer-cli -- dev-setup<br><span class="dim"># 2) liga o dispositivo (tela e botões ficam NESTE terminal)</span><br>cargo run -p device-emulator</div>
        ${S.dev.trust_ok ? '' : callout('warn', 'alert', 'Falta preparar o ambiente de teste', '<p>Não encontrei <code>dev/trust.txt</code>. Rode o passo 1 acima.</p>')}
      </article>`}`;
  }

  // ───────────────────────── onboarding ─────────────────────────
  const STEPS = [['detect', 'Dispositivo'], ['auth', 'Autenticidade'], ['firmware', 'Firmware'], ['pin', 'PIN'], ['wallet', 'Carteira'], ['id', 'ID Cívico'], ['issue', 'Credencial'], ['done', 'Pronto']];

  function onboardStep() {
    const st = S.dev.status;
    if (!st) return 'detect';
    switch (st.state) {
      case 'virgin': return S.authAck ? 'firmware' : 'auth';
      case 'firmware_installed': return 'pin';
      case 'initialized': return !st.wallet_ready ? 'wallet' : S.civic ? 'issue' : 'id';
      default: return 'done';
    }
  }

  function vOnboarding() {
    const cur = onboardStep();
    const idx = STEPS.findIndex(([k]) => k === cur);
    const stepper = STEPS.map(([k, label], i) => `
      <li class="step ${i < idx ? 'step--done' : i === idx ? 'step--now' : ''}" ${i === idx ? 'aria-current="step"' : ''}>
        <span class="step__n">${i < idx ? ico('check') : i + 1}</span>${esc(label)}</li>`).join('');
    const body = { detect: sDetect, auth: sAuth, firmware: sFirmware, pin: sPin, wallet: sWallet, id: sId, issue: sIssue, done: sDone }[cur]();
    const wipedBanner = S.wiped
      ? `<div class="callout callout--bad" role="alert">${ico('alert')}<div><strong>Seu dispositivo foi apagado</strong>
          <p>Houve excesso de tentativas de PIN (ou o dispositivo foi apagado). Chaves, ID Cívico e credencial foram <strong>destruídos</strong>. A <strong>identidade</strong> não tem seed: é preciso configurar de novo e refazer a emissão da credencial (na vida real, presencialmente na autoridade). Já a <strong>carteira</strong> se recupera com a sua frase de 24 palavras (etapa Carteira → Restaurar).</p>
          <div class="row" style="margin-top:8px"><button class="btn btn--ghost btn--sm" data-act="dismisswipe">Entendi</button></div></div></div>`
      : '';
    return `${wipedBanner}${head('Primeira utilização', 'Vamos configurar seu dispositivo', 'Cada etapa sensível é confirmada na tela e nos botões do dispositivo, nunca neste computador.')}
      <ol class="stepper" aria-label="Etapas">${stepper}</ol>${body}`;
  }

  const sDetect = () => `<article class="card"><p>Procurando o dispositivo…</p></article>`;

  function sAuth() {
    const g = S.genuine;
    let inner;
    if (S.authRunning) {
      inner = `<div class="row">${ico('spin', 'spin')}<span>Enviando um desafio aleatório ao dispositivo…</span></div>`;
    } else if (!g) {
      inner = `<button class="btn" data-act="auth">${ico('shield')}Verificar autenticidade</button>`;
    } else if (g.genuine) {
      inner = `${callout('ok', 'shield', 'Dispositivo genuíno', `<p>A cadeia de certificados leva até a autoridade raiz. Lote <code>${esc(g.batch_id)}</code>. Nenhum dado seu foi enviado.</p>`)}
        <div class="row"><button class="btn" data-act="authnext">Continuar${ico('check')}</button></div>`;
    } else {
      inner = `${callout('bad', 'alert', 'Falhou: não use este dispositivo', `<p>${esc(g.reason || 'A atestação não pôde ser verificada.')}</p>`)}
        <div class="row"><button class="btn btn--ghost" data-act="auth">Tentar de novo</button></div>`;
    }
    return `<article class="card">
      <div class="card__head"><div class="card__ico">${ico('shield')}</div><h2>É um dispositivo Teko de verdade?</h2></div>
      <p class="muted">A Suite faz um desafio-resposta: o dispositivo assina um número aleatório com uma chave de lote (compartilhada por muitos dispositivos, para ninguém ser rastreado) e a Suite confere até a raiz da autoridade.</p>
      ${inner}</article>`;
  }

  function sFirmware() {
    const fw = S.fw;
    const list = !fw
      ? (S.fwTried && !S.fwLoading
        ? `${callout('warn', 'alert', 'Não consegui ler os firmwares', '<p>Veja a mensagem acima. Se faltou o ambiente de teste, rode <code>make setup</code>.</p>')}<div class="row"><button class="btn btn--ghost" data-act="fwretry">Tentar de novo</button></div>`
        : `<div class="row">${ico('spin', 'spin')}<span>Lendo firmwares assinados…</span></div>`)
      : fw.length === 0
        ? callout('warn', 'alert', 'Nenhum firmware encontrado', '<p>Rode <code>cargo run -p issuer-cli -- dev-setup</code>.</p>')
        : `<div class="fw">${fw.map(fwItem).join('')}</div>`;
    return `<article class="card">
      <div class="card__head"><div class="card__ico card__ico--arara">${ico('key')}</div><h2>Firmware assinado por várias instituições</h2></div>
      <p class="muted">O dispositivo só roda firmware com assinaturas de pelo menos ${fw && fw[0] ? fw[0].threshold : 2} de ${fw && fw[0] ? fw[0].total_signers : 3} instituições independentes. A Suite confere antes; o dispositivo confere de novo e pede sua confirmação.</p>
      ${S.fwProgress ? `<div class="progress" role="progressbar" aria-valuenow="${Math.round(S.fwProgress * 100)}" aria-valuemin="0" aria-valuemax="100"><span style="width:${Math.round(S.fwProgress * 100)}%"></span></div>` : ''}
      ${list}</article>`;
  }

  function fwItem(f) {
    const fp = f.fingerprint.replace(/\s/g, '');
    return `<div class="fw__item">
      <div>
        <h3>Versão ${f.version} ${f.installed_version === f.version ? chip('instalada', 'ok') : ''}</h3>
        <p class="muted">${esc(f.verdict)} · assinado por ${f.valid_signers} de ${f.total_signers} · ${Math.round(f.size / 1024)} KiB</p>
        <div class="blocks mono" aria-label="Impressão digital">${groups(fp)}</div>
      </div>
      <button class="btn" data-act="fwinstall" data-v="${f.version}" data-fp="${esc(fp)}" ${f.installable ? '' : 'disabled'}>${ico('upload')}${S.dev.status.fw_version ? 'Atualizar' : 'Instalar'}</button>
    </div>`;
  }

  function sPin() {
    return `<article class="card">
      <div class="card__head"><div class="card__ico card__ico--onca">${ico('lock')}</div><h2>Defina seu PIN no dispositivo</h2></div>
      <p class="muted">O PIN é digitado <strong>no terminal do emulador</strong> (a "tela e botões" do dispositivo), duas vezes. Esta janela nunca o vê.</p>
      ${callout('warn', 'alert', 'Guarde bem o PIN', '<p>Se perder o PIN ou errar 8 vezes, o dispositivo se apaga. A identidade se refaz presencialmente na autoridade; a carteira se recupera com a frase de 24 palavras (próxima etapa). PINs fracos (123456, 111111) são recusados pelo dispositivo.</p>')}
      <div class="row"><button class="btn" data-act="pin">${ico('lock')}Definir PIN no dispositivo</button></div>
    </article>`;
  }

  function sWallet() {
    return `<article class="card">
      <div class="card__head"><div class="card__ico card__ico--boto">${ico('wallet')}</div><h2>Sua carteira e a frase de recuperação</h2></div>
      <p class="muted">O dispositivo cria a carteira e mostra <strong>24 palavras na tela dele</strong>. Elas <strong>nunca</strong> passam por este computador. Depois ele pede algumas palavras de posições sorteadas para ter certeza de que você anotou.</p>
      ${callout('warn', 'alert', 'Quem tem a frase tem o dinheiro', '<p>Anote em <strong>papel</strong>, na ordem, e guarde em lugar seguro. Não fotografe, não digite em computador nem celular, não envie a ninguém. Se perder o dispositivo, é a única forma de recuperar os fundos. A <strong>identidade</strong> continua sem seed (reemissão presencial); a frase serve só para a carteira.</p>')}
      <div class="row">
        <button class="btn" data-act="walletnew">${ico('key')}Criar nova carteira</button>
        <button class="btn btn--ghost" data-act="walletrestore">${ico('upload')}Restaurar com minha frase</button>
      </div>
      <p class="muted" style="margin-top:10px">Opcional: o dispositivo perguntará se você quer uma <em>passphrase</em> (senha extra). Se esquecê-la, os fundos se perdem.</p>
    </article>`;
  }

  function sId() {
    return `<article class="card">
      <div class="card__head"><div class="card__ico">${ico('id')}</div><h2>Suas chaves nasceram dentro do dispositivo</h2></div>
      <p class="muted">Nenhuma chave privada sai dele. Vamos mostrar seu ID Cívico: confira se é idêntico ao exibido na tela do dispositivo.</p>
      <div class="row"><button class="btn" data-act="showid">${ico('eye')}Mostrar meu ID Cívico</button></div>
    </article>`;
  }

  function sIssue() {
    const r = S.civicReq;
    return `<article class="card">
      <div class="card__head"><div class="card__ico">${ico('id')}</div><h2>Seu ID Cívico</h2></div>
      <div class="screen"><div class="screen__label">Confira com o dispositivo</div><div class="id-big">${esc(S.civic.id)}</div></div>
      <p class="muted">Não é RG nem CPF: é derivado da sua chave pública e não contém dados pessoais.</p>
    </article>
    <article class="card">
      <div class="card__head"><div class="card__ico card__ico--boto">${ico('badge')}</div><h2>Credencial cívica</h2></div>
      ${callout('info', 'shield', 'Emitida por um órgão, na chain', '<p>A credencial é emitida por um <strong>órgão autorizado</strong> (quórum de membros, cada um com o próprio dispositivo), fica <strong>registrada na chain</strong> e passa por uma <strong>janela de contestação</strong> antes de valer. A Suite não emite nada: ela só pede e instala.</p>')}
      <ol class="muted">
        <li><strong>Peça</strong> a credencial: o dispositivo prova que é genuíno e que tem a sua chave.</li>
        <li>Leve o pedido ao órgão, no app <strong>Teko Emissor</strong> (<code>make issuer</code>, aba <em>ID cívico</em>).</li>
        <li>Depois da emissão e da aprovação do quórum, <strong>instale</strong> o pacote entregue pelo órgão.</li>
      </ol>
      <div class="row">
        <button class="btn" data-act="civicrequest">${ico('upload')}Pedir a credencial ao órgão</button>
        <button class="btn btn--ghost" data-act="civicimport">${ico('badge')}Instalar o pacote do órgão</button>
      </div>
      ${r ? `<div class="callout callout--ok">${ico('check')}<div><strong>Pedido gerado.</strong> Entregue ao órgão o arquivo <code>${esc(r.path)}</code>. O Emissor o lê dali.</div></div>` : ''}
      <p class="muted">Se você perdeu ou trocou de dispositivo: a identidade não tem frase de recuperação (a carteira tem). O órgão confere você <strong>presencialmente</strong>, <strong>revoga</strong> a credencial anterior na chain e emite uma nova com o mesmo registro. Quem estiver com o dispositivo antigo não consegue mais provar nada.</p>
    </article>`;
  }

  function sDone() {
    const st = S.dev.status;
    return `<article class="card">
      <div class="card__head"><div class="card__ico">${ico('check')}</div><h2>Tudo pronto</h2></div>
      <dl class="kv">
        <dt>Autenticidade</dt><dd>${S.genuine && S.genuine.genuine ? 'Dispositivo genuíno' : 'não verificada nesta sessão'}</dd>
        <dt>Firmware</dt><dd>v${st.fw_version}</dd>
        <dt>Credencial</dt><dd>Emitida pelo órgão, na chain, e vinculada à sua chave</dd>
      </dl>
      <div class="row"><button class="btn" data-act="gohome">${ico('id')}Ir para minha identidade</button></div>
    </article>`;
  }

  // ───────────────────────── desbloqueio ─────────────────────────
  function vUnlock() {
    const st = S.dev.status;
    const r = st.retries_remaining;
    const last = r <= 1;
    let warn = '';
    if (last) {
      warn = callout('bad', 'alert', 'ÚLTIMA TENTATIVA', `<p>Se o PIN estiver errado, o dispositivo <strong>apaga chaves e credencial</strong> e será preciso refazer a emissão. Não tem certeza do PIN? No emulador, deixe o PIN vazio e aperte Enter: isso <strong>cancela sem gastar a tentativa</strong>.</p>`);
    } else if (r <= 3) {
      warn = callout('warn', 'alert', `Atenção: restam ${r} tentativas`, `<p>Ao zerar, o dispositivo se apaga. Para desistir sem gastar tentativa, deixe o PIN vazio no emulador e aperte Enter.</p>`);
    }
    const blocked = last && !S.lastAck;
    return `${head('Dispositivo bloqueado', 'Desbloqueie no dispositivo', 'Ao conectar, o dispositivo sempre começa bloqueado.')}
      <article class="card">
        <div class="card__head"><div class="card__ico card__ico--onca">${ico('lock')}</div><h2>Digite o PIN no terminal do emulador</h2></div>
        <p class="muted">A Suite só pede o desbloqueio; quem lê o PIN é o dispositivo. Tentativas restantes: <strong>${r}</strong> de 8.</p>
        ${warn}
        ${S.lockMsg ? callout('bad', 'alert', '', `<p>${esc(S.lockMsg)}</p>`) : ''}
        ${last ? `<label class="row"><input type="checkbox" data-act="lastack" ${S.lastAck ? 'checked' : ''}> <span>Entendo que, se errar, o dispositivo será apagado.</span></label>` : ''}
        <div class="row"><button class="btn" data-act="unlock" ${blocked ? 'disabled' : ''}>${ico('unlock')}Desbloquear</button></div>
      </article>`;
  }

  // ───────────────────────── identidade ─────────────────────────
  function vWallet() {
    const a = S.walletAddr;
    const b = S.balance;
    const bal = !b ? `<div class="row">${ico('spin', 'spin')}<span>Lendo o saldo…</span></div>`
      : b.available
        ? `<div class="kpi__val">${esc(money(b.amount))} <span class="muted" style="font-size:1rem;font-weight:600">TEKO</span></div><div class="row">${chip('moeda de TESTE', 'warn')}</div><p class="muted">${esc(b.note || '')}</p>`
        : `<p class="muted">${esc(b.reason || 'Saldo indisponível.')}</p>`;
    return `${head('Carteira', 'Seu endereço e seu saldo', 'A chave desta carteira nasceu no dispositivo e nunca sai dele. Só o endereço, que é público, aparece aqui.')}
      <article class="card">
        <div class="card__head"><div class="card__ico card__ico--boto">${ico('wallet')}</div><h2>Endereço</h2></div>
        ${a ? `<div class="screen"><div class="screen__label">Endereço da carteira (público)</div><div class="id-big">${esc(a.address)}</div></div>
          <div class="row"><button class="btn btn--ghost btn--sm" data-act="copy" data-text="${esc(a.address)}">${ico('copy')}Copiar</button></div>`
        : `<div class="row">${ico('spin', 'spin')}<span>Lendo do dispositivo…</span></div>`}
      </article>
      <article class="card">
        <div class="card__head"><div class="card__ico card__ico--arara">${ico('chart')}</div><h2>Saldo</h2></div>
        ${bal}
      </article>
      <article class="card">
        <div class="card__head"><div class="card__ico card__ico--onca">${ico('upload')}</div><h2>Enviar moedas</h2></div>
        <p class="muted">O dispositivo mostra o destino, o valor e a taxa e só assina depois da sua confirmação no botão dele. Moedas de teste, na devnet local.</p>
        <div class="fields">
          <div class="field"><label for="f-to">Endereço de destino</label><input class="input" id="f-to" placeholder="tekow1…" autocomplete="off" spellcheck="false"></div>
          <div class="field"><label for="f-amt">Valor (TEKO)</label><input class="input" id="f-amt" type="text" inputmode="decimal" autocomplete="off" placeholder="ex.: 12,5" aria-describedby="f-amt-h"><div class="muted" id="f-amt-h" style="font-size:.8rem">${AMOUNT_HELP}</div></div>
        </div>
        <p class="muted">A taxa é calculada por byte e paga a quem produz o bloco; você vê o valor exato no dispositivo.</p>
        <div class="row"><button class="btn" data-act="walletsend">${ico('upload')}Enviar com o dispositivo</button></div>
      </article>
      <article class="card">
        <div class="card__head"><div class="card__ico card__ico--arara">${ico('list')}</div><h2>Histórico</h2></div>
        ${!S.history ? `<div class="row">${ico('spin', 'spin')}<span>Lendo…</span></div>`
          : S.history.length === 0 ? '<p class="muted">Nenhuma movimentação ainda.</p>'
          : `<dl class="kv">${S.history.map((h) => `<dt>${h.direction === 'sent' ? 'Enviado' : 'Recebido'} · bloco ${esc(String(h.height))}</dt><dd><strong>${h.direction === 'sent' ? '−' : '+'}${esc(money(h.amount))} TEKO</strong>${h.fee ? ` <span class="muted">(taxa ${esc(money(h.fee))})</span>` : ''} <span class="muted mono" style="font-size:.8rem">${h.direction === 'sent' ? 'para' : 'de'} ${esc(h.counterparty.slice(0, 14))}…${esc(h.counterparty.slice(-6))}</span></dd>`).join('')}</dl>`}
      </article>
      <article class="card">
        <div class="card__head"><div class="card__ico">${ico('shield')}</div><h2>Backup da frase</h2></div>
        <p class="muted">Quer ter certeza de que a sua anotação está correta? O dispositivo pede a frase inteira e só responde se confere ou não. Nada é exibido aqui.</p>
        <div class="row"><button class="btn" data-act="walletcheck">${ico('check')}Verificar meu backup</button></div>
        ${callout('warn', 'alert', 'Nunca digite as palavras neste computador', '<p>Esta janela não tem campo para a frase de propósito. Se algum site ou programa pedir as 24 palavras, é golpe.</p>')}
      </article>`;
  }

  function vIdentity() {
    const id = S.civic;
    const data = S.identity;
    const labels = { name: 'Nome', birth_date: 'Nascimento', nationality: 'Nacionalidade', registry_id: 'Registro Teko' };
    return `${head('Minha identidade', 'Quem sou eu, sob meu controle', 'Seus dados ficam no dispositivo. Só saem assinaturas e provas.')}
      <article class="card">
        <div class="card__head"><div class="card__ico">${ico('id')}</div><h2>ID Cívico</h2></div>
        ${id ? `<div class="screen"><div class="screen__label">Identificador público</div><div class="id-big">${esc(id.id)}</div></div>
          <div class="row"><button class="btn btn--ghost btn--sm" data-act="copy" data-text="${esc(id.id)}">${ico('copy')}Copiar</button></div>`
        : `<div class="row">${ico('spin', 'spin')}<span>Lendo do dispositivo…</span></div>`}
      </article>

      <article class="card">
        <div class="card__head"><div class="card__ico card__ico--arara">${ico('eye')}</div><h2>Dados de identificação</h2></div>
        <p class="muted">Cada campo é revelado com uma confirmação no dispositivo (4 confirmações).</p>
        ${data ? `<dl class="kv">${data.map(([k, v]) => `<dt>${esc(labels[k] || k)}</dt><dd>${esc(v)}</dd>`).join('')}</dl>
          <div class="row"><button class="btn btn--ghost btn--sm" data-act="hideid">Ocultar</button></div>`
        : `<div class="row"><button class="btn" data-act="reveal">${ico('eye')}Ver meus dados</button></div>`}
      </article>

      <article class="card">
        <div class="card__head"><div class="card__ico card__ico--boto">${ico('link')}</div><h2>ID por serviço</h2></div>
        <p class="muted">Cada serviço recebe um identificador diferente: dois serviços não conseguem saber que você é a mesma pessoa.</p>
        <div class="row"><div class="field" style="flex:1;min-width:240px"><label for="f-svc">Serviço</label><input class="input" id="f-svc" placeholder="ex.: banco.exemplo.com.br" autocomplete="off"></div>
          <button class="btn" data-act="pairwise" style="align-self:end">${ico('key')}Gerar ID</button></div>
        ${S.pairwise ? `<div class="screen"><div class="screen__label">${esc(S.pairwise.service)}</div><div class="id-big">${esc(S.pairwise.id)}</div></div>` : ''}
      </article>

      <article class="card card--flat">
        <div class="card__head"><div class="card__ico card__ico--guara">${ico('trash')}</div><h2>Apagar dispositivo</h2></div>
        <p class="muted">Destrói chaves e credenciais. Para voltar a usar, é preciso reemissão presencial. A confirmação final é feita no dispositivo.</p>
        <div class="row">${S.wipeArmed
          ? `<button class="btn btn--danger" data-act="wipe">${ico('trash')}Sim, apagar (confirmar no dispositivo)</button><button class="btn btn--ghost" data-act="wipecancel">Cancelar</button>`
          : `<button class="btn btn--ghost" data-act="wipearm">${ico('trash')}Apagar dispositivo…</button>`}</div>
      </article>`;
  }

  // ───────────────────────── assinar documento ─────────────────────────
  function vSign() {
    const f = S.file, r = S.signed, v = S.verifyRes;
    return `${head('Assinatura', 'Assine documentos com sua identidade', 'O arquivo vira um hash; você confere o hash na tela do dispositivo e confirma. A assinatura é verificável por qualquer pessoa.')}
      <article class="card">
        <div class="card__head"><div class="card__ico card__ico--onca">${ico('pen')}</div><h2>Documento</h2></div>
        <label class="dropzone" id="dz" for="f-file">${ico('upload')}<div><strong>Escolha ou solte um arquivo aqui</strong><br><span class="hint">Qualquer formato, até 20 MB</span></div><input id="f-file" type="file"></label>
        ${f ? `<div class="file-chosen">${ico('file')}<div><strong>${esc(f.name)}</strong><div class="muted">${Math.round(f.bytes.length / 1024 * 10) / 10} KiB</div></div></div>
          <div class="screen"><div class="screen__label">Hash SHA3-256 do arquivo</div><div class="blocks">${groups(f.hash)}</div></div>
          <div class="field"><label for="f-sum">Resumo (aparece no dispositivo como "informado pelo computador")</label><input class="input" id="f-sum" maxlength="120" value="${esc(f.summary)}"></div>
          <div class="row"><button class="btn" data-act="sign">${ico('pen')}Assinar no dispositivo</button></div>` : ''}
      </article>
      ${r ? `<article class="card">
        <div class="card__head"><div class="card__ico">${ico('check')}</div><h2>Assinatura gerada e verificada</h2></div>
        ${callout('ok', 'shield', 'A Suite conferiu a assinatura', '<p>Assinatura válida, ID Cívico derivado da chave pública e hash igual ao do arquivo.</p>')}
        <dl class="kv"><dt>Arquivo</dt><dd>${esc(r.fileName)}</dd><dt>Assinante</dt><dd class="mono">${esc(short(r.doc.civic_id, 14, 8))}</dd><dt>Hash</dt><dd class="mono">${esc(short(r.doc.hash_sha3_256, 12, 8))}</dd><dt>Assinatura</dt><dd class="mono">${esc(short(r.doc.signature, 12, 8))}</dd></dl>
        <div class="row"><button class="btn" data-act="savesig">${ico('file')}Salvar assinatura (.json)</button><button class="btn btn--ghost" data-act="copysig">${ico('copy')}Copiar JSON</button></div>
      </article>` : ''}
      <article class="card">
        <div class="card__head"><div class="card__ico card__ico--arara">${ico('shield')}</div><h2>Verificar uma assinatura</h2></div>
        <p class="muted">Confira a assinatura de qualquer pessoa, sem dispositivo: escolha o arquivo <code>.teko-signature.json</code> e, se quiser, o documento original.</p>
        <div class="fields">
          <div class="field"><label for="v-bundle">Arquivo de assinatura (.json)</label><input class="input" id="v-bundle" type="file" accept=".json,application/json"></div>
          <div class="field"><label for="v-file">Documento original (opcional)</label><input class="input" id="v-file" type="file"></div>
        </div>
        <div class="row"><button class="btn btn--ghost" data-act="verify">${ico('shield')}Verificar</button></div>
        ${v ? (v.ok
          ? callout('ok', 'check', 'Assinatura válida', `<p>Assinada por <span class="mono">${esc(short(v.res.civic_id, 14, 8))}</span>.${v.res.matches_file ? ' O arquivo confere com o hash assinado.' : ' (Nenhum arquivo foi comparado.)'}</p>`)
          : callout('bad', 'alert', 'Não confere', `<p>${esc(v.error)}</p>`)) : ''}
      </article>`;
  }

  // ───────────────────────── provar atributo ─────────────────────────
  const CHOICES = {
    age18: ['Tenho 18 anos ou mais', { kind: 'age_at_least', arg: 18, name: '' }, 'idade'],
    age21: ['Tenho 21 anos ou mais', { kind: 'age_at_least', arg: 21, name: '' }, 'idade'],
    age16: ['Tenho 16 anos ou mais', { kind: 'age_at_least', arg: 16, name: '' }, 'idade'],
    age65: ['Tenho 65 anos ou mais', { kind: 'age_at_least', arg: 65, name: '' }, 'idade'],
    nat: ['Mostrar minha nacionalidade', { kind: 'reveal', arg: 0, name: 'nationality' }, 'nacionalidade'],
  };

  function vProve() {
    const p = S.prove;
    const names = { age_over_16: '16+ anos', age_over_18: '18+ anos', age_over_21: '21+ anos', age_over_65: '65+ anos', nationality: 'Nacionalidade' };
    return `${head('Prova de atributo', 'Prove um fato sem entregar seus dados', 'Divulgação seletiva: o verificador recebe só o que você autorizar, assinado pela autoridade e por você.')}
      <article class="card">
        <div class="card__head"><div class="card__ico card__ico--arara">${ico('badge')}</div><h2>O que você quer provar?</h2></div>
        <div class="options" role="radiogroup" aria-label="Atributo">${Object.entries(CHOICES).map(([k, [label]]) => `
          <label class="option"><input type="radio" name="choice" value="${k}" ${p.choice === k ? 'checked' : ''}><span>${esc(label)}</span></label>`).join('')}</div>
        <div class="field"><label for="p-ver">Quem está pedindo (verificador)</label><input class="input" id="p-ver" value="${esc(p.verifier)}" maxlength="60"></div>
        <div class="row"><button class="btn" data-act="prove">${ico('badge')}Gerar prova no dispositivo</button></div>
      </article>
      ${p.error ? callout('bad', 'alert', 'Não foi possível provar', `<p>${esc(p.error)}</p>`) : ''}
      ${p.result ? `<article class="card">
        <div class="card__head"><div class="card__ico">${ico('check')}</div><h2>Prova verificada pela Suite</h2></div>
        <dl class="kv"><dt>Emitida por</dt><dd>${esc(p.result.issuer)}</dd>
          ${p.result.disclosed.map(([k, v]) => `<dt>Revelado</dt><dd>${esc(names[k] || k)}: <strong>${esc(v === 'true' ? 'sim' : v)}</strong></dd>`).join('')}</dl>
        ${CHOICES[p.choice][2] === 'idade' ? callout('ok', 'shield', 'Sua data de nascimento NÃO foi revelada', '<p>O verificador só soube que a condição é verdadeira.</p>') : ''}
      </article>` : ''}`;
  }

  // ───────────────────────── votar (protótipo): vários pleitos ─────────────────────────
  const PHASES = {
    draft: ['Em preparação', ''], candidates_published: ['Candidatos publicados', 'info'], registration_open: ['Registro aberto', 'info'],
    voting_open: ['Votação aberta', 'ok'], voting_closed: ['Votação encerrada', 'warn'], tallied: ['Apurada', ''], audited: ['Auditada', ''], cancelled: ['Cancelada', 'bad'],
  };
  const fmtDate = (secs) => (secs ? new Date(secs * 1000).toLocaleString('pt-BR', { dateStyle: 'short', timeStyle: 'short' }) : '—');
  const scopeLabel = (c) => (c.scope === 'national' ? 'Nacional' : c.scope === 'state' ? `Estado ${c.scope_code}` : `Município ${c.scope_code}`);

  function ballotCheckView(r) {
    const rec = r.recorded
      ? `<li>${chip('Conferido pelo dispositivo', 'ok', 'check')} Sua cédula está no <strong>bloco #${r.recorded.height}</strong>, certificado por <strong>${r.recorded.signers} de ${r.recorded.institutions}</strong> instituições. O dispositivo conferiu sozinho o certificado e a prova de inclusão. Rastreador <span class="mono">${esc(r.recorded.tracker)}</span>.</li>`
      : `<li>${chip('Ainda não conferido', 'warn', 'alert')} ${esc(r.recorded_note || '')}</li>`;
    const cnt = r.counted
      ? `<li>${chip(r.counted_state === 'counted' ? 'Contada' : 'Recusada pela apuração', r.counted_state === 'counted' ? 'ok' : 'bad', r.counted_state === 'counted' ? 'check' : 'alert')} ${esc(r.counted)}</li>`
      : `<li>${chip('Sem veredito', 'warn')} A apuração ainda não viu a sua cédula.</li>`;
    return `<ul class="muted" style="margin:8px 0 0;padding-left:1.2em">${rec}${cnt}</ul>`;
  }

  function contestCard(c) {
    const [phaseLabel, phaseKind] = PHASES[c.phase] || [c.phase, ''];
    const canVote = c.phase === 'voting_open' && c.eligible && !c.voted && Date.now() / 1000 < c.voting_closes;
    const ballot = S.ballots[c.contest_id];
    const why = !c.eligible ? 'Fora do seu domicílio eleitoral: o dispositivo recusa.'
      : c.voted ? 'Este dispositivo já votou neste pleito.'
      : c.phase !== 'voting_open' ? `Ainda não dá para votar: ${phaseLabel.toLowerCase()}.` : '';
    const checks = ((S.candChecks || []).find((x) => x.contest_id === c.contest_id) || { options: [] }).options;
    const badge = (i) => {
      const st = (checks.find((o) => o.index === i) || {}).state;
      return { verified: chip('candidatura conferida na chain', 'ok', 'check'), pending: chip('candidatura em contestação', 'warn'), expired: chip('candidatura expirada', 'bad'), revoked: chip('candidatura REVOGADA', 'bad', 'alert'), mismatch: chip('não confere com a chain', 'bad', 'alert'), unknown: chip('candidatura não está na chain', 'warn') }[st] || '';
    };
    const kinds = c.option_kinds || [];
    const optRow = (n, i) => `
      <label class="option${kinds[i] ? ' option--special' : ''}"><input type="radio" name="cand-${esc(c.contest_id)}" value="${i}" ${S.picks[c.contest_id] === i ? 'checked' : ''} ${canVote ? '' : 'disabled'}>
        <span>${kinds[i] === 1 ? '<strong>Voto em branco</strong> <span class="muted">(não conta para nenhum candidato)</span>' : kinds[i] === 2 ? '<strong>Voto nulo</strong> <span class="muted">(não conta para nenhum candidato)</span>' : `${(c.option_numbers || [])[i] ? `<strong>${esc(c.option_numbers[i])}</strong> – ` : ''}${esc(n)} ${badge(i)}`}</span></label>`;
    const opts = c.option_names.map((n, i) => (kinds[i] ? '' : optRow(n, i))).join('')
      + (kinds.some((k) => k) ? '<div class="muted" style="margin:8px 0 2px">Ou:</div>' + c.option_names.map((n, i) => (kinds[i] ? optRow(n, i) : '')).join('') : '');
    return `<article class="card" ${c.eligible ? '' : 'style="opacity:.82"'}>
      <div class="card__head"><div class="card__ico card__ico--${c.scope === 'national' ? 'arara' : c.scope === 'state' ? 'onca' : 'boto'}">${ico(c.scope === 'national' ? 'globe' : 'ballot')}</div>
        <div><h2>${esc(c.title)}</h2>
        <div class="row" style="gap:6px;margin-top:6px">${chip(scopeLabel(c), 'info')}${chip(phaseLabel, phaseKind)}${c.eligible ? chip(c.scope === 'national' ? 'Todos votam' : 'Seu domicílio', 'ok', 'check') : chip('Fora do seu domicílio', 'bad', 'lock')}${c.voted ? chip('Você já votou', 'ok', 'badge') : ''}${c.registration_required ? chip('Registro anônimo', 'info', 'shield') : ''}</div></div></div>
      <p class="muted">Votação: ${esc(fmtDate(c.voting_opens))} até ${esc(fmtDate(c.voting_closes))}</p>
      <div class="options" role="radiogroup" aria-label="Candidatos: ${esc(c.title)}">${opts}</div>
      ${why ? `<p class="muted">${esc(why)}</p>` : ''}
      ${canVote && c.registration_required ? '<p class="muted">Este pleito exige registro anônimo: o dispositivo pedirá <strong>duas confirmações</strong> (registro e voto).</p>' : ''}
      ${canVote ? `<div class="row"><button class="btn btn--gold" data-act="vote" data-contest="${esc(c.contest_id)}">${ico('ballot')}Votar neste pleito</button></div>` : ''}
      ${canVote && ballot === undefined ? '<p class="muted">Em eleição cifrada, o dispositivo pergunta, na tela dele, se você quer <strong>votar</strong> ou <strong>auditar</strong> a cédula: auditar mostra ao computador, só desta cédula de teste, em quem ela votava, para ele conferir que o dispositivo cifrou o que você escolheu. A cédula auditada é descartada e você pode votar em seguida.</p>' : ''}
      ${S.audits[c.contest_id] ? callout('ok', 'check', 'Auditoria: o dispositivo cifrou o que você escolheu', `<p>A cédula de teste <span class="mono">${esc(short(S.audits[c.contest_id].ciphertext_hash, 10, 8))}</span> ${esc(S.audits[c.contest_id].summary)}. Ela foi <strong>descartada</strong>: não é o seu voto. Você pode auditar de novo ou votar de verdade.</p>`) : ''}
      ${ballot ? callout('info', 'shield', 'Seu voto foi registrado', `<p>Rastreador da sua cédula: <strong class="mono">${esc(ballot.tracker || '—')}</strong> (o mesmo aparece na tela do dispositivo). Ele só identifica a cédula; <strong>nada aqui mostra em quem você votou</strong>, de propósito: assim ninguém consegue cobrar ou comprar o seu voto.
        ${ballot.submitted ? `Enviada ao quadro (posição #${ballot.board_index}): <a href="#" data-act="gotoresults">veja nos resultados ao vivo</a>.` : esc(ballot.note || 'Não foi enviada ao quadro.')}
        ${ballot.encrypted ? 'O voto foi <strong>cifrado dentro do dispositivo</strong> e só os totais serão decifrados, depois do fechamento.' : 'Voto em claro (eleição sem cifra): protótipo.'}</p>
        <div class="row"><button class="btn" data-act="checkballot" data-contest="${esc(c.contest_id)}">${ico('shield')}Conferir minha cédula</button></div>
        ${S.checks[c.contest_id] ? ballotCheckView(S.checks[c.contest_id]) : ''}`) : ''}
    </article>`;
  }

  function vVote() {
    const e = S.election;
    return `${head('Votação', 'Vote em cada pleito do seu domicílio', 'O Presidente é de todos; governador e prefeito, só do seu estado e município. Quem decide se você pode votar é o dispositivo.')}
      ${callout('warn', 'alert', 'Protótipo', '<p>A cédula é assinada de verdade pelo dispositivo (com o candidato mostrado na tela dele), mas <strong>ainda não é enviada, não tem prova ZK e a primeira versão não resiste à compra de voto</strong>. Os nomes são exemplos.</p>')}
      ${!e ? `<article class="card">
        <div class="card__head"><div class="card__ico card__ico--onca">${ico('ballot')}</div><h2>Carregar a eleição no dispositivo</h2></div>
        <p class="muted">O dispositivo confere as assinaturas das instituições (2 de 3) e o histórico de cada pleito, e pede a sua confirmação. Cada pleito tem as suas próprias datas.</p>
        <div class="row"><button class="btn" data-act="loadelection" ${S.elecLoading ? 'disabled' : ''}>${ico('upload')}Carregar eleição</button></div>
      </article>`
      : `<article class="card card--flat">
        <div class="row"><div><h2>${esc(e.title)}</h2><p class="muted">Revisão ${e.revision} · ${e.contests.length} pleitos · histórico de governança #${e.seq}</p></div>
          <span class="spacer"></span><button class="btn btn--ghost btn--sm" data-act="loadelection">${ico('upload')}Recarregar</button></div>
      </article>
      ${e.contests.map(contestCard).join('')}`}`;
  }

  // ───────────────────────── resultados ao vivo (público) ─────────────────────────
  const OFFICES = { president: 'Presidente', governor: 'Governador', senator: 'Senador', federal_deputy: 'Dep. federal', state_deputy: 'Dep. estadual', mayor: 'Prefeito', city_councillor: 'Vereador', other: 'Outras consultas' };
  const OFFICE_ORDER = ['president', 'governor', 'senator', 'federal_deputy', 'state_deputy', 'mayor', 'city_councillor', 'other'];
  const VIS = { live: ['Parciais ao vivo', 'info'], turnout_only: ['Só comparecimento até o fechamento', 'warn'], after_close: ['Nada até o fechamento', 'warn'] };
  const nf = new Intl.NumberFormat('pt-BR');

  // ───────────────────────── visual: cores, dicas, indicadores, mapa ─────────────────────────
  // Cor FIXA por candidato (índice no manifesto): não troca quando o ranking muda.
  const CAND = ['#1F4BB8', '#C62F24', '#12663B', '#B8365F', '#9A5208', '#7A3FB8', '#0E8A8A', '#6B7A1F', '#D9822B', '#566A5E'];
  const candColor = (i) => CAND[((i % CAND.length) + CAND.length) % CAND.length];
  const pctTxt = (x) => `${x.toFixed(1).replace('.', ',')}%`;
  // Mapa-tile do Brasil: [coluna, linha]. Cada UF tem o mesmo tamanho (não distorce por área).
  const UF_TILES = { RR: [2, 0], AP: [4, 0], AM: [2, 1], PA: [4, 1], MA: [5, 1], CE: [6, 1], RN: [7, 1], AC: [1, 2], RO: [2, 2], MT: [3, 2], TO: [4, 2], PI: [5, 2], PE: [6, 2], PB: [7, 2],
    MS: [2, 3], GO: [3, 3], DF: [4, 3], BA: [5, 3], SE: [6, 3], AL: [7, 3], SP: [3, 4], MG: [4, 4], ES: [5, 4], PR: [3, 5], RJ: [4, 5], RS: [2, 6], SC: [3, 6] };

  function sparkline(vals, w = 120, h = 34) {
    if (vals.length < 2) return `<svg class="spark" viewBox="0 0 ${w} ${h}" aria-hidden="true"><line x1="0" y1="${h - 2}" x2="${w}" y2="${h - 2}" stroke="currentColor" opacity=".25"/></svg>`;
    const lo = Math.min(...vals), hi = Math.max(...vals), span = Math.max(1, hi - lo);
    const pts = vals.map((v, i) => `${((i / (vals.length - 1)) * (w - 4) + 2).toFixed(1)},${(h - 3 - ((v - lo) / span) * (h - 8)).toFixed(1)}`);
    return `<svg class="spark" viewBox="0 0 ${w} ${h}" aria-hidden="true"><polyline points="${pts.join(' ')}" fill="none" stroke="currentColor" stroke-width="2" stroke-linejoin="round" stroke-linecap="round"/><circle cx="${pts[pts.length - 1].split(',')[0]}" cy="${pts[pts.length - 1].split(',')[1]}" r="3" fill="currentColor"/></svg>`;
  }

  function donutSvg(c, size = 150) {
    const total = c.valid || 0;   // base: votos em candidatos (branco e nulo ficam de fora)
    const rows = c.top.filter((o) => o.votes > 0).map((o) => ({ key: String(o.index), name: o.name, votes: o.votes, color: candColor(o.index) }));
    if (c.others_votes > 0) rows.push({ key: 'others', name: `Outros (${c.others_count})`, votes: c.others_votes, color: '#8A978F' });
    if (!total || !rows.length) return `<svg viewBox="0 0 42 42" width="${size}" height="${size}" role="img" aria-label="Sem votos ainda"><circle cx="21" cy="21" r="15.9155" fill="none" stroke="var(--surface-2)" stroke-width="6"/></svg>`;
    let off = 25;   // começa às 12h
    const arcs = rows.map((r) => {
      const p = (r.votes * 100) / total;
      const a = `<circle class="slice" data-key="${esc(r.key)}" data-tip="${esc(r.name)}|${nf.format(r.votes)} votos · ${pctTxt(p)}" cx="21" cy="21" r="15.9155" fill="none" stroke="${r.color}" stroke-width="6" stroke-dasharray="${Math.max(0, p - 0.4).toFixed(2)} ${(100 - Math.max(0, p - 0.4)).toFixed(2)}" stroke-dashoffset="${off.toFixed(2)}"/>`;
      off -= p; return a;
    }).join('');
    const lead = rows[0], lp = (lead.votes * 100) / total;
    return `<svg viewBox="0 0 42 42" width="${size}" height="${size}" role="img" aria-label="Participação de cada candidato">${arcs}
      <text x="21" y="20.2" text-anchor="middle" class="donut-pct">${pctTxt(lp)}</text><text x="21" y="25.6" text-anchor="middle" class="donut-sub">lidera</text></svg>`;
  }

  // Dica flutuante única (mouse e teclado): qualquer elemento com data-tip="título|detalhe".
  function tipEl() { let t = $('#tip'); if (!t) { t = document.createElement('div'); t.id = 'tip'; t.setAttribute('role', 'tooltip'); t.hidden = true; document.body.append(t); } return t; }
  document.addEventListener('mouseover', (e) => {
    const el = e.target.closest && e.target.closest('[data-tip]');
    const card = e.target.closest && e.target.closest('.rcard');
    // ligação entre rosca e barras do MESMO pleito
    document.querySelectorAll('.rcard.has-hot').forEach((c) => { if (c !== card) { c.classList.remove('has-hot'); c.querySelectorAll('.is-hot').forEach((x) => x.classList.remove('is-hot')); } });
    if (card) {
      const k = e.target.closest('[data-key]');
      card.querySelectorAll('.is-hot').forEach((x) => x.classList.remove('is-hot'));
      if (k) { card.classList.add('has-hot'); card.querySelectorAll(`[data-key="${CSS.escape(k.dataset.key)}"]`).forEach((x) => x.classList.add('is-hot')); } else card.classList.remove('has-hot');
    }
    const t = tipEl();
    if (!el) { t.hidden = true; return; }
    const [a, b] = el.dataset.tip.split('|');
    t.innerHTML = `<strong>${esc(a)}</strong>${b ? `<span>${esc(b)}</span>` : ''}`; t.hidden = false;
  });
  document.addEventListener('mousemove', (e) => { const t = $('#tip'); if (t && !t.hidden) { t.style.left = `${Math.min(e.clientX + 14, innerWidth - t.offsetWidth - 8)}px`; t.style.top = `${Math.min(e.clientY + 16, innerHeight - t.offsetHeight - 8)}px`; } });
  document.addEventListener('mouseleave', () => { const t = $('#tip'); if (t) t.hidden = true; });

  function renderOverview(r) {
    const el = $('#res-overview'); if (!el) return;
    const cs = r.contests;
    const known = cs.filter((c) => c.accepted !== null);
    const total = known.reduce((a, c) => a + c.accepted, 0);
    const hidden = known.length === 0;
    const registered = cs.reduce((a, c) => a + (c.registration_required ? c.registered : 0), 0);
    const rejected = cs.reduce((a, c) => a + c.rejected, 0);
    const byPhase = { open: cs.filter((c) => c.phase === 'voting_open').length, closed: cs.filter((c) => ['voting_closed', 'tallied', 'audited'].includes(c.phase)).length };
    byPhase.other = cs.length - byPhase.open - byPhase.closed;
    // histórico para o minigráfico (só cresce; zera se o quadro for reiniciado)
    S.resultsHist = S.resultsHist || [];
    if (!hidden && (!S.resultsHist.length || S.resultsHist[S.resultsHist.length - 1] !== total)) { if (S.resultsHist.length && total < S.resultsHist[S.resultsHist.length - 1]) S.resultsHist = []; S.resultsHist.push(total); if (S.resultsHist.length > 40) S.resultsHist.shift(); }
    const part = registered > 0 && !hidden ? Math.min(100, (total * 100) / registered) : null;
    const seg = (n, color, label) => (cs.length && n ? `<span class="stack__seg" style="flex:${n};background:${color}" data-tip="${esc(label)}|${n} pleito(s)"></span>` : '');
    const kpis = `<div class="kpis">
      <div class="kpi"><div class="kpi__label">Cédulas aceitas</div><div class="kpi__val">${hidden ? '—' : nf.format(total)}</div>${hidden ? '<div class="kpi__sub">oculto até o fechamento</div>' : `<div class="kpi__spark">${sparkline(S.resultsHist)}</div>`}</div>
      <div class="kpi"><div class="kpi__label">Registrados</div><div class="kpi__val">${nf.format(registered)}</div>${part === null ? '<div class="kpi__sub">anônimos, por pleito</div>' : `<div class="meter" data-tip="Comparecimento|${nf.format(total)} de ${nf.format(registered)} registrados"><span style="width:${part}%"></span></div><div class="kpi__sub">${pctTxt(part)} já votaram</div>`}</div>
      <div class="kpi"><div class="kpi__label">Pleitos</div><div class="kpi__val">${cs.length}</div><div class="stack">${seg(byPhase.open, 'var(--primary)', 'Votação aberta')}${seg(byPhase.closed, 'var(--gold)', 'Encerrados')}${seg(byPhase.other, 'var(--surface-2)', 'Outras fases')}</div><div class="kpi__sub">${byPhase.open} abertos · ${byPhase.closed} encerrados</div></div>
      <div class="kpi"><div class="kpi__label">Rejeitadas</div><div class="kpi__val">${nf.format(rejected)}</div><div class="kpi__sub">${rejected ? 'veja o motivo no quadro' : 'nenhuma'}</div></div>
    </div>`;
    const govs = {}; cs.filter((c) => c.office === 'governor').forEach((c) => { govs[c.scope_code] = c; });
    const tile = (uf) => {
      const [col, row] = UF_TILES[uf]; const c = govs[uf];
      let cls = 'tile tile--none', style = '', tip = `${uf}|sem pleito de governador`, inner = uf;
      if (c) {
        const lead = c.distribution_visible && c.top[0] && c.top[0].votes > 0 ? c.top[0] : null;
        if (lead) {
          const share = c.valid ? (lead.votes * 100) / c.valid : 0;
          cls = 'tile tile--lead'; style = `background:${candColor(lead.index)};--a:${(0.45 + Math.min(1, share / 70) * 0.55).toFixed(2)}`;
          tip = `${uf} · ${c.title}|${lead.name}: ${pctTxt(share)} (${nf.format(lead.votes)} votos)`;
        } else {
          cls = `tile tile--${c.phase === 'voting_open' ? 'open' : ['voting_closed', 'tallied', 'audited'].includes(c.phase) ? 'closed' : 'wait'}`;
          tip = `${uf} · ${c.title}|${(PHASES[c.phase] || [c.phase])[0]}${c.accepted === null ? ' · resultado oculto até o fechamento' : ''}`;
        }
        cls += ' tile--click';
      }
      return `<button type="button" class="${cls}" style="grid-column:${col + 1};grid-row:${row + 1};${style}" data-tip="${esc(tip)}" ${c ? `data-act="mapstate" data-uf="${uf}"` : 'disabled tabindex="-1"'} aria-label="${esc(tip.replace('|', ': '))}">${uf}</button>`;
    };
    const anyLead = cs.some((c) => c.office === 'governor' && c.distribution_visible && c.top[0] && c.top[0].votes > 0);
    const map = `<article class="card mapcard"><div class="card__head"><div class="card__ico card__ico--onca">${ico('globe')}</div><div><h3>Governadores por estado</h3><div class="muted">${anyLead ? 'Cor = quem lidera · intensidade = vantagem' : 'Cor = situação do pleito'} · clique para ver</div></div></div>
      <div class="tilemap" role="group" aria-label="Mapa de estados">${Object.keys(UF_TILES).map(tile).join('')}</div>
      ${anyLead ? '' : `<div class="legend"><span class="lg lg--open"></span>votando <span class="lg lg--closed"></span>encerrado <span class="lg lg--wait"></span>aguardando <span class="lg lg--none"></span>sem pleito</div>`}</article>`;
    const html = `${kpis}${map}`;
    if (el.dataset.v !== html) { el.dataset.v = html; el.innerHTML = html; }
  }

  function officesOf(r) {
    const by = {};
    for (const c of r.contests) (by[c.office] = by[c.office] || []).push(c);
    return OFFICE_ORDER.filter((o) => by[o]).map((o) => [o, by[o]]);
  }

  function vResults() {
    const r = S.results;
    const sim = r && r.available && r.simulated;
    return `${head('Resultados', 'Quem está na frente', 'Passe o mouse ou clique nos gráficos para explorar.')}
      <div id="res-status"></div>
      ${!r ? `<article class="card"><div class="row">${ico('spin', 'spin')}<span>Lendo o quadro de avisos…</span></div></article>`
        : !r.available ? resUnavailable(r)
        : `${sim ? `<div class="simbanner" role="alert">${ico('alert')}<div><strong>Dados simulados</strong><div class="muted">Gerados por simulador: <strong>não</strong> é resultado de eleição.</div></div></div>` : ''}
           <details class="protonote"><summary>${ico('alert')}Contagem de protótipo: sem prova ZK; cédulas públicas</summary>
             <p class="muted">Há registro anônimo, mas ainda não há prova ZK nem nullifier. Com cédula pública, qualquer pessoa pode contar a distribuição mesmo quando a política a esconde da tela. A apuração é refeita por qualquer um a partir do quadro.</p></details>
           <div id="res-overview"></div>
           <div id="res-tabs"></div><div id="res-grid" class="results-grid"></div>`}`;
  }

  function resUnavailable(r) {
    return `<article class="card">
      <div class="card__head"><div class="card__ico card__ico--onca">${ico('chart')}</div><h2>Ainda não há resultados para mostrar</h2></div>
      <p class="muted">${esc(r.reason || '')}</p>
      ${S.devMode ? `<div class="row"><button class="btn" data-act="publishboard">${ico('upload')}Publicar a eleição de exemplo no quadro</button></div>
      <p class="muted">Depois, para ver os gráficos andando, abra outro terminal:</p>
      <div class="codeblock"><span class="codeline">make simulate-live   <span class="dim"># ~15 cédulas/s por 2 min (DADOS SIMULADOS)</span></span><span class="codeline">make simulate        <span class="dim"># 5.000 eleitores sintéticos de uma vez</span></span></div>` : '<p class="muted">O quadro é publicado pelos validadores da rede.</p>'}
    </article>`;
  }

  function relTime(ms) {
    const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
    return s < 2 ? 'agora' : s < 60 ? `há ${s} s` : `há ${Math.round(s / 60)} min`;
  }

  function netChip() {
    const n = S.net;
    if (!n) return '';
    if (n.mode !== 'net') return chip('Arquivo local (sem rede)', '', 'link');
    const up = n.nodes.filter((x) => x.up).length;
    const bad = n.nodes.some((x) => x.equivocations > 0);
    return `${chip(`${n.name} · altura ${n.height} · ${up}/${n.validators} nós`, bad ? 'bad' : up >= n.threshold ? 'ok' : 'warn', 'link')}`;
  }

  function renderResStatus() {
    const el = $('#res-status'); if (!el) return;
    const r = S.results;
    const ok = r && r.available && !S.resultsErr;
    const [visLabel, visKind] = (r && VIS[r.visibility]) || ['—', ''];
    const verifying = ok && r.verified < r.total ? verifyBar(r.verified, r.total) : '';
    el.innerHTML = `${verifying}<article class="card card--flat"><div class="livebar">
      <span class="live-dot ${ok ? '' : 'live-dot--off'}" aria-hidden="true"></span>
      <strong>${ok ? 'Ao vivo' : 'Sem conexão com o quadro'}</strong>${netChip()}
      <span class="muted">${S.resultsAt ? `atualizado ${relTime(S.resultsAt)}` : ''}</span>
      ${r && r.available ? `${chip(visLabel, visKind)}<span class="muted mono" title="Raiz Merkle do quadro ${esc(r.head_root)}">${nf.format(r.head_size)} entradas</span>${r.ignored_governance ? chip(`${r.ignored_governance} entradas de governança recusadas`, 'bad', 'alert') : ''}` : ''}

    </div></article>`;
  }

  function renderResTabs(r) {
    const el = $('#res-tabs'); if (!el) return;
    const offices = officesOf(r);
    if (!S.resultsOffice || !offices.some(([o]) => o === S.resultsOffice)) S.resultsOffice = offices[0] && offices[0][0];
    el.innerHTML = `<div class="segmented" role="group" aria-label="Cargo">${offices.map(([o, list]) => `<button data-act="resoffice" data-o="${o}" aria-pressed="${o === S.resultsOffice}">${esc(OFFICES[o] || o)}<span class="count">${list.length}</span></button>`).join('')}</div>`;
  }

  function barRow(o, rank, max, total, kind) {
    const pct = total > 0 ? (o.votes * 100) / total : 0;
    const w = max > 0 ? (o.votes * 100) / max : 0;
    const li = document.createElement('li');
    li.className = `bar-row${kind === 'others' ? ' bar-row--others' : ''}`;
    li.dataset.key = kind === 'others' ? 'others' : String(o.index);
    li.innerHTML = `<span class="bar-rank"></span><span class="bar-name"></span><span class="bar-track" aria-hidden="true"><span class="bar-fill"></span></span><span class="bar-val"><b></b><span></span></span>`;
    patchRow(li, o, rank, w, pct, kind);
    return li;
  }

  function patchRow(li, o, rank, w, pct, kind) {
    const set = (sel, text) => { const el = $(sel, li); if (el.textContent !== text) el.textContent = text; return el; };
    set('.bar-rank', kind === 'others' ? '' : String(rank));
    const nm = $('.bar-name', li);
    const label = kind === 'others' ? `Outros (${o.count})` : o.name;
    if (nm.dataset.l !== label) { nm.dataset.l = label; nm.innerHTML = ''; nm.append(label); if (o.number && kind !== 'others') { const sm = document.createElement('small'); sm.textContent = o.number; nm.append(sm); } nm.title = label; }
    const valEl = $('.bar-val b', li);
    const txt = nf.format(o.votes);
    if (valEl.textContent !== txt) { const had = valEl.textContent !== ''; valEl.textContent = txt; if (had) { li.classList.remove('bump'); void li.offsetWidth; li.classList.add('bump'); } }
    set('.bar-val span', `${pct.toFixed(1).replace('.', ',')}%`);
    $('.bar-fill', li).style.width = `${w}%`;
    li.setAttribute('role', 'img');
    li.setAttribute('aria-label', `${label}: ${txt} votos, ${pct.toFixed(1)}%`);
    li.classList.toggle('bar-row--lead', kind === 'lead');
    li.style.setProperty('--c', kind === 'others' ? '#8A978F' : candColor(o.index));
    li.dataset.tip = `${label}|${txt} votos · ${pctTxt(pct)}`;
  }

  function contestCardEl(c) {
    const art = document.createElement('article');
    art.className = `card rcard rcard--${c.office === 'governor' ? 'governor' : c.office === 'mayor' ? 'mayor' : c.office === 'president' ? 'president' : 'other'}`;
    art.dataset.contest = c.contest_id;
    art.innerHTML = `<div class="card__head"><div class="card__ico card__ico--${c.scope === 'national' ? 'arara' : c.scope === 'state' ? 'onca' : 'boto'}">${ico(c.scope === 'national' ? 'globe' : 'ballot')}</div>
      <div style="min-width:0"><h2></h2><div class="row" style="gap:6px;margin-top:6px"><span data-slot="chips"></span></div></div></div>
      <div class="row"><span class="muted" data-slot="total"></span></div>
      <div data-slot="body"></div>`;
    return art;
  }

  function patchContestCard(art, c, vis) {
    const h = $('h2', art); if (h.textContent !== c.title) h.textContent = c.title;
    const [phaseLabel, phaseKind] = PHASES[c.phase] || [c.phase, ''];
    const scope = c.scope === 'national' ? 'Nacional' : c.scope === 'state' ? `Estado ${c.scope_code}` : `Município ${c.scope_code}`;
    const chips = `${chip(scope, 'info')}${chip(phaseLabel, phaseKind)}`;
    const chipsEl = $('[data-slot=chips]', art); if (chipsEl.dataset.v !== chips) { chipsEl.dataset.v = chips; chipsEl.innerHTML = chips; }
    const totalEl = $('[data-slot=total]', art);
    const total = c.accepted === null ? 'Total oculto até o fechamento' : `${nf.format(c.accepted)} cédulas ${c.encrypted ? 'cifradas ' : ''}aceitas${c.registration_required ? ` · ${nf.format(c.registered)} registrados` : ''}${c.rejected ? ` · ${nf.format(c.rejected)} rejeitadas` : ''}${(c.blank || c.null) ? ` · ${nf.format(c.blank || 0)} em branco · ${nf.format(c.null || 0)} nulos (fora da base dos percentuais)` : ''}`;
    if (totalEl.textContent !== total) totalEl.textContent = total;
    const body = $('[data-slot=body]', art);
    if (!c.distribution_visible) {
      const closedPhase = ['voting_closed', 'tallied', 'audited'].includes(c.phase);
      const msg = c.encrypted && closedPhase ? 'Votação encerrada. Apurando: aguardando os validadores decifrarem os totais (basta a maioria).'
        : c.encrypted && c.phase === 'voting_open' ? 'Votos CIFRADOS: o resultado só existe depois do fechamento e ninguém, nem os validadores sozinhos, consegue vê-lo antes.'
        : c.accepted === null ? 'Oculto até o fechamento da votação (política do manifesto).'
        : c.phase === 'voting_open' ? 'Só o comparecimento aparece agora; a distribuição é divulgada depois do fechamento.'
        : 'Ainda sem votação neste pleito.';
      if (!body.querySelector('.hidden-dist') || body.dataset.m !== msg) { body.dataset.m = msg; body.innerHTML = `<div class="hidden-dist">${ico('lock')}<div>${esc(msg)}</div></div>`; }
      return;
    }
    let ul = body.querySelector('ul.bars');
    if (!ul) {
      body.innerHTML = '<div class="viz"><div class="viz__donut" data-slot="donut"></div><div class="viz__bars"><div class="lead-badge" data-slot="lead"></div></div></div>';
      body.dataset.m = ''; ul = document.createElement('ul'); ul.className = 'bars'; ul.setAttribute('aria-label', `Ranking: ${c.title}`); $('.viz__bars', body).append(ul);
    }
    const dn = $('[data-slot=donut]', body), dv = `${c.valid}|${c.top.map((o) => o.votes).join(',')}|${c.others_votes}`;
    if (dn.dataset.v !== dv) { dn.dataset.v = dv; dn.innerHTML = donutSvg(c); }
    const lb = $('[data-slot=lead]', body);
    const l1 = c.top[0], l2 = c.top[1];
    const badge = l1 && l1.votes > 0 && c.valid ? `<span class="dot" style="background:${candColor(l1.index)}"></span><strong>${esc(l1.name)}</strong> lidera${l2 ? ` com <b>+${pctTxt(((l1.votes - l2.votes) * 100) / c.valid).replace('%', ' p.p.')}</b> sobre ${esc(l2.name)}` : ''}` : '';
    if (lb.innerHTML !== badge) lb.innerHTML = badge;
    const rows = c.top.map((o) => ({ ...o }));
    if (c.others_count > 0) rows.push({ index: -1, votes: c.others_votes, count: c.others_count, others: true });
    const max = Math.max(1, ...c.top.map((o) => o.votes));
    const total2 = c.valid || 0;
    const existing = new Map([...ul.children].map((li) => [li.dataset.key, li]));
    rows.forEach((o, i) => {
      const key = o.others ? 'others' : String(o.index);
      const kind = o.others ? 'others' : i === 0 && o.votes > 0 ? 'lead' : '';
      const pct = total2 > 0 ? (o.votes * 100) / total2 : 0;
      let li = existing.get(key);
      if (li) { patchRow(li, o, i + 1, o.others ? (o.votes * 100) / Math.max(1, max) : (o.votes * 100) / max, pct, kind); existing.delete(key); }
      else li = barRow(o, i + 1, max, total2, kind);
      if (ul.children[i] !== li) ul.insertBefore(li, ul.children[i] || null);
    });
    existing.forEach((li) => li.remove());
  }

  function renderResults() {
    const r = S.results;
    renderResStatus();
    if (!r || !r.available) return;
    renderOverview(r);
    renderResTabs(r);
    const grid = $('#res-grid'); if (!grid) return;
    const list = (officesOf(r).find(([o]) => o === S.resultsOffice) || [null, []])[1]
      .slice().sort((a, b) => (a.scope_code || '').localeCompare(b.scope_code || '') || a.title.localeCompare(b.title));
    const want = list.map((c) => c.contest_id).join('|');
    if (grid.dataset.want !== want) {
      grid.dataset.want = want; grid.innerHTML = '';
      list.forEach((c) => grid.append(contestCardEl(c)));
    }
    list.forEach((c) => { const el = grid.querySelector(`[data-contest="${CSS.escape(c.contest_id)}"]`); if (el) patchContestCard(el, c, r.visibility); });
  }

  let resultsBusy = false;
  async function pollResults() {
    // sem sobreposição (uma verificação longa não empilha chamadas) e parado com a janela oculta
    if (S.route !== 'results' || resultsBusy || document.hidden) return;
    resultsBusy = true;
    try {
      S.results = await invoke('results_snapshot');
      S.resultsErr = null; S.resultsAt = Date.now();
      try { S.net = await invoke('network_status'); } catch { /* sem rede: tudo bem */ }
    } catch (e) { S.resultsErr = asErr(e).message; }
    const need = S.results && S.results.available ? 'ready' : 'empty';
    if (S.resultsShape !== need) { S.resultsShape = need; renderMain(); }
    renderResults();
    resultsBusy = false;
    // enquanto o quadro ainda está sendo verificado, pede de novo logo em seguida
    if (S.results && S.results.available && S.results.verified < S.results.total) setTimeout(pollResults, 50);
  }

  // ───────────────────────── ações ─────────────────────────
  const act = {
    nav(b) {
      const id = b.dataset.id;
      if (!routeAllowed(id)) { toast(id === 'onboarding' ? 'A configuração já foi concluída.' : 'Conecte e desbloqueie seu dispositivo para usar esta área.', 'bad'); return; }
      S.route = id; S.userPicked = true; S.lastKey = '';
      if (id === 'vote') S.elecChecked = false;
      if (id === 'explorer') { S.explorerTried = false; S.inclusion = null; }
      if (id === 'results') { S.resultsShape = null; setTimeout(pollResults, 0); }
      if (id === 'identity') S.idTried = false;
      if (id === 'home') { S.homeTried = false; S.home = null; }
      if (id === 'stake') { S.stakeTried = false; S.stake = null; }
      if (id === 'assets') { S.assetsTried = false; S.assets = null; }
      if (id === 'credentials') { S.credsTried = false; S.creds = null; }
      if (id === 'vote') S.candTried = false;
      if (id === 'wallet') { S.walletTried = false; S.walletAddr = null; S.balance = null; S.history = null; }
      if (id === 'identity') loadId();
      render();
    },
    async checkincl() {
      const index = Number($('#incl-idx').value);
      await guarded(async () => { S.inclusion = await invoke('verify_inclusion', { index }); });
      S.lastKey = ''; render();
    },
    gotoresults() { S.route = 'results'; S.userPicked = true; S.resultsShape = null; S.lastKey = ''; render(); pollResults(); },
    mapstate(b) {
      S.resultsOffice = 'governor'; renderResults();
      const g = $('#res-grid'); const c = (S.results.contests || []).find((x) => x.office === 'governor' && x.scope_code === b.dataset.uf);
      const card = c && g && g.querySelector(`[data-contest="${CSS.escape(c.contest_id)}"]`);
      if (card) { card.scrollIntoView({ behavior: 'smooth', block: 'center' }); card.classList.add('flash'); setTimeout(() => card.classList.remove('flash'), 1600); }
    },
    resoffice(b) { S.resultsOffice = b.dataset.o; $('#res-grid').dataset.want = ''; renderResults(); },
    async publishboard() {
      await guarded(async () => { const n = await invoke('board_publish'); toast(n ? 'Eleição publicada no quadro.' : 'A eleição já estava no quadro.', 'ok'); });
      S.resultsShape = null; pollResults();
    },
    async rescan() { await poll(); if (phase() === 'none') toast('Nenhum dispositivo encontrado. O emulador está rodando?'); },
    async auth() {
      S.authRunning = true; S.lastKey = ''; render();
      await guarded(async () => { S.genuine = await call('verify_authenticity', {}, { delay: 600 }); });
      S.authRunning = false; S.lastKey = ''; render();
    },
    fwretry() { S.fw = null; S.fwTried = false; S.lastKey = ''; render(); },
    authnext() { S.authAck = true; S.fw = null; S.fwTried = false; S.lastKey = ''; render(); loadFirmware(); },
    async fwinstall(b) {
      const v = Number(b.dataset.v);
      S.fwProgress = 0.02;
      await guarded(async () => {
        await call('install_firmware', { version: v }, { expect: { label: 'Impressão digital do firmware', value: b.dataset.fp } });
        toast(`Firmware v${v} instalado e verificado.`, 'ok');
      });
      S.fwProgress = null; S.fw = null; S.fwTried = false; S.lastKey = ''; await poll(); loadFirmware();
    },
    async pin() {
      await guarded(async () => { await call('setup_pin'); toast('PIN definido no dispositivo.', 'ok'); });
      S.lastKey = ''; await poll();
    },
    async walletnew() {
      await guarded(async () => { const w = await call('wallet_setup'); S.wallet = w; toast('Carteira criada e backup confirmado no dispositivo.', 'ok'); });
      S.lastKey = ''; await poll();
    },
    async walletrestore() {
      await guarded(async () => { const w = await call('wallet_restore'); S.wallet = w; toast('Carteira restaurada. Confira o endereço.', 'ok'); });
      S.lastKey = ''; await poll();
    },
    async walletsend() {
      const to = ($('#f-to') || {}).value || '', amount = parseTeko(($('#f-amt') || {}).value);
      if (!to.trim() || !amount || amount <= 0) { toast('Informe o endereço de destino e um valor válido maior que zero (até 6 casas decimais).'); return; }
      let r = null;
      await guarded(async () => { r = await call('wallet_send', { to: to.trim(), amount }, { delay: 300 }); });
      if (r) {
        toast(`Enviado: ${money(r.amount)} TEKO (taxa ${money(r.fee)}). Saldo: ${money(r.balance_after)} TEKO.`, 'ok');
        S.walletTried = false; S.balance = null; S.history = null; S.lastKey = ''; render(); loadWallet();
      }
    },
    async stakebond() { await stakeAct(1); },
    async stakeunbond() { await stakeAct(2); },
    async stakeclaim() { await stakeAct(3); },
    regsample() {
      if (S.reg.form) { S.reg.values = { ...S.reg.form.sample }; S.reg.error = ''; S.lastKey = ''; render(); }
    },
    async assetreg(b) {
      const cp = b.dataset.cp === '1';
      document.querySelectorAll('[data-reg]').forEach((i) => { S.reg.values[i.dataset.reg] = i.value; });
      try {
        const id = await invoke('asset_register_test', { address: S.walletAddr.address, class: S.reg.class, fields: S.reg.values, forCounterparty: cp });
        toast(`Órgão simulado registrou ${id} ${cp ? 'para a contraparte' : 'para você'}.`, 'ok');
        S.reg.values = {}; S.reg.error = ''; S.reg.form = null; S.assetsTried = false; S.lastKey = ''; render(); loadAssets();
      } catch (e) {
        S.reg.error = asErr(e).message; S.lastKey = ''; render();
      }
    },
    async credkey(b) {
      await guarded(async () => { await call('credential_key', { ctype: b.dataset.ctype }); toast('Chave de credencial gerada no dispositivo. Entregue-a ao órgão.', 'ok'); });
      S.credsTried = false; S.lastKey = ''; render(); loadCreds();
    },
    async recover() {
      await guarded(async () => { S.recovery = await call('recover_all'); toast('Tudo o que pertence a esta carteira foi relido da chain.', 'ok'); });
      S.credsTried = false; S.lastKey = ''; render(); loadCreds();
    },
    async credrefresh() { S.credsTried = false; S.lastKey = ''; render(); loadCreds(); },
    async assetlien(b) {
      await guarded(async () => { await call('asset_lien', { class: b.dataset.class, asset: b.dataset.id, on: b.dataset.on === '1' }); toast(b.dataset.on === '1' ? 'Órgão simulado registrou a penhora.' : 'Órgão simulado liberou o gravame.', 'ok'); });
      S.assetsTried = false; S.lastKey = ''; render(); loadAssets();
    },
    async assetfinalize(b) {
      await guarded(async () => { await call('asset_finalize', { class: b.dataset.class, asset: b.dataset.id }); toast('Transferência concluída: o bem foi ao comprador e o preço ao vendedor.', 'ok'); });
      S.assetsTried = false; S.walletTried = false; S.balance = null; S.history = null; S.lastKey = ''; render(); loadAssets();
    },
    async assetveto(b) {
      await guarded(async () => { await call('asset_veto', { class: b.dataset.class, asset: b.dataset.id }); toast('Órgão simulado vetou: o preço voltou ao comprador e o bem continua com o dono.', 'ok'); });
      S.assetsTried = false; S.walletTried = false; S.balance = null; S.history = null; S.lastKey = ''; render(); loadAssets();
    },
    async assetsell(b) { await trade(b.dataset.class, b.dataset.id, true); },
    async assetbuy(b) { await trade(b.dataset.class, b.dataset.id, false); },
    async walletcheck() {
      await guarded(async () => { await call('wallet_check_backup'); toast('Backup conferido: a frase confere com esta carteira.', 'ok'); });
    },
    async showid() {
      await guarded(async () => { S.civic = await call('civic_id'); });
      S.lastKey = ''; render();
    },
    async civicrequest() {
      await guarded(async () => { S.civicReq = await call('civic_request'); toast('Pedido gerado. Leve-o ao órgão (app Teko Emissor).', 'ok'); });
      S.lastKey = ''; render();
    },
    async civicimport() {
      await guarded(async () => { await call('civic_import'); toast('Credencial instalada no dispositivo.', 'ok'); });
      S.lastKey = ''; await poll();
    },
    lastack(b) { S.lastAck = b.checked; S.lastKey = ''; render(); },
    dismisswipe() { S.wiped = false; S.lastKey = ''; render(); },
    gohome() { S.route = 'home'; S.userPicked = true; S.homeTried = false; S.home = null; S.lastKey = ''; render(); },
    async unlock() {
      S.lockMsg = null; S.lastAck = false;
      await guarded(async () => {
        const r = await call('unlock');
        if (r.result === 'wrong_pin') { S.lockMsg = `PIN incorreto. Restam ${r.remaining} tentativa(s) antes do apagamento.`; }
        else { toast('Dispositivo desbloqueado.', 'ok'); S.userPicked = false; }
      });
      S.lastKey = ''; await poll();
    },
    async reveal() {
      await guarded(async () => { S.identity = await call('reveal_identity'); });
      S.lastKey = ''; render();
    },
    hideid() { S.identity = null; S.lastKey = ''; render(); },
    async pairwise() {
      const service = $('#f-svc').value.trim();
      if (!service) { toast('Informe o serviço.'); return; }
      await guarded(async () => { S.pairwise = { service, id: await call('pairwise_id', { service }) }; });
      S.lastKey = ''; render();
    },
    wipearm() { S.wipeArmed = true; S.lastKey = ''; render(); },
    wipecancel() { S.wipeArmed = false; S.lastKey = ''; render(); },
    async wipe() {
      S.expectWipe = true;
      await guarded(async () => { await call('wipe'); toast('Dispositivo apagado.', 'ok'); resetSession(); });
      S.expectWipe = false;
      S.userPicked = false; S.lastKey = ''; await poll();
    },
    copy(b) { navigator.clipboard.writeText(b.dataset.text).then(() => toast('Copiado.', 'ok'), () => toast('Não consegui copiar.')); },
    async sign() {
      const f = S.file; f.summary = $('#f-sum').value.trim() || f.name;
      await guarded(async () => {
        const doc = await call('sign_document', { bytes: Array.from(f.bytes), summary: f.summary }, { expect: { label: 'Hash SHA3-256 esperado', value: f.hash } });
        S.signed = { doc, fileName: f.name };
        toast('Documento assinado.', 'ok');
      });
      S.lastKey = ''; render();
    },
    async savesig() {
      await guarded(async () => {
        const path = await call('save_signed_document', { doc: S.signed.doc, fileName: S.signed.fileName }, { delay: 5000 });
        toast(`Assinatura salva em ${path}`, 'ok');
      });
    },
    copysig() { navigator.clipboard.writeText(JSON.stringify(S.signed.doc, null, 2)).then(() => toast('JSON copiado.', 'ok'), () => toast('Não consegui copiar.')); },
    async verify() {
      const bundle = $('#v-bundle').files[0], orig = $('#v-file').files[0];
      if (!bundle) { toast('Escolha o arquivo de assinatura (.json).'); return; }
      try {
        const text = await bundle.text();
        const file = orig ? Array.from(new Uint8Array(await orig.arrayBuffer())) : null;
        S.verifyRes = { ok: true, res: await invoke('verify_document', { bundleJson: text, file }) };
      } catch (e) { S.verifyRes = { ok: false, error: asErr(e).message }; }
      S.lastKey = ''; render();
    },
    async prove() {
      const p = S.prove;
      p.verifier = $('#p-ver').value.trim() || 'Verificador';
      const [, pred] = CHOICES[p.choice];
      p.result = null; p.error = null;
      await guarded(async () => {
        try { p.result = await call('prove_attribute', { ...pred, verifier: p.verifier }); }
        catch (e) { if (e.code === 'InvalidArgument') p.error = 'Sua credencial não permite provar isso (por exemplo, a condição de idade não é verdadeira). Nada foi revelado.'; else throw e; }
      });
      S.lastKey = ''; render();
    },
    async loadelection() {
      S.elecLoading = true;
      await guarded(async () => { S.election = await call('election_load'); toast('Eleição carregada e verificada pelo dispositivo.', 'ok'); });
      S.elecLoading = false; S.lastKey = ''; render();
    },
    async checkballot(b) {
      const id = b.dataset.contest;
      await guarded(async () => {
        S.checks[id] = await call('election_check_ballot', { electionId: S.election.election_id, contestId: id });
      });
      S.lastKey = ''; render();
    },
    async vote(b) {
      const id = b.dataset.contest;
      const idx = S.picks[id];
      if (idx === undefined) { toast('Escolha um candidato antes de votar.'); return; }
      await guarded(async () => {
        const r = await call('election_vote', { electionId: S.election.election_id, contestId: id, optionIndex: idx });
        if (r.audit) {
          S.audits[id] = r.audit;
          toast('Auditoria concluída: a cédula de teste foi descartada. Você ainda pode votar.', 'ok');
        } else {
          delete S.audits[id];
          S.ballots[id] = { ...r.ballot, tracker: r.tracker, submitted: r.submitted, board_index: r.board_index, note: r.note };
          S.election = await call('election_get');
          toast(r.submitted ? 'Cédula assinada e enviada ao quadro.' : 'Cédula assinada pelo dispositivo.', 'ok');
        }
      });
      S.lastKey = ''; render();
    },
  };

  async function loadFirmware() {
    if (S.fw || S.fwTried) return;
    S.fwTried = true; S.fwLoading = true;
    await guarded(async () => { S.fw = await call('firmware_catalog', {}, { delay: 5000 }); }, { silent: false });
    S.fwLoading = false;
    S.lastKey = ''; render();   // uma vez: mostra a lista ou o botão "Tentar de novo"
  }

  async function loadElec() {
    // UMA tentativa por visita à tela: sem isso, "nenhuma eleição carregada" dispara
    // render -> afterRender -> loadElec em laço infinito.
    if (S.election || S.elecChecked || phase() !== 'ready') return;
    S.elecChecked = true;
    try { S.election = await call('election_get', {}, { delay: 5000 }); } catch { S.election = null; }
    if (S.election) { S.lastKey = ''; render(); }
  }

  async function trade(cls, id, selling) {
    const price = parseTeko((document.getElementById(`px-${id}`) || {}).value);
    if (!price || price <= 0) { toast('Informe um preço válido maior que zero (até 6 casas decimais).'); return; }
    let bal = null;
    await guarded(async () => { bal = await call('asset_trade', { class: cls, asset: id, price, selling }, { delay: 300 }); });
    if (bal !== null) {
      toast(selling ? `Oferta de ${money(price)} TEKO assinada e aceita. A transferência fica pendente na janela de veto do órgão.` : `Aceite assinado: ${money(price)} TEKO em escrow. A transferência fica pendente na janela de veto do órgão. Seu saldo: ${money(bal)} TEKO.`, 'ok');
      S.assetsTried = false; S.walletTried = false; S.balance = null; S.history = null; S.lastKey = ''; render(); loadAssets();
    }
  }

  async function loadWallet() {
    if (S.walletTried || !['ready', 'wallet'].includes(phase())) return;
    S.walletTried = true;
    await guarded(async () => { S.walletAddr = await call('wallet_address', {}, { delay: 5000 }); }, { silent: true });
    if (S.walletAddr) {
      try { S.balance = await invoke('wallet_balance', { address: S.walletAddr.address }); } catch { S.balance = { available: false, reason: 'Não foi possível ler o saldo.' }; }
      try { S.history = await invoke('wallet_history', { address: S.walletAddr.address }); } catch { S.history = []; }
    }
    S.lastKey = ''; render();
  }

  // Resumo: junta o que as outras telas mostram (cada parte falha sozinha, sem derrubar o resto).
  async function loadHome() {
    if (S.homeTried || phase() !== 'ready') return;
    S.homeTried = true;
    const h = { civic: null, addr: null, balance: null, assets: null, stake: null, election: null };
    S.home = h;
    S.lastKey = ''; render();
    const safe = async (f) => { try { return await f(); } catch { return null; } };
    h.civic = await safe(() => call('civic_id', {}, { delay: 5000 }));
    h.addr = await safe(() => call('wallet_address', {}, { delay: 5000 }));
    S.lastKey = ''; render();
    if (h.addr) {
      const a = h.addr.address;
      [h.balance, h.assets, h.stake] = await Promise.all([
        safe(() => invoke('wallet_balance', { address: a })),
        safe(() => invoke('assets_view', { address: a })),
        safe(() => invoke('stake_view', { address: a })),
      ]);
    }
    h.election = await safe(() => call('election_get', {}, { delay: 5000 }));
    h.done = true;
    S.lastKey = ''; render();
  }

  function vHome() {
    const h = S.home || {};
    const wait = `<div class="row">${ico('spin', 'spin')}<span>Lendo…</span></div>`;
    const go = (id, label) => `<button class="btn btn--ghost btn--sm" data-act="nav" data-id="${id}">${esc(label)}</button>`;
    const short = (t) => (t && t.length > 22 ? `${t.slice(0, 12)}…${t.slice(-6)}` : t || '');
    const open = h.election ? h.election.contests.filter((c) => c.phase === 'voting_open' && c.eligible) : [];
    const pending = open.filter((c) => !c.voted);
    const kpi = (label, val, sub) => `<div class="kpi"><div class="kpi__label">${esc(label)}</div><div class="kpi__val">${val}</div><div class="kpi__sub">${sub}</div></div>`;
    const bal = h.balance && h.balance.available ? esc(money(h.balance.amount)) : '—';
    const assets = h.assets && h.assets.available ? esc(String(h.assets.mine.length)) : '—';
    const stk = h.stake && h.stake.available ? esc(money(h.stake.my_stake)) : '—';
    const testNote = h.balance && h.balance.available ? 'moedas de teste' : 'indisponível aqui';
    return `${head('Início', 'Tudo em um só lugar', 'Um resumo do que importa. Cada ação sensível continua sendo confirmada no dispositivo.')}
      <div class="kpis">
        ${kpi('Saldo', bal, testNote)}
        ${kpi('Bens', assets, 'em seu nome')}
        ${kpi('Stake', stk, 'travado como validador')}
        ${kpi('Votos pendentes', h.election ? esc(String(pending.length)) : '—', h.election ? `${open.length} pleito(s) aberto(s) para você` : 'nenhuma eleição carregada')}
      </div>
      ${pending.length ? callout('info', 'ballot', `Você ainda não votou em ${pending.length} pleito(s)`, `<p>${esc(pending.slice(0, 3).map((c) => c.title).join(' · '))}${pending.length > 3 ? '…' : ''}</p><div class="row" style="margin-top:8px">${go('vote', 'Ir votar')}</div>`) : ''}
      <article class="card">
        <div class="card__head"><div class="card__ico">${ico('id')}</div><h2>Identidade</h2></div>
        ${h.civic ? `<div class="screen"><div class="screen__label">ID Cívico</div><div class="id-big" style="font-size:.9rem">${esc(h.civic.id)}</div></div>` : wait}
        <div class="row">${go('identity', 'Minha identidade')}${go('sign', 'Assinar documento')}${go('prove', 'Provar atributo')}</div>
      </article>
      <article class="card">
        <div class="card__head"><div class="card__ico card__ico--boto">${ico('wallet')}</div><h2>Carteira</h2></div>
        ${h.addr ? `<p class="muted">Endereço: <code>${esc(short(h.addr.address))}</code></p>` : wait}
        <div class="row">${go('wallet', 'Carteira')}${go('assets', 'Bens')}${go('stake', 'Validadores')}</div>
      </article>
      <article class="card">
        <div class="card__head"><div class="card__ico card__ico--arara">${ico('shield')}</div><h2>Segurança</h2></div>
        <ul class="muted" style="margin:0;padding-left:18px;display:grid;gap:6px">
          <li>Chaves e PIN ficam <strong>só no dispositivo</strong>; este computador não os vê.</li>
          <li>Guarde a <strong>frase de 24 palavras</strong> em papel. Confira o backup na tela Carteira quando quiser.</li>
          <li>Confirme <strong>endereço, valor e taxa na tela do dispositivo</strong> antes de aprovar um envio.</li>
        </ul>
        <div class="row">${go('wallet', 'Verificar meu backup')}</div>
      </article>`;
  }

  async function loadStake() {
    if (S.stakeTried || !['ready', 'wallet'].includes(phase())) return;
    S.stakeTried = true;
    if (!S.walletAddr) await guarded(async () => { S.walletAddr = await call('wallet_address', {}, { delay: 5000 }); }, { silent: true });
    if (S.walletAddr) {
      try { S.stake = await invoke('stake_view', { address: S.walletAddr.address }); } catch (e) { S.stake = { available: false, reason: asErr(e).message }; }
    }
    S.lastKey = ''; render();
  }

  function vStake() {
    const k = S.stake;
    const h0 = head('Validadores', 'Stake e rede', 'Quem trava moeda como stake ajuda a validar a rede e pode ser penalizado se agir de má-fé. Cada ação é confirmada no dispositivo.');
    if (!k) return `${h0}<article class="card"><div class="row">${ico('spin', 'spin')}<span>Lendo…</span></div></article>`;
    if (!k.available) return `${h0}<article class="card">${callout('info', 'shield', 'Indisponível nesta versão', `<p>${esc(k.reason || 'O livro de economia ainda não está na rede.')}</p>`)}</article>`;
    const unb = k.unbonding.map(([rel, amt]) => `<dt>${esc(money(amt))} TEKO</dt><dd>${k.height >= rel ? chip('pronto para resgatar', 'ok', 'check') : chip(`libera no bloco ${rel} (faltam ${rel - k.height})`, 'warn')}</dd>`).join('');
    const total = k.validators.reduce((a, v) => a + v[1], 0);
    const phase = k.mode === 'node' ? founderCard(k) : '';
    return `${h0}
      ${phase}
      ${callout('warn', 'alert', 'Ambiente de teste', k.mode === 'node' ? '<p>Rede <strong>local</strong> de teste com consenso BFT (<code>make devnet</code>): moedas de <strong>teste</strong>, só na sua máquina. O que você vê aqui foi <strong>verificado por esta janela</strong> (certificados de cada bloco), não apenas informado por um nó.</p>' : '<p>O livro é local e as moedas são de <strong>teste</strong> (<code>make fund</code>). Em desenvolvimento cada transação é um bloco, por isso o prazo de saída é curto (3 blocos).</p>')}
      <div class="kpis">
        <div class="kpi"><div class="kpi__label">Saldo livre</div><div class="kpi__val">${esc(money(k.balance))}</div></div>
        <div class="kpi"><div class="kpi__label">Meu stake</div><div class="kpi__val">${esc(money(k.my_stake))}</div><div class="kpi__sub">${k.my_stake ? 'você é validador' : 'ainda não é validador'}</div></div>
        <div class="kpi"><div class="kpi__label">Penalidade</div><div class="kpi__val">${esc(String(k.slash_bps / 100))}%</div><div class="kpi__sub">por voto duplo comprovado</div></div>
      </div>
      <article class="card">
        <div class="card__head"><div class="card__ico card__ico--onca">${ico('stake')}</div><h2>Travar e sair</h2></div>
        <div class="fields">
          <div class="field"><label for="f-stk">Valor (TEKO)</label><input class="input" id="f-stk" type="text" inputmode="decimal" autocomplete="off" placeholder="ex.: 1000" aria-describedby="f-stk-h"><div class="muted" id="f-stk-h" style="font-size:.8rem">${AMOUNT_HELP}</div></div>
        </div>
        <div class="row" style="flex-wrap:wrap">
          <button class="btn" data-act="stakebond">${ico('lock')}Travar stake</button>
          <button class="btn btn--ghost" data-act="stakeunbond">${ico('unlock')}Sair do stake</button>
          <button class="btn btn--ghost" data-act="stakeclaim">${ico('check')}Resgatar o que venceu</button>
        </div>
        <p class="muted">Ao sair, o valor espera ${esc(String(k.unbonding_period))} blocos antes do resgate e ainda pode ser penalizado nesse período.</p>
        ${unb ? `<dl class="kv">${unb}</dl>` : ''}
      </article>
      <article class="card">
        <div class="card__head"><div class="card__ico card__ico--boto">${ico('users')}</div><h2>Validadores por stake</h2></div>
        ${k.validators.length ? `<dl class="kv">${k.validators.map(([key, st, mine]) => `<dt class="mono">${esc(key)} ${mine ? chip('você', 'ok') : ''}</dt><dd>${k.founder_phase_active ? `<strong>fundador</strong> <span class="muted">(recebe as recompensas de bloco)</span>` : `<strong>${esc(money(st))}</strong> <span class="muted">(${total ? Math.round((st * 100) / total) : 0}% do poder de voto)</span>`}</dd>`).join('')}</dl>` : '<p class="muted">Ninguém travou stake ainda.</p>'}
        <p class="muted">O poder de voto de cada validador no consenso (BFT) é proporcional ao stake; mais de 2/3 do stake precisa concordar para fechar um bloco.</p>
      </article>`;
  }

  function founderCard(k) {
    const pct = k.founder_target ? Math.min(100, Math.round((k.total_stake * 100) / k.founder_target)) : 0;
    if (k.founder_phase_active) {
      return `<article class="card">
        <div class="card__head"><div class="card__ico card__ico--onca">${ico('stake')}</div><h2>Fase de fundador</h2></div>
        <p class="muted">No começo, <strong>só o validador fundador</strong> produz os blocos e as <strong>recompensas</strong> vão para a carteira do fundador (sem pré-mineração). Travando essas moedas como stake, a rede passa a ser validada por <strong>quem tem stake</strong>. A fase termina quando o stake total chega a <strong>${esc(money(k.founder_target))} TEKO</strong>.</p>
        <div class="meter" role="progressbar" aria-valuenow="${pct}" aria-valuemin="0" aria-valuemax="100"><span style="width:${pct}%"></span></div>
        <p class="muted">${esc(money(k.total_stake))} de ${esc(money(k.founder_target))} TEKO (${pct}%) · bloco ${esc(String(k.height))}</p>
      </article>`;
    }
    return `<article class="card">
      <div class="card__head"><div class="card__ico card__ico--arara">${ico('check')}</div><h2>Fase de fundador encerrada</h2></div>
      <p class="muted">Desde o stake total de ${esc(money(k.founder_target))} TEKO, os validadores são os que têm <strong>stake</strong>, com poder de voto proporcional. Bloco ${esc(String(k.height))}.</p>
    </article>`;
  }

  async function stakeAct(action) {
    const amount = action === 3 ? 0 : parseTeko(($('#f-stk') || {}).value);
    if (action !== 3 && (!amount || amount <= 0)) { toast('Informe um valor válido maior que zero (até 6 casas decimais).'); return; }
    let r = null;
    await guarded(async () => { r = await call('stake_action', { action, amount: action === 3 ? 0 : amount }, { delay: 300 }); });
    if (r !== null) { toast(['', 'Stake travado.', 'Saída iniciada: aguarde o prazo.', 'Resgate concluído.'][action], 'ok'); S.stakeTried = false; S.assetsTried = false; S.walletTried = false; S.lastKey = ''; render(); loadStake(); }
  }

  async function loadAssets() {
    if (S.assetsTried || !['ready', 'wallet'].includes(phase())) return;
    S.assetsTried = true;
    if (!S.walletAddr) await guarded(async () => { S.walletAddr = await call('wallet_address', {}, { delay: 5000 }); }, { silent: true });
    if (S.walletAddr) {
      try { S.assets = await invoke('assets_view', { address: S.walletAddr.address }); } catch (e) { S.assets = { available: false, reason: asErr(e).message }; }
    }
    S.lastKey = ''; render();
  }

  async function loadCreds() {
    if (S.credsTried || !['ready', 'wallet'].includes(phase())) return;
    S.credsTried = true;
    try { S.creds = await invoke('credentials_view'); } catch (e) { S.creds = { available: false, reason: asErr(e).message, types: [], mine: [] }; }
    S.lastKey = ''; render();
  }

  const CRED_STATUS = { proposed: ['Aguardando aprovações do órgão', 'warn'], pending: ['Em contestação', 'info'], active: ['Ativa', 'ok'], expired: ['Expirada', 'bad'], vetoed: ['Vetada pelo órgão', 'bad'], revoked: ['Revogada', 'bad'], superseded: ['Sucedida por outro eleito', 'warn'] };
  const credDur = (s) => (s >= 172800 ? `${Math.round(s / 86400)} dias` : s >= 7200 ? `${Math.round(s / 3600)} h` : s >= 120 ? `${Math.round(s / 60)} min` : `${s} s`);
  const credDate = (t) => (t ? new Date(t * 1000).toLocaleDateString('pt-BR') : 'sem prazo');

  function vCreds() {
    const c = S.creds;
    const head0 = head('Credenciais', 'Minhas credenciais', 'Permissões e cargos emitidos para você, na chain, por órgãos e comunidades autorizados. A Suite só LÊ: quem emite é o app Teko Emissor, com o dispositivo do membro do órgão.');
    if (!c) return `${head0}<article class="card"><div class="row">${ico('spin', 'spin')}<span>Lendo a chain…</span></div></article>`;
    if (!c.available) return `${head0}<article class="card">${callout('info', 'shield', 'Indisponível', `<p>${esc(c.reason)}</p>`)}</article>`;
    const mine = c.mine.length ? c.mine.map((m) => {
      const [label, kind] = CRED_STATUS[m.status] || [m.status, ''];
      const when = m.status === 'pending' ? `vale a partir de ${credDate(m.active_at)}` : m.status === 'active' ? (m.valid_until ? `até ${credDate(m.valid_until)}` : 'vitalícia') : '';
      return `<article class="card card--flat"><div class="card__head"><h3>${esc(m.ctype)}</h3>${chip(label, kind)}</div>
        <p class="muted">${esc(when)}${m.reason ? ` · motivo: ${esc(m.reason)}` : ''}</p>
        ${m.claims ? `<p class="mono">${esc(m.claims)}</p>` : '<p class="muted">Os atributos ficam com você (a chain só guarda o compromisso).</p>'}</article>`;
    }).join('') : '<p class="muted">Nada emitido para as chaves exportadas nesta sessão. Gere a chave de um tipo e entregue-a ao órgão.</p>';
    const types = c.types.map((t) => `<article class="card card--flat">
        <div class="card__head"><h3>${esc(t.ctype)}</h3>${chip(t.public ? 'dados públicos' : 'dados só com você', t.public ? 'info' : 'ok')}</div>
        <p class="muted">Contestação ${credDur(t.contest_secs)} · validade ${t.validity_secs ? credDur(t.validity_secs) : 'vitalícia'}</p>
        ${t.key ? `<div class="codeblock mono">${esc(t.key)}</div>` : ''}
        <button class="btn btn--sm" data-act="credkey" data-ctype="${esc(t.ctype)}">${ico('key')}${t.key ? 'Mostrar a chave de novo' : 'Gerar minha chave deste tipo'}</button></article>`).join('');
    return `${head0}
      <article class="card"><div class="card__head"><div class="card__ico card__ico--arara">${ico('badge')}</div><h2>Emitidas para mim</h2>
        <button class="btn btn--sm btn--ghost" data-act="credrefresh">Atualizar</button></div>${mine}</article>
      <article class="card"><div class="card__head"><div class="card__ico card__ico--boto">${ico('shield')}</div><h2>Recuperar tudo desta carteira</h2></div>
        <p class="muted">Restaurou a carteira num dispositivo novo? Saldo, bens, histórico e credenciais estão na chain e as chaves vêm da sua frase: aqui a Suite os relê, já verificados. O dispositivo abre os atributos privados (um botão por credencial). <strong>O ID cívico e o registro de voto não voltam pela frase</strong>: seguem a reemissão presencial.</p>
        <div class="row"><button class="btn" data-act="recover">${ico('key')}Recuperar tudo</button></div>
        ${S.recovery ? `<dl class="kv"><dt>Endereço</dt><dd class="mono">${esc(S.recovery.address)}</dd><dt>Saldo</dt><dd>${esc(money(S.recovery.balance))} TEKO</dd><dt>Stake</dt><dd>${esc(money(S.recovery.stake))} TEKO</dd><dt>Bens</dt><dd>${S.recovery.assets}</dd><dt>Credenciais</dt><dd>${S.recovery.credentials.length}${S.recovery.unopened ? ` (${S.recovery.unopened} sem atributos recuperáveis)` : ''}</dd></dl>` : ''}
      </article>
      <article class="card"><div class="card__head"><div class="card__ico card__ico--onca">${ico('key')}</div><h2>Tipos de credencial da rede</h2></div>
        <p class="muted">A chave de credencial é <strong>por tipo</strong>, derivada da sua <strong>carteira</strong> (volta com a frase) e sem ligação pública com o seu endereço, a identidade ou o voto. Rede <span class="mono">${esc(c.chain_id)}</span>.</p>${types}</article>`;
  }

  const CLASS_LABEL = { veiculo: 'Veículo', imovel: 'Imóvel' };

  async function loadRegForm(cls) {
    if (S.reg.loading) return;
    S.reg.loading = true; S.reg.class = cls;
    try {
      S.reg.form = await invoke('asset_form', { class: cls, n: 1 + Math.floor(Math.random() * 900) });
      S.reg.values = {}; S.reg.error = '';
    } catch (e) { S.reg.error = asErr(e).message; }
    S.reg.loading = false; S.lastKey = ''; render();
  }

  function regForm() {
    const f = S.reg.form;
    const opts = Object.entries(CLASS_LABEL).map(([k, v]) => `<option value="${k}"${k === S.reg.class ? ' selected' : ''}>${v}</option>`).join('');
    const fields = f ? f.fields.map((x) => `<div class="field"><label for="rf-${esc(x.key)}">${esc(x.label)}</label><input class="input" id="rf-${esc(x.key)}" data-reg="${esc(x.key)}" autocomplete="off" value="${esc(S.reg.values[x.key] ?? '')}" aria-describedby="rh-${esc(x.key)}"><div class="muted" id="rh-${esc(x.key)}" style="font-size:.8rem">${esc(x.help)}</div></div>`).join('') : '<p class="muted">Carregando o formulário…</p>';
    return `<div class="field"><label for="reg-class">Classe do bem</label><select class="input" id="reg-class">${opts}</select></div>
      <p class="muted">O órgão simulado da classe registra o bem. Na chain vai só o <strong>hash</strong> destes dados; os dados ficam neste computador e o seu dispositivo os mostra antes de você vender.</p>
      ${fields}
      ${S.reg.error ? `<p role="alert">${chip(S.reg.error, 'bad', 'alert')}</p>` : ''}
      <div class="row"><button class="btn btn--ghost" data-act="regsample">${ico('badge')}Preencher exemplo</button>
        <button class="btn" data-act="assetreg" data-cp="0">${ico('badge')}Órgão: registrar para mim</button>
        <button class="btn btn--ghost" data-act="assetreg" data-cp="1">${ico('badge')}Órgão: registrar para a contraparte</button></div>`;
  }

  function assetCard(a, mine) {
    const price = `<input class="input" style="max-width:120px" id="px-${esc(a.id)}" type="text" inputmode="decimal" autocomplete="off" placeholder="preço em TEKO" aria-label="preço em TEKO">`;
    const data = `data-class="${esc(a.class)}" data-id="${esc(a.id)}"`;
    const off = a.verified ? '' : ' disabled title="os metadados deste bem não estão neste computador"';
    return `<div class="fw__item"><div>
        <h3>${esc(CLASS_LABEL[a.class] || a.class)} · ${esc(a.id)} ${a.liens ? chip('PENHORADO', 'bad', 'lock') : ''}</h3>
        ${a.summary && a.summary.length ? `<p>${a.summary.map(esc).join(' · ')} ${chip('metadados verificados', 'ok', 'check')}</p>` : `<p class="muted">metadados indisponíveis neste computador (só o hash está na chain)</p>`}
        <p class="muted">versão ${esc(String(a.version))} · ${a.liens ? `${a.liens} gravame(s): não pode ser vendido` : 'sem gravames'}</p>
      </div>
      <div class="row" style="flex-wrap:wrap;gap:8px">
        ${mine ? `${price}<button class="btn" data-act="assetsell" ${data}${off}>${ico('upload')}Vender à contraparte</button>
          <button class="btn btn--ghost btn--sm" data-act="assetlien" ${data} data-on="${a.liens ? '0' : '1'}">${ico('lock')}${a.liens ? 'Órgão: liberar gravame' : 'Órgão: penhorar'}</button>`
        : `${price}<button class="btn" data-act="assetbuy" ${data}${off}>${ico('wallet')}Comprar da contraparte</button>`}
      </div></div>`;
  }

  function fmtLeft(sec) {
    if (sec <= 0) return 'a janela terminou';
    if (sec < 120) return `${sec} s`;
    if (sec < 7200) return `${Math.ceil(sec / 60)} min`;
    return `${Math.ceil(sec / 3600)} h`;
  }

  function pendingCard(a) {
    if (!a.pending || !a.pending.length) return '';
    const rows = a.pending.map((t) => {
      const data = `data-class="${esc(t.class)}" data-id="${esc(t.asset)}"`;
      const mine = t.role === 'seller' ? 'Você está vendendo' : 'Você está comprando';
      if (!t.accepted) return `<div class="fw__item"><div><h3>${esc(CLASS_LABEL[t.class] || t.class)} · ${esc(t.asset)}</h3><p>${mine} por ${esc(money(t.price))} TEKO · ${chip('oferecida, aguardando o aceite', 'warn', 'alert')}</p></div></div>`;
      const left = t.release_at - a.now;
      const ready = left <= 0;
      return `<div class="fw__item"><div><h3>${esc(CLASS_LABEL[t.class] || t.class)} · ${esc(t.asset)}</h3>
        <p>${mine} por ${esc(money(t.price))} TEKO · ${ready ? chip('janela de veto encerrada', 'ok', 'check') : chip(`pendente: faltam ${fmtLeft(left)}`, 'warn', 'alert')}</p>
        <p class="muted">O preço está em escrow. Se o órgão vetar dentro da janela, ele volta ao comprador e o bem continua com o dono; senão, qualquer um conclui.</p></div>
        <div class="row" style="flex-wrap:wrap;gap:8px">
          <button class="btn" data-act="assetfinalize" ${data}${ready ? '' : ' disabled title="a janela de veto ainda está aberta"'}>${ico('check')}Concluir transferência</button>
          <button class="btn btn--ghost btn--sm" data-act="assetveto" ${data}${ready ? ' disabled' : ''}>${ico('lock')}Órgão: vetar (teste)</button>
        </div></div>`;
    }).join('');
    return `<article class="card"><div class="card__head"><div class="card__ico card__ico--onca">${ico('alert')}</div><h2>Negociações em andamento</h2></div><div class="fw">${rows}</div></article>`;
  }

  function vAssets() {
    const a = S.assets;
    const head0 = head('Bens', 'Patrimônio e negócios', 'Bens registrados em seu nome. Você e o comprador negociam direto, cada um com a PRÓPRIA transação assinada no dispositivo; o preço fica em escrow numa janela em que o órgão pode vetar, e depois qualquer um conclui.');
    if (!a) return `${head0}<article class="card"><div class="row">${ico('spin', 'spin')}<span>Lendo…</span></div></article>`;
    if (!a.available) return `${head0}<article class="card">${callout('info', 'shield', 'Indisponível nesta versão', `<p>${esc(a.reason || 'O livro de bens ainda não está na rede.')}</p>`)}</article>`;
    return `${head0}
      ${callout('warn', 'alert', 'Ambiente de teste', '<p>O <strong>órgão registrador</strong> e a <strong>contraparte</strong> abaixo são <strong>simulados</strong> (chaves de teste em <code>dev/</code>) e o livro é local. Seu consentimento, porém, é assinado <strong>no seu dispositivo</strong>, que mostra o bem, as partes, o preço e a versão. Moedas de teste: <code>make fund</code>.</p>')}
      ${pendingCard(a)}
      <article class="card">
        <div class="card__head"><div class="card__ico card__ico--onca">${ico('home')}</div><h2>Meu patrimônio</h2></div>
        <p class="muted">Seu saldo: <strong>${esc(money(a.balance))}</strong> TEKO de teste.</p>
        ${a.mine.length ? `<div class="fw">${a.mine.map((x) => assetCard(x, true)).join('')}</div>` : '<p class="muted">Você ainda não tem bens. Peça ao órgão simulado para registrar um bem de teste.</p>'}
      </article>
      <article class="card">
        <div class="card__head"><div class="card__ico card__ico--onca">${ico('badge')}</div><h2>Registrar bem de teste</h2></div>
        ${regForm()}
      </article>
      <article class="card">
        <div class="card__head"><div class="card__ico card__ico--boto">${ico('users')}</div><h2>Contraparte simulada</h2></div>
        <p class="muted">Outra pessoa de teste, para vender ou comprar. Saldo: <strong>${esc(money(a.counterparty_balance))}</strong> TEKO.</p>
        <div class="screen"><div class="screen__label">Endereço da contraparte</div><div class="id-big" style="font-size:.8rem">${esc(a.counterparty_address)}</div></div>
        <div class="row"><button class="btn btn--ghost btn--sm" data-act="copy" data-text="${esc(a.counterparty_address)}">${ico('copy')}Copiar</button></div>
        ${a.counterparty_assets.length ? `<div class="fw">${a.counterparty_assets.map((x) => assetCard(x, false)).join('')}</div>` : '<p class="muted">A contraparte ainda não tem bens.</p>'}
        <p class="muted">Para comprar, a contraparte precisa ter bem; para ela comprar de você, ela precisa de moedas (<code>make fund ADDR=&lt;endereço acima&gt; AMOUNT=1000</code>).</p>
      </article>`;
  }

  async function loadId() {
    if (S.civic || S.idTried || phase() !== 'ready') return;
    S.idTried = true;
    await guarded(async () => { S.civic = await call('civic_id', {}, { delay: 5000 }); }, { silent: true });
    if (S.civic) { S.lastKey = ''; render(); }
  }

  async function readFile(file) {
    if (file.size > 20 * 1024 * 1024) { toast('Arquivo grande demais (limite: 20 MB).'); return; }
    const bytes = new Uint8Array(await file.arrayBuffer());
    const hash = await invoke('hash_file', { bytes: Array.from(bytes) });
    S.file = { name: file.name, bytes, hash, summary: file.name };
    S.signed = null;
    S.lastKey = ''; render();
  }

  function afterRender() {
    // Entrada nas telas que precisam de dados do dispositivo.
    if (S.route === 'onboarding' && onboardStep() === 'auth' && !S.genuine && !S.authRunning && !S.busy) act.auth();
    if (S.route === 'onboarding' && onboardStep() === 'firmware') loadFirmware();
    if (S.route === 'home') loadHome();
    if (S.route === 'identity') loadId();
    if (S.route === 'wallet') loadWallet();
    if (S.route === 'credentials') loadCreds();
    if (S.route === 'vote' && S.election && !S.candTried) { S.candTried = true; invoke('candidate_checks').then((r) => { S.candChecks = r; S.lastKey = ''; render(); }).catch(() => { S.candChecks = []; }); }
    if (S.route === 'assets') {
      loadAssets();
      if (!S.reg.form && !S.reg.loading && S.assets && S.assets.available) loadRegForm(S.reg.class);
      const sel = $('#reg-class');
      if (sel) sel.addEventListener('change', () => { S.reg.values = {}; S.reg.form = null; loadRegForm(sel.value); });
      document.querySelectorAll('[data-reg]').forEach((i) => i.addEventListener('input', () => { S.reg.values[i.dataset.reg] = i.value; }));
    }
    if (S.route === 'stake') loadStake();
    if (S.route === 'vote') loadElec();
    if (S.route === 'explorer') loadExplorer();
    if (S.route === 'results') renderResults();
    const dz = $('#dz');
    if (dz) {
      ['dragenter', 'dragover'].forEach((ev) => dz.addEventListener(ev, (e) => { e.preventDefault(); dz.classList.add('is-over'); }));
      ['dragleave', 'drop'].forEach((ev) => dz.addEventListener(ev, (e) => { e.preventDefault(); dz.classList.remove('is-over'); }));
      dz.addEventListener('drop', (e) => { const f = e.dataTransfer.files[0]; if (f) readFile(f); });
    }
    const fi = $('#f-file');
    if (fi) fi.addEventListener('change', () => { if (fi.files[0]) readFile(fi.files[0]); });
    document.querySelectorAll('input[name="choice"]').forEach((r) => r.addEventListener('change', () => { S.prove.choice = r.value; S.prove.result = null; S.prove.error = null; }));
    document.querySelectorAll('input[name^="cand-"]').forEach((r) => r.addEventListener('change', () => { S.picks[r.name.slice(5)] = Number(r.value); }));
  }

  // ───────────────────────── tema ─────────────────────────
  function themePref() { try { return localStorage.getItem('teko-theme') || 'auto'; } catch { return 'auto'; } }
  function applyTheme(v) {
    const root = document.documentElement;
    if (v === 'auto') root.removeAttribute('data-theme'); else root.setAttribute('data-theme', v);
    try { localStorage.setItem('teko-theme', v); } catch { /* sem armazenamento: tudo bem */ }
  }

  // ───────────────────────── start ─────────────────────────
  document.addEventListener('click', (e) => {
    const t = e.target.closest('[data-theme-set]');
    if (t) { applyTheme(t.dataset.themeSet); renderSide(); return; }
    const b = e.target.closest('[data-act]');
    if (b && act[b.dataset.act]) act[b.dataset.act](b);
  });

  async function start() {
    applyTheme(themePref());
    try { S.info = await invoke('dev_info'); S.dev.addr = S.info.addr; } catch { /* ok */ }
    try {
      await listen('device-prompt', (ev) => {
        S.prompt = ev.payload;
        if (!$('#overlay').hidden || S.busy) { renderOverlay(); $('#overlay').hidden = false; }
      });
      await listen('fw-progress', (ev) => {
        const [done, total] = ev.payload;
        S.fwProgress = total ? done / total : 0;
        const bar = $('.progress > span');
        if (bar) bar.style.width = `${Math.round(S.fwProgress * 100)}%`;
      });
    } catch (e) {
      toast('Não consegui ouvir eventos do dispositivo; os avisos de confirmação podem não aparecer.');
    }
    try { S.devMode = await invoke('dev_mode'); } catch { S.devMode = false; }
    await poll();
    setInterval(poll, 2000);
    setInterval(pollResults, 1500);
    setInterval(renderResStatus, 1000);
  }

  if (!TAURI) {
    document.body.innerHTML = '<main class="main"><h1>Teko Suite</h1><p>Abra pelo aplicativo (Tauri).</p></main>';
  } else {
    start();
  }
})();
