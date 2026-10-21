export function pairingUsable(qr, invitations, now = Date.now() / 1000) {
  return now < qr.expires_at && !invitations.some(i => i.invitation === qr.invitation && !['issued', 'challenged'].includes(i.state));
}
export function normalizedCode(input) {
  const code = input.replace(/\s/g, '');
  if (!code.startsWith('wispnet1:') || new TextEncoder().encode(code).length > 8192) throw Error('Enter a valid Wispnet pairing code.');
  return code;
}
export function activity(status, peer) {
  if ((status.connected_peers[peer] || 0) > 0) return 'Connected';
  if (status.recent_peers[peer] !== undefined) return `Seen ${Math.floor(status.recent_peers[peer])} seconds ago`;
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
