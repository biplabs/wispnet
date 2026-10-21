import {
  pairingUsable,
  normalizedCode,
  activity,
  activeClients,
  serviceOperation,
  ordered,
  moveBefore,
  accessSettings,
  telemetry
} from './state.js';
const invoke = window.__TAURI__.core.invoke;
const $ = selector => document.querySelector(selector);
const content = $('#content'),
  dialog = $('#dialog'),
  form = $('#form'),
  body = $('#dialog-body');
let status = null,
  section = 'Services',
  busy = false,
  polling = false,
  lastCatalog = 0,
  qrTimer,
  dragging = null;
const desktop = payload => invoke('desktop_data', { payload });
let orders = {};
try {
  const saved = JSON.parse(localStorage.getItem('cardOrder') || '{}');
  if (saved && typeof saved === 'object' && !Array.isArray(saved)) orders = saved;
} catch {}
desktop({ op: 'preferences' })
  .then(p => {
    names = { ...p.names, ...names };
    orders = { ...p.order, ...orders };
    localStorage.setItem('peerNames', JSON.stringify(names));
    localStorage.setItem('cardOrder', JSON.stringify(orders));
    render();
  })
  .catch(() => {});
const peers = new Map();
let names = {};
try {
  const saved = JSON.parse(localStorage.getItem('peerNames') || '{}');
  if (saved && typeof saved === 'object' && !Array.isArray(saved)) names = saved;
} catch {
  /* A corrupt display preference must not prevent node access. */
}
invoke('legacy_peer_names')
  .then(legacy => {
    names = { ...legacy, ...names };
    localStorage.setItem('peerNames', JSON.stringify(names));
    render();
  })
  .catch(() => {});
const request = payload =>
  invoke('node_request', {
    payload
  });
const name = host =>
  host === status?.host.host
    ? status.host_name
    : names[host] || status?.host.clients.find(c => c.peer === host)?.name || host.slice(0, 12);

function el(tag, text, className) {
  const e = document.createElement(tag);
  if (text !== undefined) e.textContent = text;
  if (className) e.className = className;
  return e;
}

function button(text, action, className) {
  const e = el('button', text, className);
  e.type = 'button';
  e.disabled = busy;
  e.onclick = () => run(action);
  return e;
}

function notice(error) {
  $('#notice').textContent = String(error);
  $('#notice').hidden = false;
}
async function run(action) {
  if (busy) return;
  busy = true;
  $('#notice').hidden = true;
  document.querySelectorAll('button').forEach(b => (b.disabled = true));
  try {
    await action();
  } catch (e) {
    if (dialog.open) $('#dialog-error').textContent = String(e);
    else notice(e);
  } finally {
    busy = false;
    document.querySelectorAll('button').forEach(b => (b.disabled = false));
    render();
    await refresh();
  }
}

function closeDialog() {
  clearInterval(qrTimer);
  body.replaceChildren();
  form.onsubmit = null;
  dialog.close();
}
$('#cancel').onclick = closeDialog;
dialog.addEventListener('cancel', e => {
  e.preventDefault();
  if (!busy) closeDialog();
});

function modal(title, submit, action) {
  closeDialog();
  const heading = el('h2', title);
  heading.id = 'dialog-title';
  body.append(heading);
  $('#dialog-error').textContent = '';
  $('#submit').textContent = submit;
  $('#submit').hidden = !action;
  form.onsubmit = e => {
    e.preventDefault();
    run(action);
  };
  dialog.showModal();
}

function field(label, value = '', type = 'text') {
  const l = el('label', label),
    input = el(type === 'textarea' ? 'textarea' : 'input');
  if (type !== 'textarea') input.type = type;
  input.value = value;
  l.append(input);
  body.append(l);
  return input;
}

function checks(items, selected = new Set(), locked = new Set()) {
  const box = el('div', undefined, 'checks');
  body.append(box);
  for (const item of items) {
    const l = el('label'),
      input = el('input');
    input.type = 'checkbox';
    input.value = item.id;
    input.checked = selected.has(item.id) || locked.has(item.id);
    input.disabled = locked.has(item.id);
    l.append(input, document.createTextNode(item.title));
    box.append(l);
  }
  return () => [...box.querySelectorAll('input:checked')].map(i => i.value);
}

function confirmAction(title, detail, action) {
  modal(title, 'Confirm', async () => {
    await action();
    closeDialog();
  });
  body.append(el('p', detail));
}

function mutate(operation, revision) {
  return request({
    op: 'mutate',
    operation: {
      ...operation,
      expected_revision: revision
    }
  });
}

function serviceEditor(service) {
  const snapshot = status,
    revision = snapshot.host.revision;
  modal(service ? 'Edit service' : 'Share a service', 'Save service', async () => {
    const operation = serviceOperation(
      {
        id: id.value.trim(),
        name: title.value.trim(),
        backend: /^:?\d+$/.test(backend.value.trim())
          ? '127.0.0.1:' + backend.value.trim().replace(':', '')
          : backend.value.trim(),
        kind: kind.value
      },
      revision,
      allowed()
    );
    if (!operation.id || !operation.backend)
      throw Error('Enter a service ID and loopback backend.');
    if (!service && snapshot.host.services.some(s => s.id === operation.id))
      throw Error('This ID already exists. Edit that service to replace it.');
    await request({ op: 'mutate', operation });
    closeDialog();
  });
  const id = field('Service ID', service?.id);
  id.disabled = !!service;
  const title = field('Display name', service?.display_name),
    backend = field('Loopback address', service?.backend || '127.0.0.1:8080');
  const label = el('label', 'Type'),
    kind = el('select');
  for (const value of ['http', 'tcp']) {
    const o = el('option', value.toUpperCase());
    o.value = value;
    kind.append(o);
  }
  kind.value = service?.kind || 'http';
  label.append(kind);
  body.append(label);
  body.append(
    el(
      'p',
      'Only selected devices can access this loopback service. Saving replaces the complete allowlist.',
      'muted'
    )
  );
  const allowed = checks(
    snapshot.host.clients.map(c => ({
      id: c.peer,
      title: `${c.name} · ${c.peer.slice(0, 12)}`
    })),
    new Set(service?.allow || [])
  );
}

function approval(invitation) {
  const snapshot = status,
    existing = new Set(
      snapshot.host.services.filter(s => s.allow?.includes(invitation.peer)).map(s => s.id)
    );
  modal(`Approve ${invitation.name || 'device'}`, 'Approve device', async () => {
    await mutate(
      {
        op: 'approve_invitation',
        invitation: invitation.invitation,
        services: allowed()
      },
      snapshot.host.revision
    );
    closeDialog();
  });
  body.append(
    el('p', invitation.peer, 'identity'),
    el(
      'p',
      'Compare the full identity. Choose services this device may access. Existing grants are preserved; no selection pairs without adding access.',
      'muted'
    )
  );
  const allowed = checks(
    snapshot.host.services.map(s => ({
      id: s.id,
      title: s.display_name || s.id
    })),
    existing,
    existing
  );
}

function pairMachine() {
  let code, host;
  modal('Pair another machine', 'Preview identity', async () => {
    if (!host) {
      code = normalizedCode(input.value);
      const result = await request({
        op: 'preview_invitation',
        invitation: code
      });
      host = result.host;
      if (typeof host !== 'string') throw Error('Invalid invitation preview.');
      input.value = '';
      input.hidden = true;
      body.append(
        el('p', 'Compare this full identity with the other machine before requesting pairing.'),
        el('p', host, 'identity')
      );
      display.value = result.host_name || '';
      $('#submit').textContent = 'Request pairing';
    } else {
      await request({
        op: 'pair',
        host,
        invitation: code
      });
      names[host] = display.value.trim();
      localStorage.setItem('peerNames', JSON.stringify(names));
      peers.set(host, {
        error: 'Awaiting approval'
      });
      closeDialog();
      notice('Pairing requested. Approve it on the other machine, then refresh services.');
    }
  });
  const input = field('Pairing code', '', 'textarea'),
    display = field('Private display name (optional)');
}
async function pairThis() {
  const qr = await request({
    op: 'invite_qr'
  });
  modal('Pair with this machine', '', null);
  body.append(
    el(
      'p',
      'Scan the QR or copy the pairing code. Then approve the request here and choose allowed services.'
    ),
    el('p', qr.host, 'identity')
  );
  const area = el('div');
  body.append(area);
  const update = () => {
    area.replaceChildren();
    if (!pairingUsable(qr, status?.invitations || [])) {
      area.append(el('p', 'Code expired or request received. Close to review requests.'));
      clearInterval(qrTimer);
      qr.code = '';
      qr.matrix = [];
      return;
    }
    const n = qr.matrix.length;
    if (!n || n > 256 || qr.matrix.some(r => r.length !== n)) throw Error('Invalid pairing QR.');
    const canvas = el('canvas', undefined, 'qr');
    canvas.width = canvas.height = (n + 8) * 4;
    canvas.setAttribute('aria-label', 'Pairing QR code');
    const ctx = canvas.getContext('2d');
    ctx.fillStyle = 'white';
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    ctx.fillStyle = 'black';
    qr.matrix.forEach((r, y) =>
      r.forEach((v, x) => {
        if (v) ctx.fillRect((x + 4) * 4, (y + 4) * 4, 4, 4);
      })
    );
    const code = el('textarea');
    code.id = 'pair-code';
    code.readOnly = true;
    code.value = qr.code;
    code.setAttribute('aria-label', 'Pairing code');
    const copy = button('Copy pairing code', async () => {
      if (pairingUsable(qr, status?.invitations || []))
        await invoke('copy_pairing_code', { code: qr.code });
    });
    const remaining = Math.max(0, Math.floor(qr.expires_at - Date.now() / 1000));
    area.append(
      canvas,
      code,
      copy,
      el('p', `Expires in ${Math.floor(remaining / 60)}:${String(remaining % 60).padStart(2, '0')}`)
    );
  };
  update();
  qrTimer = setInterval(update, 1000);
}
async function openService(service, host) {
  document.body.classList.add('warp');
  try {
    await invoke('open_service', {
      host,
      service: service.id,
      incarnation: service.incarnation,
      hostName: name(host) || null,
      reduceMotion: matchMedia('(prefers-reduced-motion: reduce)').matches
    });
  } finally {
    document.body.classList.remove('warp');
  }
}

function serviceGrid(services, host, enabled = true) {
  const grid = el('div', undefined, 'grid');
  const group = 'services/' + host;
  services = ordered(services, orders[group] || [], s => s.id);
  for (const s of services) {
    const tile = button('', () => openService(s, host), 'tile');
    const top = el('div', undefined, 'tile-top');
    const kindBadge = el(
      'span',
      (s.kind || 'http').toUpperCase(),
      `badge badge-${s.kind || 'http'}`
    );
    top.append(kindBadge);
    if (!enabled) {
      top.append(el('span', 'Offline', 'badge badge-offline'));
    }
    const tileBody = el('div', undefined, 'tile-body');
    const title = el('h3', s.display_name || s.id);
    const desc = el(
      'div',
      s.kind === 'http' ? 'Protected localhost' : 'Network port tunnel',
      'tile-desc'
    );
    tileBody.append(title, desc);
    const action = el(
      'span',
      s.kind === 'http' ? '↗ Open in browser' : 'TCP · use the terminal client',
      'tile-action'
    );
    tile.append(top, tileBody, action);
    tile.disabled = busy || !enabled || s.kind !== 'http';
    const wrapper = el('div', undefined, 'service-card');
    wrapper.append(
      orderHandle(
        group,
        s.id,
        services.map(s => s.id)
      ),
      tile
    );
    dropTarget(wrapper, group, s.id);
    grid.append(wrapper);
  }
  if (!services.length) grid.append(el('p', 'No shared services.', 'muted'));
  return grid;
}

function renderConnection() {
  const conn = $('#connection');
  conn.replaceChildren();
  const statusRow = el('div', undefined, 'connection-status');
  const dot = el('span', '', `status-dot ${status ? 'online' : 'offline'}`);
  statusRow.append(dot, el('strong', status ? '● Online' : '○ Offline'));
  const relay = el('div', status ? `Relay: ${status.relay}` : 'Node unavailable', 'connection-relay');
  conn.append(statusRow, relay);
  if (status) {
    const metrics = el('div', undefined, 'connection-metrics');
    metrics.append(
      el('span', `${activeClients(status)} active`, 'metric-pill'),
      el('span', `${status.host.clients.length} approved`, 'metric-pill')
    );
    conn.append(metrics);
  }
}

function render() {
  if (dragging) return;
  $('#heading').textContent = section;
  $('#online').textContent = status ? 'Go offline' : 'Go online';
  renderConnection();
  document.querySelectorAll('nav button').forEach(b => {
    b.setAttribute('aria-current', b.dataset.section === section ? 'page' : 'false');
    if (b.dataset.section === 'Pairing requests') {
      const pendingCount = status ? status.invitations.filter(i => i.state === 'pending').length : 0;
      b.textContent = `Pairing requests${pendingCount ? ' · ' + pendingCount : ''}`;
    }
  });
  content.replaceChildren();
  if (!status) {
    if (section === 'Owner operations') {
      renderOperations();
      return;
    }
    const empty = el('div', undefined, 'empty');
    empty.append(
      el('h2', 'Your machine is offline'),
      el('p', 'Go online to see services and connect your devices.'),
      button(
        'Go online',
        () =>
          invoke('set_online', {
            online: true
          }),
        'primary'
      )
    );
    content.append(empty);
    for (const [host, peer] of peers) {
      if (!peer.catalog) continue;
      const panel = el('div', undefined, 'panel peer-panel');
      panel.append(
        el('h2', name(host)),
        el('p', 'Cached; node offline', 'muted'),
        serviceGrid(peer.catalog.services, host, false)
      );
      content.append(panel);
    }
    return;
  }
  if (section === 'Services') {
    const machine = el('div', undefined, 'panel machine-banner'),
      info = el('div', undefined, 'machine-info'),
      avatar = el('div', '💻', 'machine-avatar'),
      text = el('div');
    text.append(el('h2', status.host_name), el('span', 'Your services. Any device.', 'muted'));
    info.append(avatar, text);
    const actions = el('div', undefined, 'actions');
    actions.append(
      button('Access settings', accessEditor),
      button('Pair a machine', pairMachine),
      button('Pair this machine', pairThis, 'primary')
    );
    machine.append(info, actions);
    content.append(machine);

    content.append(
      el('h2', 'On this machine'),
      serviceGrid(status.host.services, status.host.host)
    );
    const machineIds = [...new Set([...status.peer_pins, ...status.host.clients.map(c => c.peer)])];
    for (const host of ordered(status.peer_pins, orders.machines || [])) {
      const peer = peers.get(host),
        panel = el('div', undefined, 'panel peer-panel'),
        row = el('div', undefined, 'peer-header'),
        titleGroup = el('div', undefined, 'peer-title-group'),
        tools = el('div', undefined, 'actions');
      const revision = status.host.revision;
      tools.append(
        button('Rename', () => renamePeer(host)),
        button('Refresh', () => refreshPeers([host])),
        button(
          'Forget…',
          () =>
            confirmAction(
              'Forget this machine?',
              `${host}\nIts owner still controls access granted to this machine.`,
              () =>
                request({
                  op: 'unpair',
                  host,
                  expected_revision: revision
                }).then(() => desktop({ op: 'cache_remove', host }))
            ),
          'danger'
        )
      );
      titleGroup.append(
        el('h2', name(host)),
        el('span', peer?.error ? 'Unavailable' : 'Live', peer?.error ? 'badge badge-error' : 'badge badge-online')
      );
      const leftGroup = el('div', undefined, 'peer-header-left');
      leftGroup.append(orderHandle('machines', host, machineIds), titleGroup);
      row.append(leftGroup, tools);
      panel.dataset.machine = host;
      dropTarget(panel, 'machines', host);
      panel.append(row);

      const meta = el('div', undefined, 'peer-meta');
      meta.append(el('p', host, 'identity'));
      meta.append(el('p', telemetry(status, host), 'muted badge-pill'));
      if (peer?.fetched)
        meta.append(
          el(
            'p',
            `Last refreshed ${Math.max(0, Math.floor(Date.now() / 1000 - peer.fetched))}s ago`,
            'muted badge-pill'
          )
        );
      panel.append(meta);

      if (peer?.error) {
        const errBox = el('div', undefined, 'card-alert error');
        errBox.append(el('p', peer.error));
        panel.append(errBox);
      }
      if (peer?.catalog)
        panel.append(serviceGrid(peer.catalog.services, host, peer.live && !peer.refreshing));
      else panel.append(el('p', 'Fetching services…', 'muted'));
      content.append(panel);
    }
    for (const c of status.host.clients.filter(c => !status.peer_pins.includes(c.peer))) {
      const panel = el('div', undefined, 'panel');
      const granted = status.host.services
        .filter(s => s.allow.includes(c.peer))
        .map(s => s.display_name || s.id);
      panel.append(
        orderHandle('machines', c.peer, machineIds),
        el('h2', c.name),
        el('p', telemetry(status, c.peer), 'muted'),
        el('p', granted.length ? 'Can access: ' + granted.join(', ') : 'No local services granted')
      );
      panel.dataset.machine = c.peer;
      dropTarget(panel, 'machines', c.peer);
      content.append(panel);
    }
    const cards = new Map(
      [...content.querySelectorAll('[data-machine]')].map(card => [card.dataset.machine, card])
    );
    for (const id of ordered(machineIds, orders.machines || [])) content.append(cards.get(id));
    if (!status.peer_pins.length && !status.host.clients.length)
      content.append(el('p', 'Pair another machine to see its shared services here.', 'muted'));
  } else if (section === 'My services') {
    const topBar = el('div', undefined, 'panel row');
    topBar.append(
      el('span', 'Share local loopback servers securely with permitted devices.', 'muted'),
      button('Share a service', () => serviceEditor(), 'primary')
    );
    content.append(topBar);

    for (const s of status.host.services) {
      const row = el('div', undefined, 'panel row'),
        info = el('div'),
        actions = el('div', undefined, 'actions');
      const titleRow = el('div', undefined, 'peer-title-group');
      titleRow.append(
        el('h3', s.display_name || s.id),
        el('span', (s.kind || 'http').toUpperCase(), `badge badge-${s.kind || 'http'}`),
        el('span', s.backend, 'identity')
      );
      info.append(titleRow);
      const revision = status.host.revision;
      actions.append(
        button('Edit', () => serviceEditor(s)),
        button(
          'Remove…',
          () =>
            confirmAction('Remove service?', s.display_name || s.id, () =>
              mutate(
                {
                  op: 'remove',
                  id: s.id
                },
                revision
              )
            ),
          'danger'
        )
      );
      if (s.kind === 'http') {
        const pub = status.public_access?.services?.find(p => p.service === s.id);
        actions.append(
          button(pub?.enabled ? 'Disable public access…' : 'Enable public access…', () =>
            confirmAction(
              'Change public access?',
              'Anyone with the public URL can connect while enabled. The service’s own login still applies.',
              () =>
                request({
                  op: 'configure_public',
                  service: s.id,
                  incarnation: s.incarnation,
                  enabled: !pub?.enabled,
                  name: status.host_name
                })
            )
          )
        );
        if (pub?.url) info.append(el('p', pub.url, 'identity'));
        if (pub?.enabled) info.append(el('p', 'Public · ' + pub.state, 'muted'));
      }
      row.append(info, actions);
      content.append(row);
    }
    if (!status.host.services.length)
      content.append(el('p', 'No local services shared yet. Click "Share a service" above to add one.', 'muted'));
  } else if (section === 'Devices') {
    const summary = el('div', undefined, 'panel row');
    summary.append(
      el(
        'p',
        `${status.host.clients.length} approved · ${activeClients(status)} active. Active means connected now or seen within the last 60 seconds.`,
        'muted'
      ),
      el('div', undefined, 'actions')
    );
    summary.querySelector('.actions').append(
      button('Pair a machine', pairMachine),
      button('Pair this machine', pairThis, 'primary')
    );
    content.append(summary);

    for (const c of status.host.clients) {
      const row = el('div', undefined, 'panel row'),
        info = el('div'),
        revision = status.host.revision;
      const deviceTitle = el('div', undefined, 'peer-title-group');
      const act = activity(status, c.peer);
      deviceTitle.append(
        el('h3', c.name),
        el('span', act, act === 'Connected' ? 'badge badge-online' : 'badge badge-offline')
      );
      info.append(
        deviceTitle,
        el('p', c.peer, 'identity')
      );
      row.append(
        info,
        button(
          'Revoke…',
          () =>
            confirmAction(
              `Revoke ${c.name}?`,
              `${c.peer}\nThis removes access to services on this machine.`,
              () =>
                mutate(
                  {
                    op: 'revoke',
                    peer: c.peer
                  },
                  revision
                )
            ),
          'danger'
        )
      );
      content.append(row);
    }
    const myId = el('div', undefined, 'panel');
    myId.append(el('h3', 'This machine’s identity'), el('p', status.host.host, 'identity'));
    content.append(myId);
  } else if (section === 'Owner operations') {
    renderOperations();
  } else {
    const pending = status.invitations.filter(i => i.state === 'pending');
    content.append(
      el(
        'p',
        'Compare the full device identity before approving. Each machine controls its own service permissions.',
        'muted'
      )
    );
    for (const i of pending) {
      const panel = el('div', undefined, 'panel'),
        actions = el('div', undefined, 'actions'),
        revision = status.host.revision;
      actions.append(
        button('Approve…', () => approval(i), 'primary'),
        button(
          'Reject…',
          () =>
            confirmAction('Reject request?', i.peer, () =>
              mutate(
                {
                  op: 'reject_invitation',
                  invitation: i.invitation
                },
                revision
              )
            ),
          'danger'
        )
      );
      panel.append(el('h3', i.name || 'New device'), el('p', i.peer, 'identity'), actions);
      content.append(panel);
    }
    if (!pending.length) content.append(el('p', 'No pending requests.', 'muted'));
  }
}
async function refresh() {
  if (polling || busy || dragging) return;
  polling = true;
  try {
    status = await request({
      op: 'status',
      refresh_clients: true
    });
    for (const host of status.peer_pins) {
      if (!peers.has(host)) {
        peers.set(host, { error: 'Fetching services…' });
        try {
          const cached = await desktop({ op: 'cache_load', host });
          if (cached)
            peers.set(host, {
              ...cached,
              live: false,
              error: 'Cached; refresh required'
            });
        } catch {}
      }
    }
    for (const host of peers.keys()) if (!status.peer_pins.includes(host)) peers.delete(host);
    if (Date.now() - lastCatalog > 15000) {
      lastCatalog = Date.now();
      void refreshPeers();
    }
  } catch (e) {
    status = null;
    for (const [host, peer] of peers)
      peers.set(host, { ...peer, live: false, error: 'Cached; node offline' });
  } finally {
    polling = false;
    render();
    void invoke('tray_status', {
      online: !!status,
      text: status
        ? `Wispnet · ${activeClients(status)} active · ${status.invitations.filter(i => i.state === 'pending').length} requests · Relay: ${status.relay}`
        : 'Wispnet · node offline'
    });
  }
}
async function refreshPeers(hosts = status?.peer_pins || []) {
  const queue = [...hosts];
  async function worker() {
    while (queue.length) {
      const host = queue.shift();
      if (peers.get(host)?.refreshing) continue;
      peers.set(host, {
        ...peers.get(host),
        refreshing: true
      });
      render();
      let result;
      try {
        const catalog = await request({
          op: 'catalog',
          host
        });
        if (catalog.host !== host) throw Error('Wrong catalog host.');
        result = { catalog, live: true, fetched: Date.now() / 1000 };
        try {
          await desktop({ op: 'cache_save', host, catalog });
        } catch (e) {
          notice('Could not save catalog: ' + e);
        }
      } catch (e) {
        result = {
          ...peers.get(host),
          refreshing: false,
          live: false,
          error: String(e)
        };
      }
      if (status?.peer_pins.includes(host)) peers.set(host, result);
      render();
    }
  }
  await Promise.all(
    Array.from(
      {
        length: Math.min(4, queue.length)
      },
      worker
    )
  );
}
function renamePeer(host) {
  modal('Rename machine', 'Save name', async () => {
    names[host] = input.value.trim();
    localStorage.setItem('peerNames', JSON.stringify(names));
    closeDialog();
  });
  const input = field('Private display name', name(host));
  input.maxLength = 128;
}

async function accessEditor() {
  const { settings } = await request({ op: 'gateway_settings' });
  modal('Access settings', 'Save settings', async () => {
    const updated = accessSettings(
      registry.value,
      origins.value,
      routes.map(r => ({
        host: r.host,
        service: r.service,
        port: r.input.value
      }))
    );
    await request({ op: 'configure_access', settings: updated });
    closeDialog();
  });
  const registry = field('Registry port', settings.registry_port, 'number');
  const origins = field(
    'Allowed websites (one exact origin per line)',
    settings.allowed_origins.join('\n'),
    'textarea'
  );
  const services = new Map();
  const add = (host, service, title) =>
    services.set(JSON.stringify([host, service]), { host, service, title });
  for (const s of status.host.services.filter(s => s.kind === 'http'))
    add(status.host.host, s.id, s.display_name || s.id);
  for (const [host, peer] of peers)
    for (const s of peer.catalog?.services || [])
      if (s.kind === 'http') add(host, s.id, name(host) + ' / ' + (s.display_name || s.id));
  for (const r of settings.service_ports)
    if (!services.has(JSON.stringify([r.host, r.service])))
      add(r.host, r.service, r.host.slice(0, 12) + ' / ' + r.service);
  const routes = [...services.values()].map(r => ({
    ...r,
    input: field(
      r.title + ' gateway port',
      settings.service_ports.find(p => p.host === r.host && p.service === r.service)?.port || '',
      'number'
    )
  }));
  body.append(
    el(
      'p',
      'Leave service ports blank for automatic assignment. Ports: 1024–65535. Saving reconnects active sessions.',
      'muted'
    )
  );
}

function orderHandle(group, key, ids) {
  const handle = el('button', '⠿', 'order-handle');
  handle.type = 'button';
  handle.dataset.orderGroup = group;
  handle.dataset.orderKey = key;
  handle.draggable = true;
  handle.setAttribute('aria-label', 'Reorder ' + key + '; use arrow keys to move');
  handle.addEventListener('dragstart', e => {
    dragging = { group, key, ids: ordered(ids, orders[group] || []) };
    e.dataTransfer.effectAllowed = 'move';
    e.dataTransfer.setData('text/plain', key);
    handle.parentElement.classList.add('dragging');
  });
  handle.addEventListener('dragend', finishDrag);
  handle.addEventListener('keydown', e => {
    if (!['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight'].includes(e.key)) return;
    e.preventDefault();
    const next = ordered(ids, orders[group] || []),
      index = next.indexOf(key);
    const target = index + (['ArrowUp', 'ArrowLeft'].includes(e.key) ? -1 : 1);
    if (target < 0 || target >= next.length) return;
    [next[index], next[target]] = [next[target], next[index]];
    orders[group] = next;
    localStorage.setItem('cardOrder', JSON.stringify(orders));
    render();
    [...document.querySelectorAll('.order-handle')]
      .find(h => h.dataset.orderGroup === group && h.dataset.orderKey === key)
      ?.focus();
  });
  return handle;
}
function dropTarget(element, group, key) {
  element.addEventListener('dragover', e => {
    if (dragging?.group !== group || dragging.key === key) return;
    e.preventDefault();
    e.stopPropagation();
    element.classList.add('drop-target');
  });
  element.addEventListener('dragleave', () => element.classList.remove('drop-target'));
  element.addEventListener('drop', e => {
    if (dragging?.group !== group) return;
    e.preventDefault();
    e.stopPropagation();
    orders[group] = moveBefore(dragging.ids, dragging.key, key);
    localStorage.setItem('cardOrder', JSON.stringify(orders));
    finishDrag();
  });
}
function finishDrag() {
  dragging = null;
  render();
}
document.addEventListener('keydown', e => {
  if (e.key === 'Escape' && dragging) finishDrag();
});

let operationSnapshot = [],
  loadingOperations = false;
async function loadOperations() {
  if (loadingOperations) return;
  loadingOperations = true;
  try {
    operationSnapshot = await desktop({ op: 'operations' });
  } finally {
    loadingOperations = false;
  }
}
function renderOperations() {
  const actions = el('div', undefined, 'actions');
  actions.append(
    button('Add client across hosts…', () => combinedEditor('approve')),
    button('Revoke across hosts…', () => combinedEditor('revoke')),
    button('Refresh operations', loadOperations)
  );
  content.append(
    actions,
    el(
      'p',
      'Each selected host applies independently using your configured SSH owner access. Adding a client grants no services automatically. Pending hosts can be retried; rejected hosts require a new decision.',
      'muted'
    )
  );
  for (const op of operationSnapshot) {
    const panel = el('div', undefined, 'panel');
    panel.append(
      el('h3', op.action.op + ' · ' + op.action.peer.slice(0, 12)),
      el('p', op.id, 'identity')
    );
    for (const t of op.targets)
      panel.append(
        el(
          'p',
          `${t.ssh || 'This host'} · ${t.host.slice(0, 12)} · ${t.state} · ${t.reason || ''}${t.revision == null ? '' : ' · revision ' + t.revision}`
        )
      );
    if (op.targets.some(t => t.state === 'pending'))
      panel.append(
        button('Retry pending hosts', async () => {
          await desktop({ op: 'retry', id: op.id });
          await loadOperations();
        })
      );
    content.append(panel);
  }
  if (!operationSnapshot.length) content.append(el('p', 'No saved owner operations.', 'muted'));
}
async function combinedEditor(op) {
  const targets = await desktop({ op: 'targets' });
  let reviewing = false,
    plan;
  modal(
    op === 'approve' ? 'Add client across hosts' : 'Revoke across hosts',
    'Review operation',
    async () => {
      if (!reviewing) {
        const action = { op, peer: peer.value.trim() };
        if (!/^[0-9a-f]{64}$/.test(action.peer)) throw Error('Enter a full client public ID.');
        if (op === 'approve') {
          action.name = label.value.trim();
          if (!action.name || action.name.length > 80)
            throw Error('Enter a client name (1–80 characters).');
        }
        const selected = new Set(allowed());
        plan = { action, targets: targets.filter(t => selected.has(t.host)) };
        if (!plan.targets.length) throw Error('Select at least one target host.');
        peer.disabled = true;
        if (label) label.disabled = true;
        body.querySelectorAll('input[type=checkbox]').forEach(i => (i.disabled = true));
        body.append(
          el(
            'p',
            `Apply ${op} for ${action.peer} independently on ${plan.targets.length} selected hosts? Offline hosts remain pending.`
          )
        );
        reviewing = true;
        $('#submit').textContent = 'Confirm operation';
      } else {
        await desktop({ op: 'start', plan });
        await loadOperations();
        closeDialog();
      }
    }
  );
  const peer = field('Client public ID');
  const label = op === 'approve' ? field('Client name') : null;
  const allowed = checks(
    targets.map(t => ({
      id: t.host,
      title: (t.ssh || 'This host') + ' · ' + t.host
    }))
  );
}
for (const title of [
  'Services',
  'My services',
  'Devices',
  'Pairing requests',
  'Owner operations'
]) {
  const b = button(title, async () => {
    section = title;
    if (section === 'Owner operations') await loadOperations();
    render();
  });
  b.dataset.section = title;
  $('nav').append(b);
}
$('#online').onclick = () =>
  run(() =>
    invoke('set_online', {
      online: !status
    })
  );
$('#refresh').onclick = () =>
  run(async () => {
    lastCatalog = 0;
    await refreshPeers();
  });
invoke('build_identity').then(value => ($('#version').textContent = value));
void refresh();
setInterval(refresh, 3000);
// A bounded starfield pauses with the webview and honors Reduce Motion.
const canvas = $('#stars'),
  ctx = canvas.getContext('2d'),
  stars = Array.from(
    {
      length: 96
    },
    () => ({
      x: Math.random(),
      y: Math.random(),
      r: Math.random() * 1.3 + 0.3
    })
  );
let previous = 0;

function draw(time) {
  if (!document.hidden) {
    const w = (canvas.width = innerWidth),
      h = (canvas.height = innerHeight);
    const nebula = ctx.createRadialGradient(w * 0.72, h * 0.35, 0, w * 0.72, h * 0.35, w * 0.7);
    nebula.addColorStop(0, '#102d50');
    nebula.addColorStop(1, '#080e19');
    ctx.fillStyle = nebula;
    ctx.fillRect(0, 0, w, h);
    const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches,
      delta = Math.min(time - previous, 40);
    for (const s of stars) {
      if (!reduced)
        s.x =
          (s.x +
            delta *
              (document.body.classList.contains('warp') ? 0.0005 : status ? 0.000012 : 0.000003)) %
          1;
      ctx.fillStyle = '#b1d7eb';
      ctx.fillRect(s.x * w, s.y * h, s.r, s.r);
    }
  }
  previous = time;
  requestAnimationFrame(draw);
}
requestAnimationFrame(draw);
