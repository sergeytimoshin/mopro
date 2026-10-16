import assert from 'node:assert/strict';
import { test, afterEach } from 'node:test';
import { initGnark, disposeGnark } from '../gnark.js';

class Worker {
    static instances = [];
    static throwOnPost = false;
    static stall = false;
    terminated = false;
    constructor() { Worker.instances.push(this); }
    postMessage(configuration) {
        this.configuration = configuration;
        if (Worker.throwOnPost) throw new Error('postMessage failed');
        if (Worker.stall) return;
        queueMicrotask(() => this.onmessage({ data: { id: 0, value: {
            threads: configuration.experimental ? (configuration.threads ?? 4) : 0,
        } } }));
    }
    terminate() { this.terminated = true; }
}
globalThis.Worker = Worker;
afterEach(() => { disposeGnark(); Worker.instances = []; Worker.throwOnPost = false; Worker.stall = false; });

test('default proving uses Go and never requests the experimental module', async () => {
    assert.equal((await initGnark()).threads, 0);
    assert.equal(Worker.instances[0].configuration.experimental, false);
    await assert.rejects(initGnark({ experimental: true }), /Dispose/);
});

test('positive worker counts require explicit opt-in before creating a runtime', async () => {
    await assert.rejects(initGnark({ threads: 2 }), /experimental: true/);
    await assert.rejects(initGnark({ experimental: 'yes' }), /boolean/);
    assert.equal(Worker.instances.length, 0);
    const first = initGnark({ experimental: true, threads: 2 });
    assert.equal(initGnark(), first);
    assert.equal((await first).threads, 2);
    await assert.rejects(initGnark({ experimental: false }), /Dispose/);
    assert.equal(Worker.instances.length, 1);
});

test('failed configuration is discarded and a later initialization can retry', async () => {
    Worker.throwOnPost = true;
    await assert.rejects(initGnark(), /postMessage/);
    assert.equal(Worker.instances[0].terminated, true);
    Worker.throwOnPost = false;
    assert.equal((await initGnark()).threads, 0);
    assert.equal(Worker.instances.length, 2);
});

test('fatal startup errors release the worker and preserve the ability to change settings', async () => {
    const pending = initGnark({ experimental: true, threads: 2 });
    Worker.instances[0].onmessage({ data: { fatal: true, error: 'Wasm failed' } });
    await assert.rejects(pending, /Wasm failed/);
    assert.equal((await initGnark()).threads, 0);
    assert.equal(Worker.instances.length, 2);
});


test('startup deadline terminates a stuck worker and rejects shared callers before recovery', async t => {
    t.mock.timers.enable({ apis: ['setTimeout'] });
    Worker.stall = true;
    const first = initGnark({ experimental: true, threads: 2, startupTimeoutMs: 50 });
    assert.equal(initGnark(), first);
    const rejected = assert.rejects(first, /startup timed out after 50 ms/);
    t.mock.timers.tick(50);
    await rejected;
    assert.equal(Worker.instances[0].terminated, true);
    Worker.stall = false;
    assert.equal((await initGnark()).threads, 0);
    t.mock.timers.tick(120_000);
    assert.equal(Worker.instances[1].terminated, false, 'successful startup must clear its deadline');
});

test('disposing startup clears its deadline before another runtime starts', async t => {
    t.mock.timers.enable({ apis: ['setTimeout'] });
    Worker.stall = true;
    const rejected = assert.rejects(initGnark({ startupTimeoutMs: 50 }), /disposed/);
    disposeGnark();
    await rejected;
    Worker.stall = false;
    await initGnark();
    t.mock.timers.tick(50);
    assert.equal(Worker.instances[1].terminated, false);
});

test('invalid startup deadlines reject before creating a worker', async () => {
    for (const startupTimeoutMs of [0, -1, 1.5, Infinity, NaN, 2147483648, '50']) {
        await assert.rejects(initGnark({ startupTimeoutMs }), /startupTimeoutMs/);
    }
    assert.equal(Worker.instances.length, 0);
});
