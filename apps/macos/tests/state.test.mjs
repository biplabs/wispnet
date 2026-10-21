import test from 'node:test';
import assert from 'node:assert/strict';
import {
  pairingUsable,
  normalizedCode,
  activeClients,
  serviceOperation
} from '../ui/state.js';
test('pairing material expires and disappears on receipt of a request', () => {
  const qr = {
    invitation: 'secret',
    expires_at: 100
  };
  assert.equal(pairingUsable(qr, [], 99), true);
  assert.equal(pairingUsable(qr, [], 100), false);
  for (const state of ['pending', 'approved', 'rejected']) assert.equal(pairingUsable(qr, [{
    invitation: 'secret',
    state
  }], 99), false);
  for (const state of ['issued', 'challenged']) assert.equal(pairingUsable(qr, [{
    invitation: 'secret',
    state
  }], 99), true);
});
test('multiline pairing codes normalize without accepting arbitrary input', () => {
  assert.equal(normalizedCode(' wispnet1:abc\n def '), 'wispnet1:abcdef');
  assert.throws(() => normalizedCode('https://example.com'));
  assert.throws(() => normalizedCode('wispnet1:' + 'a'.repeat(8192)));
});
test('activity counts only approved devices once', () => {
  assert.equal(activeClients({
    host: {
      clients: [{
        peer: 'a'
      }, {
        peer: 'b'
      }, {
        peer: 'idle'
      }]
    },
    connected_peers: {
      a: 3,
      unapproved: 2
    },
    recent_peers: {
      a: 2,
      b: 59
    }
  }), 2);
});
test('service edits carry captured revision and replace the complete allowlist', () => {
  assert.deepEqual(serviceOperation({
    id: 'web',
    name: '',
    backend: '127.0.0.1:80',
    kind: 'http'
  }, 7, ['a']), {
    op: 'service',
    id: 'web',
    display_name: 'web',
    backend: '127.0.0.1:80',
    kind: 'http',
    allow: ['a'],
    expected_revision: 7
  });
});
