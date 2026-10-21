import {
  pairingUsable,
  normalizedCode,
  activity,
  activeClients,
  serviceOperation
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
  qrTimer;
const peers = new Map();
let names = {};
try {
  const saved = JSON.parse(localStorage.getItem('peerNames') || '{}');
  if (saved && typeof saved === 'object' && !Array.isArray(saved)) names = saved;
} catch { /* A corrupt display preference must not prevent node access. */ }
invoke('legacy_peer_names').then(legacy => {
  names = {...legacy, ...names};
  localStorage.setItem('peerNames', JSON.stringify(names));
  render();
}).catch(() => {});
const request = payload => invoke('node_request', {
  payload
});
const name = host => host === status?.host.host ? status.host_name : names[host] || status?.host.clients.find(c => c.peer === host)?.name || host.slice(0, 12);

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
  document.querySelectorAll('button').forEach(b => b.disabled = true);
  try {
    await action();
  } catch (e) {
    if (dialog.open) $('#dialog-error').textContent = String(e);
    else notice(e);
  } finally {
    busy = false;
    document.querySelectorAll('button').forEach(b => b.disabled = false);
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
    const operation = serviceOperation({
      id: id.value.trim(),
      name: title.value.trim(),
      backend: backend.value.trim(),
      kind: kind.value
    }, revision, allowed());
    if (!operation.id || !operation.backend) throw Error('Enter a service ID and loopback backend.');
    await request({
      op: 'mutate',
      operation
    });
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
  body.append(el('p', 'Only selected devices can access this loopback service. Saving replaces the complete allowlist.', 'muted'));
  const allowed = checks(snapshot.host.clients.map(c => ({
    id: c.peer,
    title: `${c.name} · ${c.peer.slice(0,12)}`
  })), new Set(service?.allow || []));
}

function approval(invitation) {
  const snapshot = status,
    existing = new Set(snapshot.host.services.filter(s => s.allow?.includes(invitation.peer)).map(s => s.id));
  modal(`Approve ${invitation.name||'device'}`, 'Approve device', async () => {
    await mutate({
      op: 'approve_invitation',
      invitation: invitation.invitation,
      services: allowed()
    }, snapshot.host.revision);
    closeDialog();
  });
  body.append(el('p', invitation.peer, 'identity'), el('p', 'Compare the full identity. Choose services this device may access. Existing grants are preserved; no selection pairs without adding access.', 'muted'));
  const allowed = checks(snapshot.host.services.map(s => ({
    id: s.id,
    title: s.display_name || s.id
  })), existing, existing);
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
      body.append(el('p', 'Compare this full identity with the other machine before requesting pairing.'), el('p', host, 'identity'));
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
  modal('Pair with this Mac', '', null);
  body.append(el('p', 'Scan the QR or copy the pairing code. Then approve the request here and choose allowed services.'), el('p', qr.host, 'identity'));
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
    qr.matrix.forEach((r, y) => r.forEach((v, x) => {
      if (v) ctx.fillRect((x + 4) * 4, (y + 4) * 4, 4, 4);
    }));
    const code = el('textarea');
    code.id = 'pair-code';
    code.readOnly = true;
    code.value = qr.code;
    code.setAttribute('aria-label', 'Pairing code');
    const copy = button('Copy pairing code', async () => {
      if (pairingUsable(qr, status?.invitations || [])) await invoke('copy_pairing_code', {code:qr.code});
    });
    const remaining = Math.max(0, Math.floor(qr.expires_at - Date.now() / 1000));
    area.append(canvas, code, copy, el('p', `Expires in ${Math.floor(remaining/60)}:${String(remaining%60).padStart(2,'0')}`));
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
  for (const s of services) {
    const tile = button('', () => openService(s, host), 'tile');
    tile.append(el('h3', s.display_name || s.id), el('span', s.kind === 'http' ? '↗ Open in browser' : 'TCP · use the terminal client'));
    tile.disabled = busy || !enabled || s.kind !== 'http';
    grid.append(tile);
  }
  if (!services.length) grid.append(el('p', 'No shared services.', 'muted'));
  return grid;
}

function render() {
  $('#heading').textContent = section;
  $('#online').textContent = status ? 'Go offline' : 'Go online';
  $('#connection').replaceChildren(el('strong', status ? '● Online' : '○ Offline'), el('div', status ? `Relay: ${status.relay}` : 'Node unavailable'), el('small', status ? `${activeClients(status)} active · ${status.host.clients.length} approved` : ''));
  document.querySelectorAll('nav button').forEach(b => {
    b.setAttribute('aria-current', b.dataset.section === section ? 'page' : 'false');
    if (b.dataset.section === 'Pairing requests') b.textContent = `Pairing requests${status?' · '+status.invitations.filter(i=>i.state==='pending').length:''}`;
  });
  content.replaceChildren();
  if (!status) {
    const empty = el('div', undefined, 'empty');
    empty.append(el('h2', 'Your Mac is offline'), el('p', 'Go online to see services and connect your devices.'), button('Go online', () => invoke('set_online', {
      online: true
    }), 'primary'));
    content.append(empty);
    return;
  }
  const machine = el('div', undefined, 'panel machine'),
    info = el('div');
  info.append(el('h2', status.host_name), el('span', 'Your services. Any device.', 'muted'));
  const actions = el('div', undefined, 'actions');
  actions.append(button('Pair a machine', pairMachine), button('Pair this machine', pairThis, 'primary'));
  machine.append(info, actions);
  content.append(machine);
  if (section === 'Services') {
    content.append(el('h2', 'On this Mac'), serviceGrid(status.host.services, status.host.host));
    for (const host of status.peer_pins) {
      const peer = peers.get(host),
        panel = el('div', undefined, 'panel'),
        row = el('div', undefined, 'row'),
        tools = el('div', undefined, 'actions');
      const revision = status.host.revision;
      tools.append(button('Refresh', () => refreshPeers([host])), button('Forget…', () => confirmAction('Forget this machine?', `${host}\nIts owner still controls access granted to this Mac.`, () => request({
        op: 'unpair',
        host,
        expected_revision: revision
      })), 'danger'));
      row.append(el('h2', name(host)), tools);
      panel.append(row, el('p', host, 'identity'));
      if (peer?.error) panel.append(el('p', peer.error, 'muted'));
      else if (peer?.catalog) panel.append(serviceGrid(peer.catalog.services, host, !peer.refreshing));
      else panel.append(el('p', 'Fetching services…', 'muted'));
      content.append(panel);
    }
    if (!status.peer_pins.length) content.append(el('p', 'Pair another machine to see its shared services here.', 'muted'));
  } else if (section === 'My services') {
    content.append(button('Share a service', () => serviceEditor(), 'primary'));
    for (const s of status.host.services) {
      const row = el('div', undefined, 'panel row'),
        info = el('div'),
        actions = el('div', undefined, 'actions');
      info.append(el('h3', s.display_name || s.id), el('p', `${s.kind.toUpperCase()} · ${s.backend}`, 'muted'));
      const revision = status.host.revision;
      actions.append(button('Edit', () => serviceEditor(s)), button('Remove…', () => confirmAction('Remove service?', s.display_name || s.id, () => mutate({
        op: 'remove',
        id: s.id
      }, revision)), 'danger'));
      row.append(info, actions);
      content.append(row);
    }
  } else if (section === 'Devices') {
    content.append(el('p', `${status.host.clients.length} approved · ${activeClients(status)} active. Active means connected now or seen within the last 60 seconds.`, 'muted'));
    for (const c of status.host.clients) {
      const row = el('div', undefined, 'panel row'),
        info = el('div'),
        revision = status.host.revision;
      info.append(el('h3', c.name), el('p', c.peer, 'identity'), el('small', activity(status, c.peer)));
      row.append(info, button('Revoke…', () => confirmAction(`Revoke ${c.name}?`, `${c.peer}\nThis removes access to services on your Mac.`, () => mutate({
        op: 'revoke',
        peer: c.peer
      }, revision)), 'danger'));
      content.append(row);
    }
    content.append(el('h3', 'This Mac’s identity'), el('p', status.host.host, 'identity'));
  } else {
    const pending = status.invitations.filter(i => i.state === 'pending');
    content.append(el('p', 'Compare the full device identity before approving. Each machine controls its own service permissions.', 'muted'));
    for (const i of pending) {
      const panel = el('div', undefined, 'panel'),
        actions = el('div', undefined, 'actions'),
        revision = status.host.revision;
      actions.append(button('Approve…', () => approval(i), 'primary'), button('Reject…', () => confirmAction('Reject request?', i.peer, () => mutate({
        op: 'reject_invitation',
        invitation: i.invitation
      }, revision)), 'danger'));
      panel.append(el('h3', i.name || 'New device'), el('p', i.peer, 'identity'), actions);
      content.append(panel);
    }
    if (!pending.length) content.append(el('p', 'No pending requests.', 'muted'));
  }
}
async function refresh() {
  if (polling || busy) return;
  polling = true;
  try {
    status = await request({
      op: 'status',
      refresh_clients: true
    });
    for (const host of peers.keys())
      if (!status.peer_pins.includes(host)) peers.delete(host);
    if (Date.now() - lastCatalog > 15000) {
      lastCatalog = Date.now();
      void refreshPeers();
    }
  } catch (e) {
    status = null;
    peers.clear();
  } finally {
    polling = false;
    render();
    void invoke('tray_status', {
      online: !!status,
      text: status ? `Wispnet · ${activeClients(status)} active · ${status.invitations.filter(i=>i.state==='pending').length} requests · Relay: ${status.relay}` : 'Wispnet · node offline'
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
        result = {
          catalog
        };
      } catch (e) {
        result = {
          error: String(e)
        };
      }
      if (status?.peer_pins.includes(host)) peers.set(host, result);
      render();
    }
  }
  await Promise.all(Array.from({
    length: Math.min(4, queue.length)
  }, worker));
}
for (const title of ['Services', 'My services', 'Devices', 'Pairing requests']) {
  const b = button(title, () => {
    section = title;
    render();
  });
  b.dataset.section = title;
  $('nav').append(b);
}
$('#online').onclick = () => run(() => invoke('set_online', {
  online: !status
}));
$('#refresh').onclick = () => run(async () => {
  lastCatalog = 0;
  await refreshPeers();
});
invoke('build_identity').then(value => $('#version').textContent = value);
void refresh();
setInterval(refresh, 3000);
// A bounded starfield pauses with the webview and honors Reduce Motion.
const canvas = $('#stars'),
  ctx = canvas.getContext('2d'),
  stars = Array.from({
    length: 96
  }, () => ({
    x: Math.random(),
    y: Math.random(),
    r: Math.random() * 1.3 + .3
  }));
let previous = 0;

function draw(time) {
  if (!document.hidden) {
    const w = canvas.width = innerWidth,
      h = canvas.height = innerHeight;
    const nebula = ctx.createRadialGradient(w * .72, h * .35, 0, w * .72, h * .35, w * .7);
    nebula.addColorStop(0, '#102d50');
    nebula.addColorStop(1, '#080e19');
    ctx.fillStyle = nebula;
    ctx.fillRect(0, 0, w, h);
    const reduced = matchMedia('(prefers-reduced-motion: reduce)').matches,
      delta = Math.min(time - previous, 40);
    for (const s of stars) {
      if (!reduced) s.x = (s.x + delta * (document.body.classList.contains('warp') ? .0005 : status ? .000012 : .000003)) % 1;
      ctx.fillStyle = '#b1d7eb';
      ctx.fillRect(s.x * w, s.y * h, s.r, s.r);
    }
  }
  previous = time;
  requestAnimationFrame(draw);
}
requestAnimationFrame(draw);
