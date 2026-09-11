import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';

const [wasm, shim, path] = process.argv.slice(2);
await import(pathToFileURL(shim).href);
const fixtures = JSON.parse(await readFile(path, 'utf8'));
const bytes = (fixture, extension) => new Uint8Array(Buffer.from(fixtures[fixture][extension], 'hex'));
const go = new globalThis.Go();
const { instance } = await WebAssembly.instantiate(await readFile(wasm), go.importObject);
let exited = false;
void go.run(instance).then(() => { exited = true; });
const api = globalThis.__moproGnark;
const value = result => { assert.ok(result); assert.equal(result.error, undefined); return result.value; };
const prepared = value((await api.prepare(bytes('cubic', 'r1cs'), bytes('cubic', 'pk'), bytes('cubic', 'vk'))));
const cubic = value((await api.provePrepared(prepared, '{"X":"3","Y":"35"}')));
assert.deepEqual(cubic.execution, { arithmetic: 'go', solver: 'go' });

// Proof commitment counts must be checked before decoder allocations. This
// previously grew the production WASM heap by 64 MiB for a 132-byte payload.
assert.equal(value((await api.verifyPrepared(prepared, cubic.proof, cubic.public_inputs))), true);
const beforeProofDecoding = instance.exports.mem.buffer.byteLength;
for (const count of [1_048_576, 0xffff_ffff]) {
  const malformed = Buffer.from(cubic.proof, 'hex');
  malformed.writeUInt32BE(count, 128);
  for (const data of [malformed, malformed.subarray(0, 132)]) {
    assert.match((await api.verifyPrepared(prepared, data.toString('hex'), cubic.public_inputs)).error, /commitment count/);
  }
}
assert(instance.exports.mem.buffer.byteLength - beforeProofDecoding < 8 * 1024 * 1024,
  'malformed proof triggered a large WASM allocation');
assert.equal(value((await api.verifyPrepared(prepared, cubic.proof, cubic.public_inputs))), true);

// Standard ToBinary hints must be available without importing the compiler.
const binary = value((await api.prepare(bytes('binary', 'r1cs'), bytes('binary', 'pk'), bytes('binary', 'vk'))));
for (const x of ['0', '3', '255']) {
  const proof = value((await api.provePrepared(binary, JSON.stringify({ X: x }))));
  assert.equal(value((await api.verifyPrepared(binary, proof.proof, proof.public_inputs))), true);
}
assert.match((await api.provePrepared(binary, '{"X":"256"}')).error, /constraint/);

// Rejection must precede handle registration/proving and leave other handles usable.
for (const [circuit, pk, vk] of [['larger', 'cubic', null], ['cubic', 'larger', null], ['binary', null, 'cubic']]) {
  const result = (await api.prepare(bytes(circuit, 'r1cs'), pk && bytes(pk, 'pk'), vk && bytes(vk, 'vk')));
  assert.match(result.error, /key dimensions do not match/);
  assert.equal(result.value, undefined);
  assert.equal(value((await api.verifyPrepared(prepared, cubic.proof, cubic.public_inputs))), true);
}
assert.match((await api.prove(bytes('larger', 'r1cs'), bytes('cubic', 'pk'), '{"X":"1","Y":"1"}')).error, /key dimensions/);
const after = value((await api.provePrepared(prepared, '{"X":"2","Y":"15"}')));
assert.equal(value((await api.verifyPrepared(prepared, after.proof, after.public_inputs))), true);
value((await api.release(binary)));
value((await api.release(prepared)));
assert.equal(exited, false);
console.log('Production Wasm: built-in hints, bounded proof decoding, mismatched keys and runtime recovery passed');
process.exit(0);
