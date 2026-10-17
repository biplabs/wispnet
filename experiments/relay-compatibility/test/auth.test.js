import {test} from 'node:test';
import assert from 'node:assert/strict';
import {authenticate,challengeMessage} from '../worker/index.js';
test('proof requires the signing key and exact nonce',async()=>{
 const keys=await crypto.subtle.generateKey({name:'Ed25519'},true,['sign','verify']);
 const nonce=crypto.getRandomValues(new Uint8Array(16));
 const key=new Uint8Array(await crypto.subtle.exportKey('raw',keys.publicKey));
 const signature=new Uint8Array(await crypto.subtle.sign('Ed25519',keys.privateKey,challengeMessage(nonce)));
 const frame=new Uint8Array([1,...key,64,...signature]);
 assert.deepEqual(await authenticate(frame,nonce),key);
 const wrong=nonce.slice();wrong[0]^=1;
 assert.equal(await authenticate(frame,wrong),null);
 frame[97]^=1;assert.equal(await authenticate(frame,nonce),null);
 assert.equal(await authenticate(frame.slice(1),nonce),null);
});

test('coordinator route fails closed without a namespace and never falls back',async()=>{
 const {default:worker,hasRelayCoordinator}=await import('../worker/index.js');
 const request=new Request('https://example.test/relay-durable');
 for (const env of [undefined,{}, {RELAY_ROOMS:'not-a-binding'},{RELAY_ROOMS:{get(){}}}]) {
  assert.equal(hasRelayCoordinator(env),false);
  const response=await worker.fetch(request,env);
  assert.equal(response.status,503);
  assert.deepEqual(await response.json(),{error:'durable_object_binding_unavailable'});
 }
});
test('separate requests select the same coordinator, not peer-specific objects',async()=>{
 const {default:worker}=await import('../worker/index.js');
 const selected=[];const forwarded=[];
 const env={RELAY_ROOMS:{idFromName(name){selected.push(name);return 'fixed-object';},get(id){
  assert.equal(id,'fixed-object');return {fetch(request){forwarded.push(request);return new Response('coordinated');}};
 }}};
 for(const value of ['first','second']) {
  const response=await worker.fetch(new Request('https://example.test/relay-durable',{headers:{'Sec-WebSocket-Protocol':'iroh-relay-v2','X-Test':value}}),env);
  assert.equal(await response.text(),'coordinated');
 }
 assert.deepEqual(selected,['wispnet-relay-v1','wispnet-relay-v1']);
 assert.deepEqual(forwarded.map(r=>new URL(r.url).pathname),['/relay','/relay']);
 assert.deepEqual(forwarded.map(r=>r.headers.get('X-Test')),['first','second']);
});
test('capabilities expose only fixed booleans and no environment secrets',async()=>{
 const {default:worker}=await import('../worker/index.js');
 const response=await worker.fetch(new Request('https://example.test/capabilities'),{SECRET:'must-not-leak'});
 assert.deepEqual(await response.json(),{experiment:'iroh-1.3-sites',durable_object_binding:false,durable_storage:false});
});
