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
