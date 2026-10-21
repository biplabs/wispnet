export function pairingUsable(qr, invitations, now = Date.now() / 1000) {
  return (
    now < qr.expires_at &&
    !invitations.some(
      i => i.invitation === qr.invitation && !['issued', 'challenged'].includes(i.state)
    )
  );
}
export function normalizedCode(input) {
  const code = input.replace(/\s/g, '');
  if (!code.startsWith('wispnet1:') || new TextEncoder().encode(code).length > 8192)
    throw Error('Enter a valid Wispnet pairing code.');
  return code;
}
export function activity(status, peer) {
  if ((status.connected_peers[peer] || 0) > 0) return 'Connected';
  if (status.recent_peers[peer] !== undefined)
    return `Seen ${Math.floor(status.recent_peers[peer])} seconds ago`;
  return 'Idle';
}
export function activeClients(status) {
  return status.host.clients.filter(c => activity(status, c.peer) !== 'Idle').length;
}
export function serviceOperation(draft, revision, allow) {
  return {
    op: 'service',
    id: draft.id,
    display_name: draft.name || draft.id,
    backend: draft.backend,
    kind: draft.kind,
    allow,
    expected_revision: revision
  };
}

export function ordered(values, saved = [], key = v => v) {
  if (!Array.isArray(saved)) saved = [];
  const ranks = new Map(saved.map((id, i) => [id, i]));
  return [...values].sort(
    (a, b) => (ranks.get(key(a)) ?? ranks.size) - (ranks.get(key(b)) ?? ranks.size)
  );
}
export function moveBefore(ids, source, target) {
  if (source === target || !ids.includes(source) || !ids.includes(target)) return ids;
  const next = ids.filter(id => id !== source);
  next.splice(next.indexOf(target), 0, source);
  return next;
}
export function accessSettings(registry, origins, routes) {
  const port = value => {
    if (!/^\d+$/.test(String(value).trim()) || +value < 1024 || +value > 65535)
      throw Error('Ports must be between 1024 and 65535.');
    return +value;
  };
  return {
    registry_port: port(registry),
    allowed_origins: origins
      .split(/\r?\n/)
      .map(s => s.trim())
      .filter(Boolean),
    service_ports: routes
      .filter(r => String(r.port).trim())
      .map(r => ({ ...r, port: port(r.port) }))
  };
}
export function telemetry(status, host) {
  const info = status?.peer_telemetry?.[host] || {};
  return [
    activity(status, host),
    ['direct', 'relay'].includes(info.path) ? info.path : '',
    info.rtt_ms > 0 ? `${info.rtt_ms}ms` : ''
  ]
    .filter(Boolean)
    .join(' · ');
}
