// Compatibility spike, NOT a deployable multi-isolate relay architecture.
import { blake3 } from '@noble/hashes/blake3';
const DOMAIN = 'iroh-relay handshake v1 challenge signature';
export const challengeMessage = challenge => blake3(challenge, { context: DOMAIN });
export async function authenticate(frame, challenge) {
  // postcard: fixed public key array, byte-string length varint, signature.
  if (frame.length !== 98 || frame[0] !== 1 || frame[33] !== 64) return null;
  const key = await crypto.subtle.importKey('raw', frame.slice(1,33), {name:'Ed25519'}, false, ['verify']);
  if (!await crypto.subtle.verify('Ed25519', key, frame.slice(34), challengeMessage(challenge))) return null;
  return frame.slice(1,33);
}
export function createRelay() {
let active = 0;
const peers = new Map(); // Deliberately isolate-local: the experiment must expose routing limitations.
const hex = bytes => Array.from(bytes, b => b.toString(16).padStart(2,'0')).join('');
return {
  async fetch(request) {
    const path = new URL(request.url).pathname;
    if (path === '/' || path === '/ping') return Response.json({experiment:'iroh-1.3-sites',routing:'isolate-local-unproven',durable_storage:false});
    if (path !== '/relay') return new Response('Not found', {status:404});
    const protocols = (request.headers.get('Sec-WebSocket-Protocol') || '').split(',').map(s=>s.trim());
    if (request.headers.get('Upgrade')?.toLowerCase() !== 'websocket' || !protocols.includes('iroh-relay-v2')) return new Response('WebSocket v2 required',{status:426});
    if (active >= 32) return new Response('Capacity',{status:503});
    const pair = new WebSocketPair(); const [client, socket] = Object.values(pair);
    socket.accept(); active++; socket.binaryType = "arraybuffer";
    const challenge = crypto.getRandomValues(new Uint8Array(16));
    let identity = null, identityHex = null, closed = false;
    let budget = 0, windowStart = Date.now();
    const cleanup = () => {
      if (closed) return; closed = true; active--; clearTimeout(deadline); clearInterval(heartbeat);
      if (identityHex && peers.get(identityHex) === socket) {
        peers.delete(identityHex);
        // Conservatively notify live peers; no durable reverse-route records.
        for (const other of peers.values()) { try { other.send(new Uint8Array([8,...identity])); } catch {} }
      }
    };
    const close = (reason='closed') => { console.log('relay-probe:'+reason); cleanup(); try {socket.close(1008,'Relay experiment closed');} catch {}};
    const deadline = setTimeout(close, 5000);
    const heartbeat = setInterval(()=>{try {socket.send(new Uint8Array([9,...crypto.getRandomValues(new Uint8Array(8))]));} catch {close();}},15000);
    // Serialise authentication and packets: event callbacks are otherwise concurrent.
    let chain = Promise.resolve();
    socket.addEventListener('message', event => {
      chain = chain.then(async()=>{
        if (closed) return;
        const data = event.data instanceof Blob ? await event.data.arrayBuffer() : event.data;
        if (!(data instanceof ArrayBuffer)) return close('binary-required');
        const frame = new Uint8Array(data);
        if (Date.now()-windowStart > 1000) {budget=0;windowStart=Date.now();}
        budget += frame.length;
        if (frame.length > 1024*1024 || budget > 4*1024*1024) return close();
        if (!identity) {
          identity = await authenticate(frame,challenge);
          if (!identity) return close('invalid-proof');
          identityHex = hex(identity); clearTimeout(deadline);
          const old = peers.get(identityHex);
          if (old) {try {old.send(new Uint8Array([13,1]));old.close(1000,'Replaced');}catch{}}
          peers.set(identityHex,socket);socket.send(new Uint8Array([2]));return;
        }
        if (frame[0] === 9 && frame.length === 9) {socket.send(new Uint8Array([10,...frame.slice(1)]));return;}
        if (frame[0] === 10 && frame.length === 9) return;
        if ((frame[0] !== 4 && frame[0] !== 5) || frame.length < (frame[0]===4?35:37)) return close();
        const destination = peers.get(hex(frame.slice(1,33)));
        if (!destination) {socket.send(new Uint8Array([8,...frame.slice(1,33)]));return;}
        const forwarded = frame.slice();forwarded[0] += 2;forwarded.set(identity,1);
        try {destination.send(forwarded);} catch {close();}
      }).catch(error=>close('handler-'+error.name));
    });
    socket.addEventListener('close',cleanup);socket.addEventListener('error',cleanup);
    socket.send(new Uint8Array([0,...challenge]));
    return new Response(null,{status:101,webSocket:client,headers:{'Sec-WebSocket-Protocol':'iroh-relay-v2'}});
  }
};

}
// Each object owns one ephemeral socket router. Never use object storage for
// packets, identities, catalog, permissions or pairing capabilities.
export class RelayRoom {
  constructor(_state, _env) { this.relay = createRelay(); }
  fetch(request) { return this.relay.fetch(request); }
}
const isolateRelay = createRelay();
export function hasRelayCoordinator(env) {
  return typeof env?.RELAY_ROOMS?.idFromName === 'function' &&
         typeof env?.RELAY_ROOMS?.get === 'function';
}
export default {
  async fetch(request, env) {
    const path = new URL(request.url).pathname;
    if (path === '/capabilities') return Response.json({
      experiment:'iroh-1.3-sites', durable_object_binding:hasRelayCoordinator(env),
      durable_storage:false
    });
    if (path === '/relay-durable') {
      if (!hasRelayCoordinator(env)) return Response.json({error:'durable_object_binding_unavailable'}, {status:503});
      // One fixed object, independent of peer identity and incoming request path.
      const room = env.RELAY_ROOMS.get(env.RELAY_ROOMS.idFromName('wispnet-relay-v1'));
      const target = new URL(request.url); target.pathname = '/relay';
      return room.fetch(new Request(target, request));
    }
    return isolateRelay.fetch(request);
  }
};
