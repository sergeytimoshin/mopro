// Selected at build time; only accelerated packages include this module.
export async function initAccelerator(options) {
    const requestedThreads = options.experimental ? (options.threads ?? Math.min(16, navigator.hardwareConcurrency || 4)) : 0;
    let threads = 0;
    if (requestedThreads > 0 && globalThis.crossOriginIsolated && typeof SharedArrayBuffer === "function") {
        const kernel = await import("./accelerator/gnark_kernel.js");
        await kernel.default();
        await kernel.initThreadPool(requestedThreads);
        // Go and Rayon read exports dynamically. Retain the whole namespace so
        // bundlers keep Rayon's wbg_rayon_start_worker export in this module.
        globalThis.__moproGnarkKernel = kernel;
        globalThis.__moproBenchRuntime = { backend: "arkworks", pools: { rayon: requestedThreads } };
        threads = requestedThreads;
    }
    return threads;
}
