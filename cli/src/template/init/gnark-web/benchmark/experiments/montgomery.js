// Benchmark-only hybrid: montgomery 0.4.0 G1, Arkworks G2 and quotient FFT.
// Load only after isolation checks so ordinary pages can use the Go fallback.
let BN254, startThreads;
let rust;
let curve, fields, scalarMemory;

// Reuse released key/scratch blocks. The upstream allocator is a bump allocator;
// all pool members must perform the same allocations in the same order.
class Blocks {
    constructor(allocate) { this.allocate = allocate; this.available = []; }
    async take(size) {
        size = Math.ceil(size / 4) * 4;
        const i = this.available.findIndex(block => block.size >= size);
        return i < 0 ? { ptr: await this.allocate(size), size } : this.available.splice(i, 1)[0];
    }
    give(block) { if (block) this.available.push(block); }
}
class Points {
    constructor(raw) {
        if (raw.length % 64) throw new Error("Invalid G1 byte length");
        this.raw = raw.slice(); this.n = raw.length / 64;
    }
    async initialize() {
        if (!this.n) return;
        const bytes = rust.canonical_fields(this.raw, true);
        this.block = await fields.take(this.n * curve.Affine.size);
        const input = await fields.take(bytes.length);
        try {
            curve.Field.memoryBytes.set(bytes, input.ptr);
            await curve.Parallel.pointsFromBytes(this.block.ptr, input.ptr, this.n);
            // The bulk reader sets every nonzero flag. Preserve gnark infinity.
            for (let i = 0; i < this.n; i++) {
                if (this.raw.subarray(i * 64, (i + 1) * 64).every(x => x === 0)) {
                    curve.Affine.setIsNonZero(this.block.ptr + i * curve.Affine.size, false);
                }
            }
            this.raw = null;
        } finally { fields.give(input); }
    }
    free() { fields.give(this.block); this.block = null; this.raw = null; }
    async msm(rawScalars) {
        if (rawScalars.length !== this.n * 32) throw new Error("G1 scalar length mismatch");
        if (!this.n) return new Uint8Array(64);
        const bytes = rust.canonical_fields(rawScalars, false);
        const input = await scalarMemory.take(bytes.length);
        const scalars = await scalarMemory.take(this.n * curve.Scalar.sizeField);
        try {
            curve.Scalar.memoryBytes.set(bytes, input.ptr);
            await curve.Parallel.scalarsFromBytes(scalars.ptr, input.ptr, this.n);
            const offset = curve.Field.global.offset;
            const { result } = await curve.Parallel.msm(scalars.ptr, this.block.ptr, this.n);
            let point;
            const localOffset = curve.Field.local.offset;
            try {
                const scratch = curve.Field.local.getPointers(5);
                const affine = curve.Field.local.getPointer(curve.Affine.size);
                curve.Projective.toAffine(scratch, affine, result);
                point = curve.Affine.toBigint(affine);
            } finally { curve.Field.local.offset = localOffset; }
            // msm retains exactly its result; its internal scratch scopes unwind.
            // Check the pinned allocator contract before rewinding every worker.
            const retained = curve.Field.global.offset - offset;
            if (retained !== curve.Projective.size) throw new Error("montgomery allocation contract changed");
            await curve.Parallel.getPointer(-retained);
            const out = new Uint8Array(64);
            if (!point.isZero) {
                for (const [i, coordinate] of [point.x, point.y].entries()) {
                    let n = coordinate;
                    for (let j = 0; j < 32; j++, n >>= 8n) out[i * 32 + j] = Number(n & 255n);
                }
            }
            return rust.montgomery_fields(out, true);
        } finally { scalarMemory.give(scalars); scalarMemory.give(input); }
    }
}
export class Key {
    constructor(a, b, k, z, b2, params) {
        if (b.length / 64 !== b2.length / 128) throw new Error("G1/G2 dimensions differ");
        this.points = [a, b, k, z].map(bytes => new Points(bytes));
        this.rust = new rust.HybridKey(b2, z.length / 64 + 1, params);
        this.commitments = []; this.timings = {};
    }
    add_commitment(basis, sigma) {
        if (basis.length !== sigma.length) throw new Error("Commitment dimensions differ");
        this.commitments.push([new Points(basis), new Points(sigma)]);
    }
    async initialize() {
        for (const points of [...this.points, ...this.commitments.flat()]) await points.initialize();
    }
    async parts(sa, sb, sk, a, b, c) {
        const t = performance.now();
        const h = this.rust.quotient(a, b, c);
        const fft = performance.now() - t;
        const t2 = performance.now();
        const b2 = this.rust.g2(sb);
        const g2B = performance.now() - t2;
        const out = new Uint8Array(384);
        const names = ["g1A", "g1B", "g1K", "g1Z"];
        this.timings = { fft, g2B };
        for (const [i, values] of [sa, sb, sk, h].entries()) {
            const start = performance.now();
            out.set(await this.points[i].msm(values), i * 64);
            this.timings[names[i]] = performance.now() - start;
        }
        out.set(b2, 256);
        this.timings.arithmeticWall = performance.now() - t;
        return out;
    }
    async commitment(index, knowledge, values) {
        const points = this.commitments[index]?.[Number(knowledge)];
        if (!points) throw new Error("Unknown commitment key");
        return points.msm(values);
    }
    profile() { return JSON.stringify(this.timings); }
    free() {
        this.rust?.free(); this.rust = null;
        for (const points of [...this.points, ...this.commitments.flat()]) points.free();
        this.points = []; this.commitments = [];
    }
}
export async function initAccelerator(options) {
    const threads = options.experimental ? (options.threads ?? Math.min(16, navigator.hardwareConcurrency || 4)) : 0;
    if (!threads || !globalThis.crossOriginIsolated || typeof SharedArrayBuffer !== "function") return 0;
    ({ BN254, startThreads } = await import("montgomery"));
    rust = await import("./accelerator/gnark_kernel.js");
    await rust.default();
    await rust.initThreadPool(threads);
    curve = await BN254();
    await startThreads(threads);
    fields = new Blocks(size => curve.Parallel.getPointer(size));
    scalarMemory = new Blocks(size => curve.Parallel.getScalarPointer(size));
    // Retain dynamic Rayon exports through production bundlers.
    globalThis.__moproGnarkKernel = { Key, rust };
    globalThis.__moproBenchRuntime = { backend: "montgomery", pools: { rayon: threads, montgomery: threads - 1, coordinator: 1 } };
    return threads;
}
