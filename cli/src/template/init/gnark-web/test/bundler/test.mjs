// npm ci && npm test -- /path/to/generated/web
import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { build } from 'vite';

assert(process.argv[2], 'provide the generated web directory containing packed bindings and gnark-solver fixtures');
const web = path.resolve(process.argv[2]);
const localRequire = createRequire(path.join(web, 'package.json'));
const { Builder } = localRequire('selenium-webdriver');
const chrome = localRequire('selenium-webdriver/chrome');
const source = path.dirname(fileURLToPath(import.meta.url));
const temporary = await fs.mkdtemp(path.join(os.tmpdir(), 'mopro-vite-'));
const base = '/bundled-gnark/';
let driver, server;
let stallWasm = true, wasmRequests = 0, isolated = true;
const stalledResponses = new Set();
try {
    await fs.cp(path.join(web, 'MoproWasmBindings'), path.join(temporary, 'MoproWasmBindings'), { recursive: true });
    await fs.cp(path.join(web, 'assets/gnark-solver'), path.join(temporary, 'public/fixtures'), { recursive: true });
    for (const file of ['index.html', 'main.js']) await fs.copyFile(path.join(source, file), path.join(temporary, file));
    await build({
        root: temporary, configFile: false, base, logLevel: 'warn',
        worker: { format: 'es' },
    });
    const dist = path.join(temporary, 'dist');
    const assets = await fs.readdir(dist, { recursive: true });
    assert.equal(assets.filter(file => file.endsWith('.wasm')).length, 1, 'Bundle must contain only the Go WASM');
    server = http.createServer(async (request, response) => {
        const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
        const relative = pathname.slice(base.length) || 'index.html';
        const file = path.resolve(dist, relative);
        if (isolated) {
            response.setHeader('Cross-Origin-Opener-Policy', 'same-origin');
            response.setHeader('Cross-Origin-Embedder-Policy', 'require-corp');
        }
        response.setHeader('Cache-Control', 'no-store');
        try {
            assert(pathname.startsWith(base) && file.startsWith(dist + path.sep));
            // Hold the download open to exercise the caller's startup deadline.
            if (stallWasm && file.endsWith('.wasm')) {
                wasmRequests++;
                stalledResponses.add(response);
                response.on('close', () => stalledResponses.delete(response));
                return;
            }
            const data = await fs.readFile(file);
            response.setHeader('Content-Type', {
                '.html': 'text/html', '.js': 'text/javascript',
                '.wasm': 'application/wasm', '.json': 'application/json',
            }[path.extname(file)] || 'application/octet-stream');
            response.end(data);
        } catch {
            response.writeHead(404);
            response.end();
        }
    });
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const options = new chrome.Options().addArguments('--headless', '--no-sandbox');
    if (process.env.CHROME_BIN) options.setChromeBinaryPath(process.env.CHROME_BIN);
    const builder = new Builder().forBrowser('chrome').setChromeOptions(options);
    if (process.env.CHROMEDRIVER_BIN) builder.setChromeService(new chrome.ServiceBuilder(process.env.CHROMEDRIVER_BIN));
    driver = await builder.build();
    await driver.manage().setTimeouts({ script: 30000 });
    await driver.get(`http://127.0.0.1:${server.address().port}${base}`);
    await driver.wait(() => driver.executeScript('return !!globalThis.gnarkBundlerTest'), 10000);

    const failure = await driver.executeAsyncScript(done => {
        globalThis.gnarkBundlerTest.initGnark({ startupTimeoutMs: 2000 })
            .then(() => done({ unexpectedSuccess: true }), error => done({ error: String(error) }));
    });
    assert(wasmRequests > 0, 'test did not reach the WASM download');
    assert.match(failure.error || '', /startup timed out/);
    stallWasm = false;
    for (const response of stalledResponses) response.destroy();

    // Recovery must work on the same page, without a manual dispose after failure.
    for (const isolation of [true, false]) {
        if (isolated !== isolation) {
            isolated = isolation;
            await driver.get(`http://127.0.0.1:${server.address().port}${base}`);
            await driver.wait(() => driver.executeScript('return !!globalThis.gnarkBundlerTest'), 10000);
        }
        const result = await driver.executeAsyncScript(done => {
            (async () => {
                const api = globalThis.gnarkBundlerTest;
                try {
                    await api.initGnark({ startupTimeoutMs: 10000 });
                    const load = async name => {
                        const response = await fetch(new URL('fixtures/' + name, location.href));
                        if (!response.ok) throw new Error(`Cannot load ${name}`);
                        return new Uint8Array(await response.arrayBuffer());
                    };
                    const fixture = await (await fetch(new URL('fixtures/fixture.json', location.href))).json();
                    const [r1cs, pk, vk] = await Promise.all(['circuit.r1cs', 'circuit.pk', 'circuit.vk'].map(load));
                    const circuit = await api.prepareGnarkCircuit(r1cs, { provingKey: pk, verifyingKey: vk });
                    const proofs = [];
                    for (let i = 0; i < 9; i++) {
                        const proof = await circuit.prove(fixture.inputs[i % fixture.inputs.length]);
                        if (!await circuit.verify(proof)) throw new Error('Bundled proof failed verification');
                        proofs.push(proof);
                    }
                    await circuit.dispose();
                    return { isolated: crossOriginIsolated, proofs };
                } finally { api.disposeGnark(); }
            })().then(done, error => done({ error: String(error) }));
        });
        assert.equal(result.error, undefined, result.error);
        assert.equal(result.isolated, isolation);
        const mode = isolation ? 'isolated' : 'portable';
        await fs.writeFile(path.join(web, `gnark-vite-${mode}-benchmark.json`), JSON.stringify(result, null, 2));
        console.log(`Vite production ${mode}: ${result.proofs.length} verified proofs`);
    }
    console.log('Startup deadline, recovery, and proving with and without isolation passed.');
} finally {
    try { if (driver) await driver.quit(); }
    finally {
        for (const response of stalledResponses) response.destroy();
        if (server?.listening) await new Promise(resolve => server.close(resolve));
        await fs.rm(temporary, { recursive: true, force: true });
    }
}
