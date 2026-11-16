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
  };

  const PROMPTS = {
    credential_key: ['Confirme a CHAVE DE CREDENCIAL no dispositivo', 'Só a chave pública deste tipo será mostrada. Ela não se liga à sua identidade, carteira ou voto.'],
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
  async function refresh() { await refreshDevice(); await refreshOverview(); render(); }

  const unlocked = () => !!(S.dev && S.dev.connected && S.dev.status && !S.dev.status.locked);
  const myOrgs = () => (S.ov ? S.ov.orgs.filter((o) => o.is_member) : []);

  // ───────────────────────── ações ─────────────────────────
  const ACTIONS = {
    nav(el) { S.route = el.dataset.id; S.result = null; render(); },
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
    const items = [['orgs', 'Órgãos'], ['issue', 'Emitir'], ['creds', 'Credenciais']];
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
    const views = { orgs: vOrgs, issue: vIssue, creds: vCreds };
    const focus = document.activeElement && document.activeElement.id;
    $('#main').innerHTML = `<div class="view">${(views[S.route] || vOrgs)()}</div>`;
    if (focus) { const e = document.getElementById(focus); if (e) e.focus(); }
  }

  listen('device-prompt', (e) => { S.prompt = e.payload; renderOverlay(); });
  refresh();
  setInterval(async () => { if (!S.busy && !S.voting && document.activeElement.tagName !== 'INPUT' && document.activeElement.tagName !== 'TEXTAREA') { await refreshDevice(); await refreshOverview(); render(); } }, 5000);
})();
