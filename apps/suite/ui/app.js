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
    idTried: false,       // evitam laços de re-tentativa em loadId/loadFirmware
    fwTried: false,
    fwLoading: false,   // já perguntei ao dispositivo por uma eleição nesta visita?
    picks: {},           // contest_id -> índice escolhido
    ballots: {},         // contest_id -> cédula assinada (protótipo)
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
  };

  const NAV = [
    { id: 'onboarding', label: 'Configurar dispositivo', icon: 'usb', need: 'onboarding' },
    { id: 'identity', label: 'Minha identidade', icon: 'id', need: 'ready' },
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
    return 'onboarding';
  }

  function defaultRoute() {
    switch (phase()) {
      case 'none': return 'explorer';
      case 'locked': return 'unlock';
      case 'onboarding': return 'onboarding';
      default: return 'identity';
    }
  }

  function routeAllowed(r) {
    const p = phase();
    if (r === 'explorer' || r === 'results') return true;
    if (r === 'unlock') return p === 'locked';
    if (r === 'onboarding') return p === 'onboarding';
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
    S.genuine = null; S.authAck = false; S.fw = null; S.civic = null; S.identity = null;
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
    const views = { results: vResults, explorer: vExplorer, onboarding: vOnboarding, unlock: vUnlock, identity: vIdentity, sign: vSign, prove: vProve, vote: vVote };
    m.innerHTML = `<div class="view">${(views[S.route] || vExplorer)()}</div>`;
    afterRender();
  }

  function head(eyebrow, title, desc) {
    return `<header class="pagehead"><div class="eyebrow">${esc(eyebrow)}</div><h1>${esc(title)}</h1>${desc ? `<p>${esc(desc)}</p>` : ''}</header>`;
  }
  const callout = (kind, icon, title, body) => `<div class="callout callout--${kind}">${ico(icon)}<div>${title ? `<strong>${esc(title)}</strong>` : ''}${body}</div></div>`;

  // ───────────────────────── explorador (sem dispositivo) ─────────────────────────
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

      <div class="grid grid--3">
        ${[['list', 'Eleições', 'Calendário e regras de cada eleição.', ''],
           ['users', 'Candidatos', 'Quem concorre e suas propostas.', 'arara'],
           ['chart', 'Resultados apurados', 'Totais verificáveis, apurados publicamente.', 'onca']]
          .map(([i, t, d, c]) => `
          <article class="card">
            <div class="card__head"><div class="card__ico ${c ? `card__ico--${c}` : ''}">${ico(i)}</div><h3>${t}</h3></div>
            <p class="muted">${d}</p>
            ${t === 'Resultados apurados'
              ? `<div class="row"><button class="btn" data-act="gotoresults">${ico('chart')}Ver resultados ao vivo</button></div>`
              : `<div class="empty">${ico('link')}<div><strong>Em breve</strong><br>Será alimentado pela blockchain de votação.</div></div>`}
          </article>`).join('')}
      </div>

      ${connected ? '' : `
      <article class="card">
        <div class="card__head"><div class="card__ico card__ico--boto">${ico('usb')}</div><h3>Como conectar o dispositivo (emulador)</h3></div>
        <p class="muted">Em um terminal, na pasta do projeto:</p>
        <div class="codeblock"><span class="dim"># 1) uma vez: cria as chaves de teste, firmware e fábrica</span><br>cargo run -p issuer-cli -- dev-setup<br><span class="dim"># 2) liga o dispositivo (tela e botões ficam NESTE terminal)</span><br>cargo run -p device-emulator</div>
        ${S.dev.trust_ok ? '' : callout('warn', 'alert', 'Falta preparar o ambiente de teste', '<p>Não encontrei <code>dev/trust.txt</code>. Rode o passo 1 acima.</p>')}
      </article>`}`;
  }

  // ───────────────────────── onboarding ─────────────────────────
  const STEPS = [['detect', 'Dispositivo'], ['auth', 'Autenticidade'], ['firmware', 'Firmware'], ['pin', 'PIN'], ['id', 'ID Cívico'], ['issue', 'Credencial'], ['done', 'Pronto']];

  function onboardStep() {
    const st = S.dev.status;
    if (!st) return 'detect';
    switch (st.state) {
      case 'virgin': return S.authAck ? 'firmware' : 'auth';
      case 'firmware_installed': return 'pin';
      case 'initialized': return S.civic ? 'issue' : 'id';
      default: return 'done';
    }
  }

  function vOnboarding() {
    const cur = onboardStep();
    const idx = STEPS.findIndex(([k]) => k === cur);
    const stepper = STEPS.map(([k, label], i) => `
      <li class="step ${i < idx ? 'step--done' : i === idx ? 'step--now' : ''}" ${i === idx ? 'aria-current="step"' : ''}>
        <span class="step__n">${i < idx ? ico('check') : i + 1}</span>${esc(label)}</li>`).join('');
    const body = { detect: sDetect, auth: sAuth, firmware: sFirmware, pin: sPin, id: sId, issue: sIssue, done: sDone }[cur]();
    const wipedBanner = S.wiped
      ? `<div class="callout callout--bad" role="alert">${ico('alert')}<div><strong>Seu dispositivo foi apagado</strong>
          <p>Houve excesso de tentativas de PIN (ou o dispositivo foi apagado). Chaves, ID Cívico e credencial foram <strong>destruídos</strong> e não há seed para recuperar. É preciso configurar de novo e refazer a emissão da credencial (na vida real, presencialmente na autoridade).</p>
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
      ${callout('warn', 'alert', 'Guarde bem o PIN', '<p>Sem seed de recuperação: se perder o PIN ou errar 8 vezes, o dispositivo se apaga e é preciso refazer a emissão presencialmente. PINs fracos (123456, 111111) são recusados pelo dispositivo.</p>')}
      <div class="row"><button class="btn" data-act="pin">${ico('lock')}Definir PIN no dispositivo</button></div>
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
    return `<article class="card">
      <div class="card__head"><div class="card__ico">${ico('id')}</div><h2>Seu ID Cívico</h2></div>
      <div class="screen"><div class="screen__label">Confira com o dispositivo</div><div class="id-big">${esc(S.civic.id)}</div></div>
      <p class="muted">Não é RG nem CPF: é derivado da sua chave pública e não contém dados pessoais.</p>
    </article>
    <article class="card">
      <div class="card__head"><div class="card__ico card__ico--boto">${ico('badge')}</div><h2>Emissão da credencial</h2></div>
      ${callout('info', 'shield', 'Simulação da autoridade', '<p>Na vida real você é verificado presencialmente pela autoridade, que digita estes dados. Aqui o <code>issuer-cli</code> faz esse papel: confere que o dispositivo é genuíno e que possui a chave, e assina a credencial vinculada à sua chave de identidade.</p>')}
      <div class="fields">
        <div class="field"><label for="f-name">Nome completo</label><input class="input" id="f-name" value="Maria da Silva" autocomplete="off"></div>
        <div class="field"><label for="f-birth">Data de nascimento</label><input class="input" id="f-birth" type="date" value="1990-05-17"></div>
        <div class="field"><label for="f-nat">Nacionalidade</label>${combo('f-nat', geoCountries(), 'BR', 'Digite para buscar o país')}</div>
        <div class="field"><label for="f-uf">Domicílio eleitoral: estado</label>${combo('f-uf', geoStates(), 'SP', 'Digite para buscar o estado')}<span class="hint">Define em quais pleitos você vota (governador)</span></div>
        <div class="field"><label for="f-mun">Domicílio eleitoral: município</label>${combo('f-mun', geoCities('SP'), '3550308', 'Escolha o estado e busque o município')}<span class="hint">Define o pleito de prefeito</span></div>
      </div>
      <p class="muted">Quem atesta o domicílio no mundo real é o cadastro eleitoral; aqui o <code>issuer-cli</code> simula esse papel.</p>
      <div class="row"><button class="btn" data-act="issue">${ico('badge')}Emitir credencial</button></div>
    </article>`;
  }

  function sDone() {
    const st = S.dev.status;
    return `<article class="card">
      <div class="card__head"><div class="card__ico">${ico('check')}</div><h2>Tudo pronto</h2></div>
      <dl class="kv">
        <dt>Autenticidade</dt><dd>${S.genuine && S.genuine.genuine ? 'Dispositivo genuíno' : 'não verificada nesta sessão'}</dd>
        <dt>Firmware</dt><dd>v${st.fw_version}</dd>
        <dt>Credencial</dt><dd>Emitida e vinculada à sua chave</dd>
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

  function contestCard(c) {
    const [phaseLabel, phaseKind] = PHASES[c.phase] || [c.phase, ''];
    const canVote = c.phase === 'voting_open' && c.eligible && !c.voted && Date.now() / 1000 < c.voting_closes;
    const ballot = S.ballots[c.contest_id];
    const why = !c.eligible ? 'Fora do seu domicílio eleitoral: o dispositivo recusa.'
      : c.voted ? 'Este dispositivo já votou neste pleito.'
      : c.phase !== 'voting_open' ? `Ainda não dá para votar: ${phaseLabel.toLowerCase()}.` : '';
    const opts = c.option_names.map((n, i) => `
      <label class="option"><input type="radio" name="cand-${esc(c.contest_id)}" value="${i}" ${S.picks[c.contest_id] === i ? 'checked' : ''} ${canVote ? '' : 'disabled'}>
        <span>${esc(n)}</span></label>`).join('');
    return `<article class="card" ${c.eligible ? '' : 'style="opacity:.82"'}>
      <div class="card__head"><div class="card__ico card__ico--${c.scope === 'national' ? 'arara' : c.scope === 'state' ? 'onca' : 'boto'}">${ico(c.scope === 'national' ? 'globe' : 'ballot')}</div>
        <div><h2>${esc(c.title)}</h2>
        <div class="row" style="gap:6px;margin-top:6px">${chip(scopeLabel(c), 'info')}${chip(phaseLabel, phaseKind)}${c.eligible ? chip(c.scope === 'national' ? 'Todos votam' : 'Seu domicílio', 'ok', 'check') : chip('Fora do seu domicílio', 'bad', 'lock')}${c.voted ? chip('Você já votou', 'ok', 'badge') : ''}${c.registration_required ? chip('Registro anônimo', 'info', 'shield') : ''}</div></div></div>
      <p class="muted">Votação: ${esc(fmtDate(c.voting_opens))} até ${esc(fmtDate(c.voting_closes))}</p>
      <div class="options" role="radiogroup" aria-label="Candidatos: ${esc(c.title)}">${opts}</div>
      ${why ? `<p class="muted">${esc(why)}</p>` : ''}
      ${canVote && c.registration_required ? '<p class="muted">Este pleito exige registro anônimo: o dispositivo pedirá <strong>duas confirmações</strong> (registro e voto).</p>' : ''}
      ${canVote ? `<div class="row"><button class="btn btn--gold" data-act="vote" data-contest="${esc(c.contest_id)}">${ico('ballot')}Votar neste pleito</button></div>` : ''}
      ${ballot ? callout('info', 'shield', 'Cédula assinada pelo dispositivo (protótipo)', `<p>Voto em <strong>${esc(ballot.option_name)}</strong>. Assinatura <span class="mono">${esc(short(ballot.signature, 10, 8))}</span> conferida pela Suite.
        ${ballot.submitted ? `Enviada ao quadro local (posição #${ballot.board_index}): <a href="#" data-act="gotoresults">veja nos resultados ao vivo</a>.` : esc(ballot.note || 'Não foi enviada ao quadro.')}
        ${c.registration_required ? 'Registro anônimo (assinatura cega) conferido; ainda sem prova ZK nem anulador: protótipo.' : 'Sem registro anônimo nem prova de elegibilidade: protótipo.'}</p>`) : ''}
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

  function officesOf(r) {
    const by = {};
    for (const c of r.contests) (by[c.office] = by[c.office] || []).push(c);
    return OFFICE_ORDER.filter((o) => by[o]).map((o) => [o, by[o]]);
  }

  function vResults() {
    const r = S.results;
    const sim = r && r.available && r.simulated;
    return `${head('Resultados · tempo real', 'Quem está na frente, pleito a pleito', 'Cada gráfico é a apuração pública de um pleito: o top 10 do mais votado ao menos votado, atualizado a cada cédula que entra no quadro de avisos.')}
      <div id="res-status"></div>
      ${!r ? `<article class="card"><div class="row">${ico('spin', 'spin')}<span>Lendo o quadro de avisos…</span></div></article>`
        : !r.available ? resUnavailable(r)
        : `${sim ? `<div class="simbanner" role="alert">${ico('alert')}<div><strong>Dados simulados</strong><div class="muted">As cédulas abaixo foram geradas por um simulador. Isto <strong>não</strong> é resultado de uma eleição.</div></div></div>` : ''}
           ${callout('warn', 'alert', 'Contagem de protótipo, não confiável', '<p>Ainda não há registro anônimo nem prova de elegibilidade: qualquer chave é aceita. A apuração é refeita por qualquer pessoa a partir do quadro, mas <strong>não protege contra fraude</strong>. Com cédula pública, a distribuição é contável por todos mesmo que a política diga o contrário.</p>')}
           <div id="res-tabs"></div><div id="res-grid" class="results-grid"></div>`}`;
  }

  function resUnavailable(r) {
    return `<article class="card">
      <div class="card__head"><div class="card__ico card__ico--onca">${ico('chart')}</div><h2>Ainda não há resultados para mostrar</h2></div>
      <p class="muted">${esc(r.reason || '')}</p>
      <div class="row"><button class="btn" data-act="publishboard">${ico('upload')}Publicar a eleição de exemplo no quadro</button></div>
      <p class="muted">Depois, para ver os gráficos andando, abra outro terminal:</p>
      <div class="codeblock"><span class="codeline">make simulate-live   <span class="dim"># ~15 cédulas/s por 2 min (DADOS SIMULADOS)</span></span><span class="codeline">make simulate        <span class="dim"># 5.000 eleitores sintéticos de uma vez</span></span></div>
    </article>`;
  }

  function relTime(ms) {
    const s = Math.max(0, Math.round((Date.now() - ms) / 1000));
    return s < 2 ? 'agora' : s < 60 ? `há ${s} s` : `há ${Math.round(s / 60)} min`;
  }

  function renderResStatus() {
    const el = $('#res-status'); if (!el) return;
    const r = S.results;
    const ok = r && r.available && !S.resultsErr;
    const [visLabel, visKind] = (r && VIS[r.visibility]) || ['—', ''];
    el.innerHTML = `<article class="card card--flat"><div class="livebar">
      <span class="live-dot ${ok ? '' : 'live-dot--off'}" aria-hidden="true"></span>
      <strong>${ok ? 'Ao vivo' : 'Sem conexão com o quadro'}</strong>
      <span class="muted">${S.resultsAt ? `atualizado ${relTime(S.resultsAt)}` : ''}</span>
      ${r && r.available ? `${chip(visLabel, visKind)}<span class="muted mono" title="Raiz Merkle do quadro">quadro: ${r.head_size} entradas · ${esc(short(r.head_root, 8, 6))}</span>${r.ignored_governance ? chip(`${r.ignored_governance} entradas de governança recusadas`, 'bad', 'alert') : ''}` : ''}
      <span class="spacer"></span>${r && r.available ? `<span class="muted">${esc(r.title)} · rev. ${r.revision}</span>` : ''}
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
    const total = c.accepted === null ? 'Total oculto até o fechamento' : `${nf.format(c.accepted)} cédulas aceitas${c.registration_required ? ` · ${nf.format(c.registered)} registrados` : ''}${c.rejected ? ` · ${nf.format(c.rejected)} rejeitadas` : ''}`;
    if (totalEl.textContent !== total) totalEl.textContent = total;
    const body = $('[data-slot=body]', art);
    if (!c.distribution_visible) {
      const msg = c.accepted === null ? 'Oculto até o fechamento da votação (política do manifesto).'
        : c.phase === 'voting_open' ? 'Só o comparecimento aparece agora; a distribuição é divulgada depois do fechamento.'
        : 'Ainda sem votação neste pleito.';
      if (!body.querySelector('.hidden-dist') || body.dataset.m !== msg) { body.dataset.m = msg; body.innerHTML = `<div class="hidden-dist">${ico('lock')}<div>${esc(msg)}</div></div>`; }
      return;
    }
    let ul = body.querySelector('ul.bars');
    if (!ul) { body.innerHTML = ''; body.dataset.m = ''; ul = document.createElement('ul'); ul.className = 'bars'; ul.setAttribute('aria-label', `Ranking: ${c.title}`); body.append(ul); }
    const rows = c.top.map((o) => ({ ...o }));
    if (c.others_count > 0) rows.push({ index: -1, votes: c.others_votes, count: c.others_count, others: true });
    const max = Math.max(1, ...c.top.map((o) => o.votes));
    const total2 = c.accepted || 0;
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

  async function pollResults() {
    if (S.route !== 'results') return;
    try {
      S.results = await invoke('results_snapshot');
      S.resultsErr = null; S.resultsAt = Date.now();
    } catch (e) { S.resultsErr = asErr(e).message; }
    const need = S.results && S.results.available ? 'ready' : 'empty';
    if (S.resultsShape !== need) { S.resultsShape = need; renderMain(); }
    renderResults();
  }

  // ───────────────────────── ações ─────────────────────────
  const act = {
    nav(b) {
      const id = b.dataset.id;
      if (!routeAllowed(id)) { toast(id === 'onboarding' ? 'A configuração já foi concluída.' : 'Conecte e desbloqueie seu dispositivo para usar esta área.', 'bad'); return; }
      S.route = id; S.userPicked = true; S.lastKey = '';
      if (id === 'vote') S.elecChecked = false;
      if (id === 'results') { S.resultsShape = null; setTimeout(pollResults, 0); }
      if (id === 'identity') S.idTried = false;
      if (id === 'identity') loadId();
      render();
    },
    gotoresults() { S.route = 'results'; S.userPicked = true; S.resultsShape = null; S.lastKey = ''; render(); pollResults(); },
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
    async showid() {
      await guarded(async () => { S.civic = await call('civic_id'); });
      S.lastKey = ''; render();
    },
    async issue() {
      const name = $('#f-name').value, birth = $('#f-birth').value, nat = comboValue('f-nat');
      const state = comboValue('f-uf'), municipality = comboValue('f-mun');
      if (!state || !municipality || !nat) { toast('Escolha nacionalidade, estado e município na lista.'); return; }
      await guarded(async () => {
        await call('issue_credential', { name, birthDate: birth, nationality: nat, state, municipality });
        toast('Credencial emitida e instalada no dispositivo.', 'ok');
      });
      S.lastKey = ''; await poll();
    },
    lastack(b) { S.lastAck = b.checked; S.lastKey = ''; render(); },
    dismisswipe() { S.wiped = false; S.lastKey = ''; render(); },
    gohome() { S.route = 'identity'; S.userPicked = true; S.lastKey = ''; loadId(); render(); },
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
    async vote(b) {
      const id = b.dataset.contest;
      const idx = S.picks[id];
      if (idx === undefined) { toast('Escolha um candidato antes de votar.'); return; }
      await guarded(async () => {
        const r = await call('election_vote', { electionId: S.election.election_id, contestId: id, optionIndex: idx });
        S.ballots[id] = { ...r.ballot, submitted: r.submitted, board_index: r.board_index, note: r.note };
        S.election = await call('election_get');
        toast(S.ballots[id].submitted ? 'Cédula assinada e enviada ao quadro local.' : 'Cédula assinada pelo dispositivo.', 'ok');
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
    if (S.route === 'identity') loadId();
    if (S.route === 'vote') loadElec();
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
