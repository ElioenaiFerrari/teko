(() => {
  'use strict';
  const $ = (s) => document.querySelector(s);
  const main = $('#main');
  // Tudo que vem da API passa por esc() antes de virar HTML.
  const esc = (v) => String(v ?? '').replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));
  const short = (h, n = 10) => (h && h.length > 2 * n + 1 ? `${h.slice(0, n)}…${h.slice(-n)}` : h || '');
  const when = (ts) => (ts ? new Date(ts * 1000).toLocaleString('pt-BR') : '—');
  const amt = (a) => (a ? esc(a.text) : '—');
  const A = {
    addr: (a) => `<a class="mono" href="#/address/${encodeURIComponent(a)}">${esc(short(a, 12))}</a>`,
    block: (h) => `<a href="#/block/${encodeURIComponent(h)}">${esc(h)}</a>`,
    tx: (id) => `<a class="mono" href="#/tx/${encodeURIComponent(id)}">${esc(short(id, 8))}</a>`,
  };

  async function api(path) {
    const r = await fetch(`/api/v1/${path}`, { headers: { Accept: 'application/json' } });
    const body = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(body.error || `erro ${r.status}`);
    return body;
  }

  function describe(k) {
    switch (k.kind) {
      case 'transfer': return `Transferência de ${amt(k.amount)} para ${A.addr(k.to)}`;
      case 'stake': return `Travou ${amt(k.amount)} como stake`;
      case 'unstake': return `Pediu a saída de ${amt(k.amount)} do stake`;
      case 'claim': return 'Resgatou o stake liberado';
      case 'slash': return `Penalidade por equivocação${k.validator ? ` (validador <span class="mono">${esc(short(k.validator, 8))}</span>)` : ''}`;
      case 'slash_votes': return `Penalidade por equivocação na rede de VOTOS (caução da instituição${k.validator ? `, chave <span class="mono">${esc(short(k.validator, 8))}</span>` : ''})`;
      case 'faucet': return 'Moedas de TESTE (faucet da devnet)';
      case 'set_validator_key': return `Vinculou a chave de validador <span class="mono">${esc(short(k.key, 8))}</span>`;
      case 'assets': return `Bens: ${esc(({ register: 'registro', offer: 'oferta de venda', accept: 'aceite da oferta (preço em escrow)', cancel: 'oferta cancelada', finalize: 'transferência concluída', veto: 'VETO do órgão', add_lien: 'gravame', release_lien: 'gravame liberado', update_registrars: 'troca de registradores' })[k.op] || k.op)} · classe ${esc(k.class)}${k.asset ? ` · ${esc(k.asset)}` : ''}`;
      default: return `Módulo ${esc(k.module)} (${esc(k.payload_len)} bytes)`;
    }
  }

  const txTable = (txs) => (txs.length
    ? `<div class="table-wrap"><table><thead><tr><th>Transação</th><th>Bloco</th><th>De</th><th>O que faz</th><th>Taxa</th></tr></thead><tbody>${txs.map((t) =>
      `<tr><td>${A.tx(t.id)}</td><td>${A.block(t.height)}</td><td>${A.addr(t.sender)}</td><td>${describe(t.kind)}</td><td>${amt(t.fee)}</td></tr>`).join('')}</tbody></table></div>`
    : '<p class="muted">Nenhuma transação ainda.</p>');

  const blockTable = (bs) => (bs.length
    ? `<div class="table-wrap"><table><thead><tr><th>Bloco</th><th>Quando</th><th>Proponente (dono)</th><th>Txs</th><th>Recompensa</th></tr></thead><tbody>${bs.map((b) =>
      `<tr><td>${A.block(b.height)}</td><td>${esc(when(b.timestamp))}</td><td>${A.addr(b.proposer_owner)}</td><td>${esc(b.tx_count)}</td><td>${amt(b.reward)}</td></tr>`).join('')}</tbody></table></div>`
    : '<p class="muted">Nenhum bloco ainda.</p>');

  const card = (k, v) => `<div class="card"><div class="k">${esc(k)}</div><div class="v">${v}</div></div>`;
  const kv = (rows) => `<dl class="kv">${rows.map(([k, v]) => `<dt>${esc(k)}</dt><dd>${v}</dd>`).join('')}</dl>`;

  let last = null;
  async function refreshBadge() {
    try {
      last = await api('status');
      const m = last.meta;
      const bad = !!m.last_error;
      const up = m.nodes.filter((n) => n.up).length;
      const b = $('#badge');
      b.className = `badge${bad ? ' warn' : ''}`;
      b.textContent = bad
        ? `Atenção: ${m.last_error}`
        : `Cadeia verificada por este explorador até o bloco ${last.chain.height} · rede ${last.network} · ${up}/${m.nodes.length} nós no ar`;
    } catch (e) {
      const b = $('#badge'); b.className = 'badge warn'; b.textContent = `Explorador sem resposta: ${e.message}`;
    }
  }

  const views = {
    async home() {
      const [s, bs, ts] = await Promise.all([api('status'), api('blocks?limit=10'), api('txs?limit=10')]);
      const c = s.chain;
      const pct = c.founder_target.ypy ? Math.min(100, Math.floor((c.total_stake.ypy * 100) / c.founder_target.ypy)) : 0;
      const phase = c.founder_phase_active
        ? `<span class="chip">fase de fundador</span><progress class="bar" max="100" value="${pct}" aria-label="Stake até o fim da fase de fundador"></progress><small class="muted">${esc(c.total_stake.text)} de ${esc(c.founder_target.text)}</small>`
        : '<span class="chip ok">validadores por stake</span>';
      return `<h1>Visão geral · ${esc(c.chain_id)}</h1>
        <div class="grid">${card('Altura', esc(c.height))}${card('Criado (só recompensas)', amt(c.minted))}${card('Teto', amt(c.cap))}
        ${card('Próxima recompensa', amt(c.next_reward))}${card('Stake total', amt(c.total_stake))}${card('Queimado', amt(c.burned))}
        ${card('Validadores', esc(c.validators))}${card('Transações', esc(c.tx_total))}${card('Fase', phase)}</div>
        <h2>Últimos blocos</h2>${blockTable(bs)}<p><a href="#/blocks">ver todos os blocos</a></p>
        <h2>Últimas transações</h2>${txTable(ts)}<p><a href="#/txs">ver todas as transações</a></p>
        <p class="muted mono">raiz do estado: ${esc(c.state_root)}</p>`;
    },
    async blocks(arg, qs) {
      const before = qs.get('before');
      const bs = await api(`blocks?limit=25${before ? `&before=${encodeURIComponent(before)}` : ''}`);
      const old = bs.length ? bs[bs.length - 1].height : 0;
      return `<h1>Blocos</h1>${blockTable(bs)}${old > 1 ? `<p><a href="#/blocks?before=${old}">blocos mais antigos →</a></p>` : ''}`;
    },
    async block(id) {
      const b = await api(`blocks/${encodeURIComponent(id)}`);
      return `<h1>Bloco ${esc(b.height)}</h1>${kv([
        ['Hash', `<span class="mono">${esc(b.hash)}</span>`],
        ['Bloco anterior', b.height > 1 ? `${A.block(b.height - 1)} <span class="mono muted">${esc(short(b.prev_hash, 10))}</span>` : 'gênese'],
        ['Quando', esc(when(b.timestamp))],
        ['Proponente (chave de consenso)', `<span class="mono">${esc(short(b.proposer_key, 12))}</span>`],
        ['Dono (recebe a recompensa)', A.addr(b.proposer_owner)],
        ['Recompensa criada', amt(b.reward)], ['Taxas', amt(b.fees)],
        ['Certificado', `${esc(b.precommits)} precommit(s), rodada ${esc(b.round)}`],
        ['Raiz do estado', `<span class="mono">${esc(b.state_root)}</span>`]])}
        <h2>Transações (${esc(b.txs.length)})</h2>${txTable(b.txs)}`;
    },
    async txs() {
      return `<h1>Transações recentes</h1>${txTable(await api('txs?limit=50'))}`;
    },
    async tx(id) {
      const t = await api(`txs/${encodeURIComponent(id)}`);
      const ev = t.events.length
        ? `<h2>Eventos</h2><table><thead><tr><th>Módulo</th><th>Tópico</th><th>Dados</th></tr></thead><tbody>${t.events.map((e) => `<tr><td>${esc(e.module)}</td><td>${esc(e.topic)}</td><td class="mono">${esc(e.data_hex)}</td></tr>`).join('')}</tbody></table>` : '';
      return `<h1>Transação</h1>${kv([
        ['Id', `<span class="mono">${esc(t.id)}</span>`], ['Estado', '<span class="chip ok">executada</span>'],
        ['Bloco', A.block(t.height)], ['Quando', esc(when(t.timestamp))], ['De', A.addr(t.sender)],
        ['Módulo', esc(t.module)], ['O que faz', describe(t.kind)], ['Taxa', amt(t.fee)], ['Nonce', esc(t.nonce)]])}${ev}`;
    },
    async address(a) {
      const x = await api(`accounts/${encodeURIComponent(a)}`);
      const unb = x.unbonding.length ? x.unbonding.map((u) => `${amt(u.amount)} (libera no bloco ${esc(u.release_height)})`).join('<br>') : '—';
      const assets = x.assets.length
        ? `<h2>Bens</h2><table><thead><tr><th>Classe</th><th>Id</th><th>Versão</th><th>Gravames</th></tr></thead><tbody>${x.assets.map((s) => `<tr><td>${esc(s.class)}</td><td>${esc(s.id)}</td><td>${esc(s.version)}</td><td>${esc(s.liens.join(', ') || '—')}</td></tr>`).join('')}</tbody></table>` : '';
      return `<h1>Carteira</h1><p class="mono">${esc(x.address)}</p>${x.known ? '' : '<p class="chip bad">este endereço ainda não apareceu na cadeia</p>'}
        <div class="grid">${card('Saldo', amt(x.balance))}${card('Stake', amt(x.stake))}${card('Nonce', esc(x.nonce))}${card('Transações', esc(x.tx_count))}</div>
        <h2>Stake em saída</h2><p>${unb}</p>${assets}<h2>Transações recentes</h2>${txTable(x.recent_txs)}`;
    },
    async validators() {
      const [vs, s] = await Promise.all([api('validators'), api('status')]);
      const c = s.chain;
      return `<h1>Validadores</h1><p>${c.founder_phase_active ? `<span class="chip">fase de fundador</span> Só o fundador valida; a fase termina com ${esc(c.founder_target.text)} de stake total (hoje ${esc(c.total_stake.text)}).` : '<span class="chip ok">validadores por stake</span> O poder de voto é proporcional ao stake.'}</p>
        <div class="table-wrap"><table><thead><tr><th>Chave de consenso</th><th>Dono</th><th>Stake</th><th>Poder de voto</th></tr></thead><tbody>${vs.map((v) =>
          `<tr><td class="mono">${esc(short(v.key, 14))}</td><td>${A.addr(v.owner)}</td><td>${amt(v.stake)}</td><td>${esc(v.power)}</td></tr>`).join('')}</tbody></table></div>
        <h2>Nós da rede</h2>${kvNodes(s.meta.nodes)}`;
    },
    async assets() {
      const a = await api('assets');
      return `<h1>Bens</h1><h2>Classes</h2>${a.classes.length ? `<table><thead><tr><th>Classe</th><th>Órgão (M-de-N)</th><th>Versão</th></tr></thead><tbody>${a.classes.map((c) => `<tr><td>${esc(c.name)}</td><td>${esc(c.threshold)} de ${esc(c.registrars)}</td><td>${esc(c.version)}</td></tr>`).join('')}</tbody></table>` : '<p class="muted">Nenhuma classe.</p>'}
        <h2>Bens registrados</h2>${a.assets.length ? `<table><thead><tr><th>Classe</th><th>Id</th><th>Dono</th><th>Gravames</th></tr></thead><tbody>${a.assets.map((s) => `<tr><td>${esc(s.class)}</td><td><a href="#/asset/${encodeURIComponent(s.class)}/${encodeURIComponent(s.id)}">${esc(s.id)}</a></td><td>${A.addr(s.owner)}</td><td>${esc(s.liens.join(', ') || '—')}</td></tr>`).join('')}</tbody></table>` : '<p class="muted">Nenhum bem ainda.</p>'}`;
    },
    async asset(arg) {
      const [cls, ...rest] = arg.split('/');
      const id = rest.join('/');
      const d = await api(`assets/${encodeURIComponent(cls)}/${encodeURIComponent(id)}`);
      const a = d.asset;
      const meta = d.metadata_verified
        ? `<h2>Metadados <span class="chip ok">verificados contra a chain</span></h2>${kv(d.metadata.map(([k, v]) => [k, esc(v)]))}`
        : `<h2>Metadados</h2><p class="muted">${d.metadata_available ? 'O registro deste bem não está neste computador (ou não confere com o hash da chain).' : 'Este explorador não guarda os metadados: eles ficam fora da chain para proteger os dados do dono. Só o hash está na chain.'}</p>`;
      return `<h1>${esc(a.class)} · ${esc(a.id)}</h1>${kv([
        ['Dono', A.addr(a.owner)], ['Versão', esc(a.version)], ['Gravames', esc(a.liens.join(', ') || '—')],
        ['Hash dos metadados (na chain)', `<span class="mono">${esc(a.metadata_hash)}</span>`]])}${meta}`;
    },
    async votes() {
      const v = await api('votes');
      if (!v.available) return '<h1>Livro de votos</h1><p class="muted">Ainda sem dados do livro de votos. Suba a rede com <code>make devnet</code>.</p>';
      const rows = v.blocks.slice().reverse().slice(0, 50).map((b) =>
        `<tr><td>${esc(b.height)}</td><td>${esc(when(b.timestamp))}</td><td>${esc(b.entries)}</td><td>${esc(Object.entries(b.by_kind).map(([k, n]) => `${k}: ${n}`).join(', ') || '—')}</td><td>${esc(b.precommits)} de ${esc(v.validators)} (mín. ${esc(b.quorum)})</td><td>${esc(b.round)}</td><td class="mono">${esc(short(b.hash, 8))}</td></tr>`).join('');
      return `<h1>Livro de votos (BFT, ${esc(v.validators)} instituições)</h1>
        <p class="muted">Cada bloco é final com o certificado de commit de pelo menos ${esc(v.quorum)} das ${esc(v.validators)} instituições (mais de 2/3). Mostra só cabeçalhos e contagens: as cédulas são cifradas e o quadro com provas de inclusão fica na Suite. Altura ${esc(v.height)} · ${esc(v.chain_id)}.</p>
        ${rows ? `<div class="table-wrap"><table><thead><tr><th>Bloco</th><th>Quando</th><th>Entradas</th><th>Tipos</th><th>Certificado</th><th>Rodada</th><th>Hash</th></tr></thead><tbody>${rows}</tbody></table></div>` : '<p class="muted">Nenhum bloco de votos ainda.</p>'}`;
    },
    async events(arg, qs) {
      const topic = qs.get('topic');
      const ev = await api(`events?limit=50${topic ? `&topic=${encodeURIComponent(topic)}` : ''}`);
      return `<h1>Eventos${topic ? ` · ${esc(topic)}` : ''}</h1>${ev.length ? `<table><thead><tr><th>Bloco</th><th>Transação</th><th>Módulo</th><th>Tópico</th><th>Dados</th></tr></thead><tbody>${ev.map((e) => `<tr><td>${A.block(e.height)}</td><td>${A.tx(e.tx_id)}</td><td>${esc(e.module)}</td><td>${esc(e.topic)}</td><td class="mono">${esc(e.data_hex)}</td></tr>`).join('')}</tbody></table>` : '<p class="muted">Nenhum evento.</p>'}`;
    },
  };

  function kvNodes(nodes) {
    return `<table><thead><tr><th>Nó</th><th>Estado</th><th>Altura</th><th>Mempool</th><th>Valida?</th></tr></thead><tbody>${nodes.map((n) =>
      `<tr><td class="mono">${esc(n.addr)}</td><td>${n.up ? '<span class="chip ok">no ar</span>' : '<span class="chip bad">fora do ar</span>'}</td><td>${esc(n.height ?? '—')}</td><td>${esc(n.mempool ?? '—')}</td><td>${n.is_validator == null ? '—' : n.is_validator ? 'sim' : 'só replica'}</td></tr>`).join('')}</tbody></table>`;
  }

  async function route() {
    const [path, query = ''] = (location.hash.slice(1) || '/').split('?');
    const [, name = '', arg = ''] = path.split('/');
    const view = views[name || 'home'];
    document.querySelectorAll('.nav a').forEach((l) => l.removeAttribute('aria-current'));
    const cur = document.querySelector(`.nav a[href="#/${name === 'block' ? 'blocks' : name === 'tx' ? 'txs' : name === 'address' ? '' : name}"]`);
    if (cur) cur.setAttribute('aria-current', 'page');
    if (!view) { main.innerHTML = '<h1>Página não encontrada</h1>'; return; }
    try {
      main.innerHTML = await view(decodeURIComponent(arg), new URLSearchParams(query));
    } catch (e) {
      main.innerHTML = `<h1>Não encontrado</h1><p>${esc(e.message)}</p>`;
    }
  }

  $('#search').addEventListener('submit', async (ev) => {
    ev.preventDefault();
    const q = $('#q').value.trim();
    if (!q) return;
    try {
      const r = await api(`search?q=${encodeURIComponent(q)}`);
      const to = { block: `#/block/${r.height}`, tx: `#/tx/${r.id}`, account: `#/address/${r.address}`, validator: '#/validators' }[r.type];
      if (to) location.hash = to; else main.innerHTML = `<h1>Nada encontrado</h1><p>Nenhum bloco, transação, carteira ou validador para “${esc(q)}”.</p>`;
    } catch (e) { main.innerHTML = `<h1>Busca inválida</h1><p>${esc(e.message)}</p>`; }
  });

  window.addEventListener('hashchange', route);
  route();
  refreshBadge();
  setInterval(() => {
    refreshBadge();
    const h = location.hash;
    if (h === '' || h === '#/' || h.startsWith('#/blocks') || h === '#/txs') route();
  }, 3000);
})();
