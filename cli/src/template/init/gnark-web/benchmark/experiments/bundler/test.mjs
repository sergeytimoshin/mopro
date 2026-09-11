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
const buildOption = process.env.MOPRO_GNARK_ACCELERATOR || 'false';
assert(['false', 'true'].includes(buildOption), 'MOPRO_GNARK_ACCELERATOR must be false or true');
const accelerator = buildOption === 'true';
const ff = process.env.MOPRO_GNARK_BENCH_BACKEND === 'ffjavascript';
const localRequire = createRequire(path.join(web, 'package.json'));
const { Builder } = localRequire('selenium-webdriver');
const chrome = localRequire('selenium-webdriver/chrome');
const source = path.dirname(fileURLToPath(import.meta.url));
const temporary = await fs.mkdtemp(path.join(os.tmpdir(), 'mopro-vite-'));
const base = '/bundled-gnark/';
let driver, server;
let failThreads = false, kernelRequests = 0, injectedFailures = 0;
try {
    await fs.cp(path.join(web, 'MoproWasmBindings'), path.join(temporary, 'MoproWasmBindings'), { recursive: true });
    await fs.cp(path.join(web, 'assets/gnark-solver'), path.join(temporary, 'public/fixtures'), { recursive: true });
    for (const file of ['index.html', 'main.js']) await fs.copyFile(path.join(source, file), path.join(temporary, file));
    await build({
        root: temporary, configFile: false, base, logLevel: 'warn',
        worker: { format: 'es' },
    });
    const dist = path.join(temporary, 'dist');
    if (!accelerator) {
        const assets = await fs.readdir(dist, { recursive: true });
        assert(!assets.some(file => /gnark_kernel|workerHelpers/.test(file)), 'Go-only bundle includes Rust assets');
        assert.equal(assets.filter(file => file.endsWith('.wasm')).length, 1, 'Go-only bundle must contain only Go WASM');
    }
    server = http.createServer(async (request, response) => {
        const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
        const relative = pathname.slice(base.length) || 'index.html';
        const file = path.resolve(dist, relative);
        response.setHeader('Cross-Origin-Opener-Policy', 'same-origin');
        response.setHeader('Cross-Origin-Embedder-Policy', 'require-corp');
        response.setHeader('Cache-Control', 'no-store');
        try {
            assert(pathname.startsWith(base) && file.startsWith(dist + path.sep));
            // Rayon fetches its own module as a blob for each nested worker.
            // Let the kernel module load, then break those worker scripts.
            const kernelRequest = /gnark_kernel-[^/]+\.js$/.test(file);
            if (kernelRequest) kernelRequests++;
            if (failThreads && kernelRequest && kernelRequests > 1) {
                injectedFailures++;
                response.setHeader('Content-Type', 'text/javascript');
                response.end('throw new Error("Injected Rayon worker startup failure");');
                return;
            }
            let data = await fs.readFile(file);
            if (ff && failThreads && /gnark.worker[^/]*\.js$/.test(file)) {
                injectedFailures++;
                data = Buffer.concat([Buffer.from('globalThis.Worker = class { constructor() { throw new Error("Injected arithmetic worker startup failure"); } };\n'),data]);
            }
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

    failThreads = accelerator;
    const failure = await driver.executeAsyncScript(done => {
        globalThis.gnarkBundlerTest.initGnark({ experimental: true, threads: 2, startupTimeoutMs: 2000 })
            .then(() => done({ unexpectedSuccess: true }), error => done({ error: String(error) }));
    });
    if (accelerator) {
        assert(injectedFailures > 0, 'test did not reach nested worker startup: '+JSON.stringify(failure));
        assert.match(failure.error || '', /startup timed out|Injected Rayon worker|Injected arithmetic worker/);
    } else {
        assert.match(failure.error || '', /accelerator is not included in this build/);
        assert.equal(kernelRequests, 0);
    }
    failThreads = false;

    // Recovery must work on the same page, without a manual dispose after failure.
    for (const experimental of accelerator ? [false, true] : [false]) {
        const result = await driver.executeAsyncScript((experimental, done) => {
            (async () => {
                const api = globalThis.gnarkBundlerTest;
                try {
                    const info = await api.initGnark({ experimental, ...(experimental ? { threads: 2 } : {}), startupTimeoutMs: 10000 });
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
                    return { info, isolated: crossOriginIsolated, proofs };
                } finally { api.disposeGnark(); }
            })().then(done, error => done({ error: String(error) }));
        }, experimental);
        assert.equal(result.error, undefined, result.error);
        assert.equal(result.isolated, true);
        assert.equal(result.info.threads, experimental ? 2 : 0);
        const backend = experimental ? 'rust' : 'go';
        const execution = { arithmetic: backend, solver: "go" };
        for (const proof of result.proofs) assert.deepEqual(proof.execution, execution);
        await fs.writeFile(path.join(web, `gnark-vite-${backend}-benchmark.json`), JSON.stringify({ ...result, execution }, null, 2));
        console.log(`Vite production ${backend}: ${result.proofs.length} verified proofs with the expected backend`);
    }
    console.log(`Build accelerator=${accelerator}: startup rejection and recovery passed.`);
} finally {
    try { if (driver) await driver.quit(); }
    finally {
        if (server?.listening) await new Promise(resolve => server.close(resolve));
        await fs.rm(temporary, { recursive: true, force: true });
    }
}
