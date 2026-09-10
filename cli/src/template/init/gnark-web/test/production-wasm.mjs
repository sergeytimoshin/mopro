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
const prepared = value(api.prepare(bytes('cubic', 'r1cs'), bytes('cubic', 'pk'), bytes('cubic', 'vk')));
const cubic = value(api.provePrepared(prepared, '{"X":"3","Y":"35"}'));

// Standard ToBinary hints must be available without importing the compiler.
const binary = value(api.prepare(bytes('binary', 'r1cs'), bytes('binary', 'pk'), bytes('binary', 'vk')));
for (const x of ['0', '3', '255']) {
  const proof = value(api.provePrepared(binary, JSON.stringify({ X: x })));
  assert.equal(value(api.verifyPrepared(binary, proof.proof, proof.public_inputs)), true);
}
assert.match(api.provePrepared(binary, '{"X":"256"}').error, /constraint/);

// Rejection must precede handle registration/proving and leave other handles usable.
for (const [circuit, pk, vk] of [['larger', 'cubic', null], ['cubic', 'larger', null], ['binary', null, 'cubic']]) {
  const result = api.prepare(bytes(circuit, 'r1cs'), pk && bytes(pk, 'pk'), vk && bytes(vk, 'vk'));
  assert.match(result.error, /key dimensions do not match/);
  assert.equal(result.value, undefined);
  assert.equal(value(api.verifyPrepared(prepared, cubic.proof, cubic.public_inputs)), true);
}
assert.match(api.prove(bytes('larger', 'r1cs'), bytes('cubic', 'pk'), '{"X":"1","Y":"1"}').error, /key dimensions/);
const after = value(api.provePrepared(prepared, '{"X":"2","Y":"15"}'));
assert.equal(value(api.verifyPrepared(prepared, after.proof, after.public_inputs)), true);
value(api.release(binary));
value(api.release(prepared));
assert.equal(exited, false);
console.log('Production Wasm: built-in hints, mismatched keys and runtime recovery passed');
process.exit(0);
