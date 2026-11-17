/* Teko Emissor — UI. Sem framework: um estado, uma função render(), ações por data-act.
   O Emissor NUNCA vê o PIN: ele é digitado no dispositivo. Cada operação é montada, mostrada e
   assinada no dispositivo do membro do órgão (ADR-0037). */
(() => {
  'use strict';
  const TAURI = window.__TAURI__;
  const invoke = (cmd, args) => TAURI.core.invoke(cmd, args);
  const listen = (ev, cb) => TAURI.event.listen(ev, cb);

  const esc = (s) => String(s ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const $ = (sel, root = document) => root.querySelector(sel);
  const short = (s, a = 10, b = 8) => (s.length > a + b + 3 ? `${s.slice(0, a)}…${s.slice(-b)}` : s);
  const asErr = (e) => (e && typeof e === 'object' && 'message' in e ? e : { code: 'Internal', message: String(e) });
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

  // ───────────────────────── seleção com busca (valores fixos, nada de texto livre) ─────────────────────────
  const G = window.TEKO_GEO || { ufs: [], municipios: {}, paises: [] };
  const geoCountries = () => G.paises.map(([v, n]) => ({ value: v, label: n, hint: v }));
  const geoStates = () => G.ufs.map(([v, n]) => ({ value: v, label: n, hint: v }));
  const geoCities = (uf) => (G.municipios[uf] || []).map(([v, n]) => ({ value: String(v), label: n, hint: uf }));
  const fold = (s) => String(s).normalize('NFD').replace(/[\u0300-\u036f]/g, '').toLowerCase();
  const COMBO_CITY_OF = { 'c-uf': 'c-mun', 'k-uf': 'k-mun' };   // estado -> o município que ele filtra
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
    if (COMBO_CITY_OF[id]) {   // o município depende do estado
      const m = document.querySelector(`[data-combo="${COMBO_CITY_OF[id]}"]`);
      if (m) { comboData[COMBO_CITY_OF[id]] = geoCities(item.value); m.dataset.value = ''; $('input', m).value = ''; $('input', m).focus(); comboOpen(m, ''); }
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


  const toast = (msg, kind = 'bad') => {
    const el = document.createElement('div');
    el.className = `toast toast--${kind}`;
    el.setAttribute('role', kind === 'bad' ? 'alert' : 'status');
    el.innerHTML = `<div>${esc(msg)}</div>`;
    $('#toasts').appendChild(el);
    setTimeout(() => el.remove(), kind === 'ok' ? 4500 : 9000);
  };

  const S = {
    route: 'orgs',
    dev: null,          // resposta de device_status
    ov: null,           // resposta de overview
    ovError: '',
    prompt: null,       // o que o dispositivo está pedindo agora
    busy: false,
    filter: 'todas',
    result: null,       // resultado da última emissão
    form: { ctype: '', holder: '', claims: '', days: 365 },
    voting: null,       // {op, id, ctype} aguardando o motivo
    civicReqs: [],      // pedidos de ID cívico (dev/requests)
    civicFile: '',
    civicForm: { name: '', birth: '', nat: 'BR', uf: 'SP', mun: '3550308', reg: '', days: 3650 },
    delivered: '',
    candForm: { eleicao: 'geral-2026', days: 120, cargo: 'governador', uf: 'SP', mun: '', numero: '', nome: '', partido: '', legenda: '', holder: '', batch: '' },
    candResult: [],
    // certidões (ADR-0047)
    marriageForm: { matricula: '', data: '', regime: 'comunhao-parcial', cartorio: '', an: '', ak: '', bn: '', bk: '' },
    divorceForm: { matricula: '', data: '', forma: 'extrajudicial', cartorio: '' },
    marriages: [],
    civilResult: [],
  };

  const PROMPTS = {
    credential_key: ['Confirme a CHAVE DE CREDENCIAL no dispositivo', 'Só a chave pública deste tipo será mostrada. Ela é derivada da carteira do titular e não se liga publicamente ao endereço, à identidade nem ao voto.'],
    sign_credential_op: ['Confirme a operação do ÓRGÃO no dispositivo', 'Confira o tipo, o titular, os atributos, a validade e o motivo na tela do dispositivo. Só confirme se for exatamente o combinado.'],
    pin_entry: ['Digite o PIN no dispositivo', 'O PIN nunca passa por este computador.'],
    other: ['Confirme no dispositivo', 'Siga as instruções na tela do dispositivo.'],
  };

  const STATUS = {
    proposed: ['Aguardando aprovações', 'warn'],
    pending: ['Em contestação', 'info'],
    active: ['Ativa', 'ok'],
    expired: ['Expirada', 'bad'],
    vetoed: ['Vetada', 'bad'],
    revoked: ['Revogada', 'bad'],
    superseded: ['Sucedida', 'warn'],
  };
  const chip = (t, k = '') => `<span class="chip ${k ? `chip--${k}` : ''}">${esc(t)}</span>`;

  const fmtDur = (s) => {
    if (s >= 172800) return `${Math.round(s / 86400)} dias`;
    if (s >= 7200) return `${Math.round(s / 3600)} h`;
    if (s >= 120) return `${Math.round(s / 60)} min`;
    return `${s} s`;
  };
  const fmtDate = (t) => (t ? new Date(t * 1000).toLocaleDateString('pt-BR') : 'sem prazo');
  const fmtLeft = (t, now) => (t > now ? `faltam ${fmtDur(t - now)}` : 'agora');

  // ───────────────────────── dispositivo e rede ─────────────────────────
  async function call(cmd, args) {
    S.busy = true; S.prompt = null; renderOverlay();
    try { return await invoke(cmd, args); }
    finally { S.busy = false; S.prompt = null; renderOverlay(); }
  }

  async function refreshDevice() {
    try { S.dev = await invoke('device_status'); } catch (e) { S.dev = { connected: false, error: asErr(e).message }; }
  }
  async function refreshOverview() {
    try { S.ov = await invoke('overview'); S.ovError = ''; } catch (e) { S.ovError = asErr(e).message; }
  }
  async function refreshCivic() {
    try { S.civicReqs = await invoke('civic_requests'); } catch (e) { S.civicReqs = []; }
  }
  async function refreshMarriages() {
    try { S.marriages = await invoke('civil_marriages'); } catch (e) { S.marriages = []; }
  }
  async function refresh() { await refreshDevice(); await refreshOverview(); if (S.route === 'civic') await refreshCivic(); if (S.route === 'civil') await refreshMarriages(); render(); }

  const unlocked = () => !!(S.dev && S.dev.connected && S.dev.status && !S.dev.status.locked);
  const myOrgs = () => (S.ov ? S.ov.orgs.filter((o) => o.is_member) : []);

  // ───────────────────────── ações ─────────────────────────
  const ACTIONS = {
    async nav(el) { S.route = el.dataset.id; S.result = null; if (S.route === 'civic') await refreshCivic(); if (S.route === 'civil') await refreshMarriages(); render(); },
    async unlock() {
      try {
        const r = await call('unlock');
        if (r && r.result === 'unlocked') toast('Dispositivo desbloqueado.', 'ok');
        else toast('PIN errado. Tente de novo no dispositivo.', 'bad');
      } catch (e) { toast(asErr(e).message); }
      await refresh();
    },
    async refresh() { await refresh(); toast('Atualizado.', 'ok'); },
    async holderkey() {
      const ctype = $('#f-ctype').value;
      if (!ctype) { toast('Escolha o tipo de credencial.'); return; }
      try { $('#f-holder').value = await call('holder_key', { ctype }); toast('Chave deste dispositivo preenchida (teste: você será o titular).', 'ok'); }
      catch (e) { toast(asErr(e).message); }
    },
    async issue() {
      S.form = { ctype: $('#f-ctype').value, holder: $('#f-holder').value.trim(), claims: $('#f-claims').value, days: Number($('#f-days').value) };
      if (!S.form.ctype || !S.form.holder) { toast('Escolha o tipo e informe a chave do titular.'); return; }
      try {
        S.result = await call('issue', { form: { ctype: S.form.ctype, holder_key: S.form.holder, claims: S.form.claims, valid_days: S.form.days } });
        toast('Proposta enviada. Ela conta como a primeira aprovação.', 'ok');
        S.form.claims = '';
      } catch (e) { toast(asErr(e).message); }
      await refreshOverview(); render();
    },
    async vote(el) {
      const { op, id, ctype } = el.dataset;
      if (op !== 'approve') { S.voting = { op, id, ctype }; render(); return; }
      await sendVote({ op, id, ctype, reason: '' });
    },
    cancelvote() { S.voting = null; render(); },
    async sendvote() {
      const reason = $('#f-reason').value.trim();
      if (!reason) { toast('Informe o motivo público.'); return; }
      const v = S.voting; S.voting = null;
      await sendVote({ ...v, reason });
    },
    pickrequest(el) { S.civicFile = el.dataset.file; render(); },
    async civicissue() {
      S.civicForm = { name: $('#c-name').value, birth: $('#c-birth').value, nat: comboValue('c-nat'), uf: comboValue('c-uf'), mun: comboValue('c-mun'), reg: $('#c-reg').value, days: Number($('#c-days').value) };
      if (!S.civicFile) { toast('Escolha um pedido verificado.'); return; }
      if (!S.civicForm.nat || !S.civicForm.uf || !S.civicForm.mun) { toast('Escolha a nacionalidade, o estado e o município na lista.'); return; }
      try {
        S.result = await call('civic_issue', { form: { request_file: S.civicFile, name: S.civicForm.name, birth_date: S.civicForm.birth, nationality: S.civicForm.nat, state: S.civicForm.uf, municipality: S.civicForm.mun, registry_id: S.civicForm.reg || null, valid_days: S.civicForm.days } });
        toast('Proposta enviada. Ela conta como a primeira aprovação.', 'ok');
      } catch (e) { toast(asErr(e).message); }
      await refreshOverview(); render();
    },
    async civicdeliver(el) {
      try { S.delivered = await call('civic_deliver', { id: el.dataset.id }); toast('Pacote entregue ao titular (dev/inbox).', 'ok'); }
      catch (e) { toast(asErr(e).message); }
      render();
    },
    async candissue() {
      const f = { eleicao: $('#k-eleicao').value.trim(), days: Number($('#k-days').value), cargo: $('#k-cargo').value, uf: comboValue('k-uf'), mun: comboValue('k-mun'), numero: $('#k-numero').value.trim(), nome: $('#k-nome').value.trim(), partido: $('#k-partido').value.trim(), legenda: $('#k-legenda').value.trim(), holder: $('#k-holder').value.trim(), batch: $('#k-batch').value };
      S.candForm = f;
      // one candidate from the fields, plus one per line of the batch: cargo|uf|municipio|numero|nome|chave|partido|legenda
      const items = [];
      if (f.nome || f.numero || f.holder) items.push({ cargo: f.cargo, uf: f.uf, municipio: f.mun, numero: f.numero, nome: f.nome, partido: f.partido, legenda: f.legenda, eleicao: f.eleicao, holder_key: f.holder, valid_days: f.days });
      for (const line of f.batch.split('\n').map((l) => l.trim()).filter(Boolean)) {
        const [cargo, uf, municipio, numero, nome, holder, partido, legenda] = line.split('|').map((x) => (x || '').trim());
        items.push({ cargo, uf, municipio, numero, nome, partido: partido || '', legenda: legenda || '', eleicao: f.eleicao, holder_key: holder, valid_days: f.days });
      }
      if (!items.length) { toast('Preencha uma candidatura ou o lote.'); return; }
      try {
        S.candResult = await call('candidacies_issue', { items });
        const ok = S.candResult.filter((r) => !r.error).length;
        toast(`${ok} de ${S.candResult.length} candidaturas propostas. Faltam as aprovações dos outros membros.`, ok ? 'ok' : 'bad');
      } catch (e) { toast(asErr(e).message); }
      await refreshOverview(); render();
    },
    async marriage() {
      const v = (id) => $(id).value.trim();
      const f = { matricula: v('#m-mat'), data: v('#m-data'), regime: v('#m-regime'), cartorio: v('#m-cart'), an: v('#m-an'), ak: v('#m-ak'), bn: v('#m-bn'), bk: v('#m-bk') };
      S.marriageForm = f;
      try {
        S.civilResult = await call('civil_marriage_issue', { form: { matricula: f.matricula, data: f.data, regime: f.regime, cartorio: f.cartorio, a: { nome: f.an, holder_key: f.ak }, b: { nome: f.bn, holder_key: f.bk } } });
        const ok = S.civilResult.filter((r) => !r.error).length;
        toast(`${ok} de ${S.civilResult.length} certidões de casamento propostas. Faltam as aprovações dos outros membros.`, ok === S.civilResult.length ? 'ok' : 'bad');
      } catch (e) { toast(asErr(e).message); }
      await refreshOverview(); await refreshMarriages(); render();
    },
    async divorce() {
      const v = (id) => $(id).value.trim();
      const f = { matricula: v('#d-mat'), data: v('#d-data'), forma: v('#d-forma'), cartorio: v('#d-cart') };
      S.divorceForm = f;
      try {
        S.civilResult = await call('civil_divorce_issue', { form: f });
        const bad = S.civilResult.filter((r) => r.error).length;
        toast(bad ? 'O divórcio não foi concluído: veja o resultado.' : 'Certidões de divórcio propostas e casamento com a revogação iniciada. Faltam os votos dos outros membros.', bad ? 'bad' : 'ok');
      } catch (e) { toast(asErr(e).message); }
      await refreshOverview(); await refreshMarriages(); render();
    },
    filter(el) { S.filter = el.dataset.id; render(); },
    async copy(el) { try { await navigator.clipboard.writeText($(el.dataset.target).textContent); toast('Copiado.', 'ok'); } catch { toast('Não consegui copiar.'); } },
  };

  async function sendVote(f) {
    try {
      await call('vote', { form: f });
      toast({ approve: 'Aprovação enviada.', veto: 'Veto enviado.', revoke: 'Revogação enviada.' }[f.op], 'ok');
    } catch (e) { toast(asErr(e).message); }
    await refreshOverview(); render();
  }

  document.addEventListener('click', (ev) => {
    const el = ev.target.closest('[data-act]');
    if (!el || el.getAttribute('aria-disabled') === 'true' || el.disabled) return;
    const fn = ACTIONS[el.dataset.act];
    if (fn) fn(el);
  });

  // ───────────────────────── telas ─────────────────────────
  const head = (eyebrow, title, desc) => `<header class="pagehead"><div class="eyebrow">${esc(eyebrow)}</div><h1>${esc(title)}</h1>${desc ? `<p>${esc(desc)}</p>` : ''}</header>`;
  const callout = (kind, title, body) => `<div class="callout callout--${kind}"><div>${title ? `<strong>${esc(title)}</strong> ` : ''}${body}</div></div>`;

  function gate() {
    if (!S.dev || !S.dev.connected) return callout('warn', 'Dispositivo não encontrado.', `Abra o emulador (<code>make emulator</code>) e plugue-o. ${S.dev && S.dev.error ? esc(S.dev.error) : ''}`);
    if (S.dev.status.state === 'virgin' || S.dev.status.state === 'firmware_installed') return callout('warn', 'Dispositivo sem configuração.', 'Configure o dispositivo (PIN e carteira) na Teko Suite primeiro.');
    if (S.dev.status.locked) return `${callout('warn', 'Dispositivo bloqueado.', 'O PIN é digitado só no dispositivo.')}<p><button class="btn" data-act="unlock">Desbloquear no dispositivo</button></p>`;
    if (!S.dev.status.wallet_ready) return callout('warn', 'Sem carteira.', 'O membro de um órgão assina com a carteira do dispositivo: crie-a na Teko Suite.');
    return '';
  }

  function vOrgs() {
    const g = gate();
    const ov = S.ov;
    const note = ov && ov.wallet ? `<p class="muted">Sua carteira: <span class="mono">${esc(ov.wallet)}</span> · rede <span class="mono">${esc(ov.chain_id)}</span></p>` : '';
    const body = !ov ? `<article class="card"><p class="muted">${esc(S.ovError || 'Carregando a chain…')}</p></article>`
      : `<div class="grid">${ov.orgs.map((o) => `
        <article class="card">
          <div class="card__head"><div><h3>${esc(o.ctype)}</h3></div>${o.is_member ? chip('Você é membro', 'ok') : chip('Somente leitura')}</div>
          <div class="kv"><span>Quórum</span><strong>${o.threshold} de ${o.members} membros</strong></div>
          <div class="kv"><span>Janela de contestação</span><strong>${fmtDur(o.contest_secs)}</strong></div>
          <div class="kv"><span>Validade</span><strong>${o.validity_secs ? fmtDur(o.validity_secs) : 'vitalícia'}</strong></div>
          <div class="kv"><span>Dados</span><strong>${o.public ? 'públicos na chain' : 'só o compromisso na chain'}</strong></div>
          <div class="kv"><span>Credenciais</span><strong>${o.credentials.length}</strong></div>
        </article>`).join('')}</div>`;
    return `${head('Órgãos', 'Quem pode emitir o quê', 'Cada tipo de credencial é governado por um órgão com quórum M-de-N. Você só emite nos tipos em que sua carteira é membro.')}${g}${note}${body}
      ${S.ov && !myOrgs().length && !g ? callout('info', 'Sua carteira não é membro de nenhum órgão.', 'Na rede de teste: <code>make devnet FOUNDER=&lt;sua carteira&gt;</code> (ou <code>ISSUER=</code>) põe a sua carteira nos órgãos de teste.') : ''}`;
  }

  function vIssue() {
    const g = gate();
    const orgs = myOrgs();
    if (g || !orgs.length) return `${head('Emitir', 'Emitir uma credencial', '')}${g || callout('info', 'Sem órgão.', 'Sua carteira não é membro de nenhum órgão.')}`;
    const sel = S.form.ctype || orgs[0].ctype;
    const org = orgs.find((o) => o.ctype === sel) || orgs[0];
    const res = S.result ? `<article class="card"><div class="card__head"><h3>Proposta enviada</h3></div>
      <p>Identificador <span class="mono">${esc(short(S.result.id, 12, 8))}</span>. ${S.result.bundle ? 'Entregue o pacote abaixo ao titular (ele guarda os atributos; a chain só tem o compromisso):' : 'Os atributos são públicos e já estão na chain.'}</p>
      ${S.result.bundle ? `<pre class="codeblock" id="bundle">${esc(S.result.bundle)}</pre><p><button class="btn btn--sm btn--ghost" data-act="copy" data-target="#bundle">Copiar o pacote</button> <span class="muted">salvo em ${esc(S.result.bundle_path)}</span></p>` : ''}
      <p class="muted">Faltam as aprovações dos outros membros (veja a aba Credenciais). Em teste: <code>make devnet-cosign</code>.</p></article>` : '';
    return `${head('Emitir', 'Emitir uma credencial', 'Você propõe; a proposta conta como a primeira aprovação. Tudo é conferido e assinado no dispositivo.')}
      <article class="card">
        <div class="issue-form">
          <label class="field"><span>Tipo de credencial</span>
            <select class="input" id="f-ctype">${orgs.map((o) => `<option value="${esc(o.ctype)}" ${o.ctype === sel ? 'selected' : ''}>${esc(o.ctype)} (${o.threshold} de ${o.members})</option>`).join('')}</select></label>
          <label class="field"><span>Chave de credencial do titular (hex)</span>
            <input class="input mono" id="f-holder" value="${esc(S.form.holder)}" placeholder="o titular mostra esta chave na Suite dele" autocomplete="off" spellcheck="false"></label>
          <p><button class="btn btn--sm btn--ghost" data-act="holderkey">Usar a chave deste dispositivo (teste: emitir para mim)</button></p>
          <label class="field"><span>Atributos (${org.public ? 'públicos na chain' : 'ficam com o titular'})</span>
            <textarea class="input" id="f-claims" rows="4" placeholder="${org.ctype.startsWith('candidato') ? 'cargo=governador;uf=SP;numero=13;nome=Ana Souza' : 'nome=Ana Souza;categoria=B;registro=123456'}">${esc(S.form.claims)}</textarea></label>
          <label class="field"><span>Validade (dias${org.validity_secs ? `, no máximo ${Math.floor(org.validity_secs / 86400)}` : ': este tipo não expira'})</span>
            <input class="input" id="f-days" type="number" min="1" value="${S.form.days}" ${org.validity_secs ? '' : 'disabled'}></label>
        </div>
        ${callout('info', 'Janela de contestação.', `Depois do quórum, a credencial fica <strong>${fmtDur(org.contest_secs)}</strong> em contestação (o órgão pode vetar) e só então vale.`)}
        <p><button class="btn" data-act="issue">Emitir no dispositivo</button></p>
      </article>${res}`;
  }

  function vCivic() {
    const g = gate();
    const org = myOrgs().find((o) => o.ctype === 'civic');
    if (g || !org) return `${head('ID cívico', 'Emitir o ID cívico', '')}${g || callout('info', 'Sem órgão.', 'Sua carteira não é membro do órgão do ID cívico (tipo <code>civic</code>).')}`;
    const f = S.civicForm;
    const reqs = S.civicReqs.length ? S.civicReqs.map((r) => `<label class="card card--flat"><input type="radio" name="req" data-act="pickrequest" data-file="${esc(r.file)}" ${S.civicFile === r.file ? 'checked' : ''} ${r.verified ? '' : 'disabled'}>
        <strong class="mono">${esc(short(r.civic_id || r.file, 14, 6))}</strong> ${r.verified ? chip('Dispositivo genuíno, chave conferida', 'ok') : chip(r.error || 'não verificado', 'bad')}</label>`).join('') : '<p class="muted">Nenhum pedido em <code>dev/requests</code>. O cidadão gera o pedido na Suite (passo "Credencial cívica").</p>';
    const res = S.result ? `<article class="card"><div class="card__head"><h3>Proposta enviada</h3></div><p>Identificador <span class="mono">${esc(short(S.result.id, 12, 8))}</span>. Faltam as aprovações dos outros membros; depois da contestação, entregue o pacote na aba Credenciais.</p></article>` : '';
    const del = S.delivered ? callout('ok', 'Pacote entregue.', `O titular o instala na Suite: <code>${esc(S.delivered)}</code>`) : '';
    return `${head('ID cívico', 'Emitir o ID cívico', 'O pedido do cidadão prova que o dispositivo é genuíno e tem a chave de identidade. Você informa o que o órgão verificou; o quórum aprova; a credencial entra na chain e passa pela janela de contestação.')}
      <article class="card"><h3>Pedidos</h3>${reqs}</article>
      <article class="card"><div class="issue-form">
        <label class="field"><span>Nome completo</span><input class="input" id="c-name" value="${esc(f.name)}" autocomplete="off"></label>
        <label class="field"><span>Data de nascimento</span><input class="input" id="c-birth" type="date" value="${esc(f.birth)}"></label>
        <div class="field"><label for="c-nat">Nacionalidade</label>${combo('c-nat', geoCountries(), f.nat, 'Digite para buscar o país')}</div>
        <div class="field"><label for="c-uf">Domicílio eleitoral: estado</label>${combo('c-uf', geoStates(), f.uf, 'Digite para buscar o estado')}</div>
        <div class="field"><label for="c-mun">Domicílio eleitoral: município</label>${combo('c-mun', geoCities(f.uf), f.mun, 'Escolha o estado e busque o município')}</div>
        <label class="field"><span>Registro anterior (só numa REEMISSÃO: mantém o mesmo cidadão)</span><input class="input mono" id="c-reg" placeholder="TK-…" value="${esc(f.reg)}"></label>
        <label class="field"><span>Validade (dias, no máximo ${Math.floor(org.validity_secs / 86400)})</span><input class="input" id="c-days" type="number" min="1" value="${f.days}"></label>
        ${callout('info', 'Reemissão.', 'Se o cidadão perdeu o dispositivo: <strong>revogue</strong> a credencial antiga (aba Credenciais) e emita a nova informando o mesmo registro.')}
        <p><button class="btn" data-act="civicissue">Emitir no dispositivo</button></p>
      </div></article>${res}${del}`;
  }

  const REGIMES = [['comunhao-parcial', 'Comunhão parcial de bens'], ['comunhao-universal', 'Comunhão universal de bens'], ['separacao-total', 'Separação total de bens'], ['participacao-final', 'Participação final nos aquestos']];
  const ST = { proposed: 'proposta', pending: 'em contestação', active: 'válida', revoked: 'revogada', vetoed: 'vetada', expired: 'vencida', superseded: 'sucedida' };

  function vCivil() {
    const g = gate();
    const orgs = myOrgs();
    const ok = orgs.some((o) => o.ctype === 'certidao-casamento') && orgs.some((o) => o.ctype === 'certidao-divorcio');
    if (g || !ok) return `${head('Certidões', 'Casamento e divórcio', '')}${g || callout('info', 'Sem órgão.', 'Sua carteira não é membro dos órgãos de certidões (tipos <code>certidao-casamento</code> e <code>certidao-divorcio</code>).')}`;
    const m = S.marriageForm, d = S.divorceForm;
    const res = S.civilResult.length ? `<article class="card"><h3>Resultado</h3>${S.civilResult.map((r) => `<p>${r.error ? chip('erro', 'bad') : chip('proposta', 'ok')} <strong>${esc(r.nome)}</strong> ${r.error ? esc(r.error) : `<span class="mono muted">${esc(short(r.id, 10, 6))}</span>`}</p>`).join('')}</article>` : '';
    const open = S.marriages.filter((x) => x.active);
    const list = S.marriages.length ? `<article class="card"><h3>Casamentos deste cartório</h3>${S.marriages.map((x) => `<p><strong>${esc(x.matricula)}</strong> · ${esc(x.data)} · ${esc(x.regime)}<br>${x.spouses.map((sp) => `${esc(sp.nome)} ${chip(ST[sp.status] || sp.status, sp.status === 'active' ? 'ok' : sp.status === 'revoked' ? 'bad' : 'warn')}`).join(' · ')}</p>`).join('')}</article>` : '';
    return `${head('Certidões', 'Casamento e divórcio', 'São credenciais PRIVADAS: a chain guarda só um compromisso e cada titular guarda os dados. O casamento vale para os dois cônjuges (uma certidão para cada um); o divórcio encerra o casamento: as certidões de casamento são revogadas e a de divórcio fica como prova. Não vencem.')}
      <article class="card"><h3>Registrar casamento</h3><div class="issue-form">
        <label class="field"><span>Matrícula (a mesma nas duas certidões)</span><input class="input mono" id="m-mat" value="${esc(m.matricula)}"></label>
        <label class="field"><span>Data do casamento (AAAA-MM-DD)</span><input class="input" id="m-data" value="${esc(m.data)}" placeholder="2020-02-29"></label>
        <label class="field"><span>Regime de bens</span><select class="input" id="m-regime">${REGIMES.map(([k, l]) => `<option value="${k}" ${k === m.regime ? 'selected' : ''}>${l}</option>`).join('')}</select></label>
        <label class="field"><span>Cartório</span><input class="input" id="m-cart" value="${esc(m.cartorio)}"></label>
        <label class="field"><span>Cônjuge 1: nome</span><input class="input" id="m-an" value="${esc(m.an)}"></label>
        <label class="field"><span>Cônjuge 1: chave de credencial (hex)</span><input class="input mono" id="m-ak" value="${esc(m.ak)}" autocomplete="off" spellcheck="false"></label>
        <label class="field"><span>Cônjuge 2: nome</span><input class="input" id="m-bn" value="${esc(m.bn)}"></label>
        <label class="field"><span>Cônjuge 2: chave de credencial (hex)</span><input class="input mono" id="m-bk" value="${esc(m.bk)}" autocomplete="off" spellcheck="false"></label>
        ${callout('info', 'Duas propostas, duas confirmações no dispositivo.', 'Uma certidão para cada cônjuge, com a mesma matrícula e cada uma nomeando o outro. Cada titular recebe os dados dele em <code>dev/issued</code> (entrega fora da chain).')}
        <p><button class="btn" data-act="marriage">Propor no dispositivo</button></p>
      </div></article>
      <article class="card"><h3>Registrar divórcio</h3><div class="issue-form">
        <label class="field"><span>Casamento (só os que têm as duas certidões válidas)</span><select class="input" id="d-mat"><option value="">escolha…</option>${open.map((x) => `<option value="${esc(x.matricula)}" ${x.matricula === d.matricula ? 'selected' : ''}>${esc(x.matricula)} · ${esc(x.spouses.map((sp) => sp.nome).join(' e '))}</option>`).join('')}</select></label>
        <label class="field"><span>Data do divórcio (AAAA-MM-DD, depois do casamento)</span><input class="input" id="d-data" value="${esc(d.data)}" placeholder="2024-05-10"></label>
        <label class="field"><span>Forma</span><select class="input" id="d-forma">${[['extrajudicial', 'Extrajudicial (em cartório)'], ['judicial', 'Judicial']].map(([k, l]) => `<option value="${k}" ${k === d.forma ? 'selected' : ''}>${l}</option>`).join('')}</select></label>
        <label class="field"><span>Cartório</span><input class="input" id="d-cart" value="${esc(d.cartorio)}"></label>
        ${callout('warn', 'Encerra o casamento.', 'As certidões de divórcio são emitidas para os dois e, em seguida, o seu voto de <strong>revogação</strong> das certidões de casamento é dado. Os outros membros do órgão completam a revogação (em teste: <code>make devnet-cosign</code>).')}
        <p><button class="btn" data-act="divorce" ${open.length ? '' : 'disabled'}>Emitir divórcio no dispositivo</button></p>
      </div></article>${res}${list}`;
  }

  function vCand() {
    const g = gate();
    const org = myOrgs().find((o) => o.ctype === 'candidato');
    if (g || !org) return `${head('Candidaturas', 'Emitir candidaturas', '')}${g || callout('info', 'Sem órgão.', 'Sua carteira não é membro do órgão de candidaturas (tipo <code>candidato</code>).')}`;
    const f = S.candForm;
    const res = S.candResult.length ? `<article class="card"><h3>Resultado</h3>${S.candResult.map((r) => `<p>${r.error ? chip('erro', 'bad') : chip('proposta', 'ok')} <strong>${esc(r.numero)} · ${esc(r.nome)}</strong> ${r.error ? esc(r.error) : `<span class="mono muted">${esc(short(r.id, 10, 6))}</span>`}</p>`).join('')}</article>` : '';
    return `${head('Candidaturas', 'Emitir candidaturas', 'Cada candidatura é uma credencial pública na chain. Os candidatos da eleição são as candidaturas ATIVAS do cargo, da jurisdição e da eleição. A validade é até o fim da eleição mais os dias de apuração (no máximo ' + Math.floor(org.validity_secs / 86400) + ' dias).')}
      <article class="card"><div class="issue-form">
        <label class="field"><span>Eleição (id)</span><input class="input mono" id="k-eleicao" value="${esc(f.eleicao)}"></label>
        <label class="field"><span>Validade (dias a partir de agora)</span><input class="input" id="k-days" type="number" min="1" value="${f.days}"></label>
        <label class="field"><span>Cargo</span><select class="input" id="k-cargo">${['presidente', 'governador', 'senador', 'deputado-federal', 'deputado-estadual', 'prefeito', 'vereador'].map((c) => `<option ${c === f.cargo ? 'selected' : ''}>${c}</option>`).join('')}</select></label>
        <div class="field"><label for="k-uf">Estado (governador, senador e deputados)</label>${combo('k-uf', geoStates(), f.uf, 'Digite para buscar o estado')}</div>
        <div class="field"><label for="k-mun">Município (prefeito e vereador)</label>${combo('k-mun', geoCities(f.uf), f.mun, 'Escolha o estado e busque o município')}</div>
        <label class="field"><span>Número</span><input class="input" id="k-numero" value="${esc(f.numero)}"></label>
        <label class="field"><span>Nome</span><input class="input" id="k-nome" value="${esc(f.nome)}"></label>
        <label class="field"><span>Partido (2 dígitos; só deputados e vereador: o número do candidato começa por ele)</span><input class="input" id="k-partido" maxlength="2" inputmode="numeric" value="${esc(f.partido)}"></label>
        <label class="field"><span>Legenda do partido (nome ou sigla; só deputados e vereador)</span><input class="input" id="k-legenda" maxlength="40" value="${esc(f.legenda)}"></label>
        <label class="field"><span>Chave de credencial do candidato (hex)</span><input class="input mono" id="k-holder" value="${esc(f.holder)}" autocomplete="off" spellcheck="false"></label>
        <label class="field"><span>Lote (uma por linha: cargo|uf|município|número|nome|chave|partido|legenda)</span><textarea class="input mono" id="k-batch" rows="4" placeholder="governador|SP||13|Ana Souza|02ab…">${esc(f.batch)}</textarea></label>
        ${callout('info', 'Cada candidatura é um pedido no dispositivo.', 'O dispositivo mostra o cargo, os dados e a validade, e pede o botão para cada uma. Em teste: <code>make candidates</code> emite candidaturas de exemplo.')}
        <p><button class="btn" data-act="candissue">Propor no dispositivo</button></p>
      </div></article>${res}`;
  }

  function credRow(c, now) {
    const [label, kind] = STATUS[c.status] || [c.status, ''];
    const when = c.status === 'pending' ? ` · vale a partir de ${fmtDate(c.active_at)} (${fmtLeft(c.active_at, now)})`
      : c.status === 'active' ? ` · ${c.valid_until ? `até ${fmtDate(c.valid_until)}` : 'sem prazo'}` : '';
    const voting = S.voting && S.voting.id === c.id ? `<div class="fields"><label class="field"><span>Motivo público (${S.voting.op === 'veto' ? 'veto' : 'revogação'})</span>
        <input class="input" id="f-reason" maxlength="200" autocomplete="off"></label>
        <p><button class="btn btn--sm btn--danger" data-act="sendvote">Enviar ao dispositivo</button> <button class="btn btn--sm btn--ghost" data-act="cancelvote">Cancelar</button></p></div>` : '';
    return `<article class="card card--flat">
      <div class="card__head"><div><h3>${esc(c.ctype)} <span class="muted mono">${esc(short(c.id, 8, 6))}</span></h3></div>${chip(label, kind)}</div>
      <p class="muted">Titular <span class="mono">${esc(short(c.subject, 10, 6))}</span> · ${c.approvals}/${c.threshold} aprovações${esc(when)}</p>
      ${c.claims ? `<p class="mono">${esc(c.claims)}</p>` : '<p class="muted">Atributos com o titular (só o compromisso na chain).</p>'}
      ${c.reason ? `<p class="muted">Motivo: ${esc(c.reason)}</p>` : ''}
      <div class="row">
        ${c.can_approve ? `<button class="btn btn--sm" data-act="vote" data-op="approve" data-id="${c.id}" data-ctype="${esc(c.ctype)}">Aprovar</button>` : ''}
        ${c.can_veto ? `<button class="btn btn--sm btn--danger" data-act="vote" data-op="veto" data-id="${c.id}" data-ctype="${esc(c.ctype)}">Vetar</button>` : ''}
        ${c.can_revoke ? `<button class="btn btn--sm btn--danger" data-act="vote" data-op="revoke" data-id="${c.id}" data-ctype="${esc(c.ctype)}">Revogar</button>` : ''}
        ${c.ctype === 'civic' && c.claims && (c.status === 'pending' || c.status === 'active') ? `<button class="btn btn--sm" data-act="civicdeliver" data-id="${c.id}">Entregar o pacote ao titular</button>` : ''}
        ${c.approved_by_me && c.status === 'proposed' ? chip('Você já aprovou', 'ok') : ''}
      </div>${voting}</article>`;
  }

  function vCreds() {
    const g = gate();
    const ov = S.ov;
    if (!ov) return `${head('Credenciais', 'Credenciais na chain', '')}${g}<article class="card"><p class="muted">${esc(S.ovError || 'Carregando…')}</p></article>`;
    const all = ov.orgs.flatMap((o) => o.credentials);
    const filters = [['todas', 'Todas'], ['proposed', 'Aguardando'], ['pending', 'Em contestação'], ['active', 'Ativas'], ['ended', 'Encerradas']];
    const keep = (c) => S.filter === 'todas' || (S.filter === 'ended' ? ['expired', 'vetoed', 'revoked'].includes(c.status) : c.status === S.filter);
    const rows = all.filter(keep).sort((a, b) => b.active_at - a.active_at);
    return `${head('Credenciais', 'Credenciais na chain', 'O estado é calculado pelo horário dos blocos: contestação, validade e fim não precisam de nenhuma transação.')}${g}
      <div class="segmented" role="group">${filters.map(([id, l]) => `<button class="btn btn--sm ${S.filter === id ? '' : 'btn--ghost'}" data-act="filter" data-id="${id}">${l}</button>`).join('')}
        <button class="btn btn--sm btn--ghost" data-act="refresh">Atualizar</button></div>
      ${rows.length ? rows.map((c) => credRow(c, ov.now)).join('') : '<article class="card"><p class="muted">Nenhuma credencial neste filtro.</p></article>'}`;
  }

  // ───────────────────────── casca ─────────────────────────
  function renderDevice() {
    const d = S.dev;
    const ok = d && d.connected;
    $('#devcard').innerHTML = `<div class="devcard__row"><div><div class="devcard__title">${ok ? 'Dispositivo conectado' : 'Sem dispositivo'}</div>
      <div class="devcard__sub">${ok ? (d.status.locked ? 'bloqueado' : 'desbloqueado') : esc((d && d.error) || 'procurando…')}</div></div></div>
      <div class="devcard__chips">${ok ? chip(d.status.wallet_ready ? 'Carteira pronta' : 'Sem carteira', d.status.wallet_ready ? 'ok' : 'warn') : ''}</div>`;
    const items = [['orgs', 'Órgãos'], ['civic', 'ID cívico'], ['cand', 'Candidaturas'], ['civil', 'Certidões'], ['issue', 'Emitir'], ['creds', 'Credenciais']];
    $('#nav').innerHTML = items.map(([id, l]) => `<button class="nav__item" data-act="nav" data-id="${id}" ${S.route === id ? 'aria-current="page"' : ''}><span>${l}</span></button>`).join('');
    $('#addr').textContent = d && d.addr ? `emulador ${d.addr}` : '';
  }

  function renderOverlay() {
    const ov = $('#overlay');
    if (!S.busy) { ov.hidden = true; ov.innerHTML = ''; return; }
    const [t, b] = PROMPTS[S.prompt] || (S.prompt ? PROMPTS.other : ['Falando com o dispositivo…', 'Aguarde um instante.']);
    ov.hidden = false;
    ov.innerHTML = `<div class="overlay__card"><h2>${esc(t)}</h2><p class="muted">${esc(b)}</p>
      ${callout('info', 'O Emissor não confia neste computador.', 'Quem decide é o dispositivo: PIN e confirmações acontecem só nele.')}</div>`;
  }

  function render() {
    renderDevice();
    const views = { orgs: vOrgs, civic: vCivic, cand: vCand, civil: vCivil, issue: vIssue, creds: vCreds };
    const focus = document.activeElement && document.activeElement.id;
    $('#main').innerHTML = `<div class="view">${(views[S.route] || vOrgs)()}</div>`;
    if (focus) { const e = document.getElementById(focus); if (e) e.focus(); }
  }

  listen('device-prompt', (e) => { S.prompt = e.payload; renderOverlay(); });
  refresh();
  setInterval(async () => { if (!S.busy && !S.voting && document.activeElement.tagName !== 'INPUT' && document.activeElement.tagName !== 'TEXTAREA') { await refreshDevice(); await refreshOverview(); render(); } }, 5000);
})();
