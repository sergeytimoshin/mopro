import "./wasm_exec.js";

try {
    const options = await new Promise((resolve) => {
        self.onmessage = ({ data }) => resolve(data);
    });
    if (!options?.configure || (options.threads !== undefined &&
        (!Number.isInteger(options.threads) || options.threads < 0 || options.threads > 64))) {
        throw new Error("Invalid gnark worker configuration");
    }
    const requestedThreads = options.threads ?? Math.min(16, navigator.hardwareConcurrency || 4);
    let threads = 0;
    if (requestedThreads > 0 && globalThis.crossOriginIsolated && typeof SharedArrayBuffer === "function") {
        const kernel = await import("./accelerator/gnark_kernel.js");
        await kernel.default();
        await kernel.initThreadPool(requestedThreads);
        globalThis.__moproGnarkKernel = kernel.Key;
        threads = requestedThreads;
    }
    const go = new globalThis.Go();
    const response = await fetch(new URL("./gnark.wasm", import.meta.url));
    if (!response.ok) throw new Error(`Cannot load gnark.wasm: HTTP ${response.status}`);
    // Byte instantiation also works on hosts that don't serve application/wasm.
    const { instance } = await WebAssembly.instantiate(await response.arrayBuffer(), go.importObject);
    go.run(instance).then(
        () => self.postMessage({ fatal: true, error: "Gnark runtime exited" }),
        (error) => self.postMessage({ fatal: true, error: String(error) }),
    );
    const api = globalThis.__moproGnark;
    if (!api) throw new Error("Gnark runtime did not register its bindings");
    const methods = new Set(["prove", "verify", "prepare", "provePrepared", "verifyPrepared", "release"]);
    self.onmessage = ({ data: { id, method, args } }) => {
        try {
            if (!methods.has(method)) throw new Error("Unknown gnark method");
            self.postMessage({ id, ...api[method](...args) });
        } catch (error) {
            self.postMessage({ id, error: String(error) });
        }
    };
    self.postMessage({ id: 0, value: { threads } });
} catch (error) {
    self.postMessage({ fatal: true, error: String(error) });
}
