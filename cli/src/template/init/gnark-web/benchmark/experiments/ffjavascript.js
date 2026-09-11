// Benchmark-only adapter. ffjavascript/wasmcurves are GPL-3 dependencies.
// Gnark remains responsible for solving, commitments, blinding and proof encoding.
import { buildBn128 } from "ffjavascript";
let curve;
function rootExponent(root, base, log) {
    const F = curve.Fr;
    let rest = root, inversePower = F.inv(base), exponent = 0;
    for (let bit = 0; bit < log; bit++) {
        if (!F.eq(F.exp(rest, 1n << BigInt(log - bit - 1)), F.one)) {
            exponent += 2 ** bit;
            rest = F.mul(rest, inversePower);
        }
        inversePower = F.square(inversePower);
    }
    if (!(exponent & 1) || !F.eq(rest, F.one)) throw new Error("Invalid gnark FFT root");
    return exponent;
}
function inverseOdd(value, n) {
    let x = 1n;
    for (let bits = 1; bits < Math.log2(n); bits *= 2) x = x * (2n - BigInt(value) * x) % BigInt(n);
    return (x + BigInt(n)) % BigInt(n);
}
async function joinABC(a, b, c) {
    // Reuse wasmcurves' QAP operation; no field arithmetic implemented here.
    const tasks = [], chunk = Math.ceil(a.length / 32 / curve.tm.concurrency);
    for (let start = 0; start < a.length / 32; start += chunk) {
        const n = Math.min(chunk, a.length / 32 - start);
        tasks.push(curve.tm.queueAction([
            ...[a,b,c].map((bytes, i) => ({cmd:"ALLOCSET",var:i,buff:bytes.slice(start*32,(start+n)*32)})),
            {cmd:"ALLOC",var:3,len:n*32},
            {cmd:"CALL",fnName:"qap_joinABC",params:[{var:0},{var:1},{var:2},{val:n},{var:3}]},
            {cmd:"GET",out:0,var:3,len:n*32},
        ]));
    }
    const chunks = await Promise.all(tasks), out = new Uint8Array(a.length);
    let offset = 0;
    for (const [bytes] of chunks) { out.set(bytes, offset); offset += bytes.length; }
    return out;
}
export class Key {
    constructor(a,b,k,z,b2,params) {
        if ([a,b,k,z].some(x => x.length%64) || b2.length%128 || b.length/64 !== b2.length/128 || params.length !== 64) throw new Error("Invalid key dimensions");
        this.n = z.length/64+1;
        const log = Math.log2(this.n);
        if (!Number.isInteger(log) || log > curve.Fr.s || log < 1) throw new Error("Invalid FFT size");
        this.points = [a,b,k,z,b2].map(x => x.slice()); this.commitments = [];
        const F = curve.Fr;
        this.shift = params.slice(32); this.shiftInv = F.inv(this.shift);
        const denominator = F.sub(F.exp(this.shift, BigInt(this.n)), F.one);
        if (F.isZero(denominator)) throw new Error("Invalid quotient coset");
        this.den = F.inv(denominator);
        const exponent = rootExponent(params.slice(0,32), F.w[log], log);
        const inverse = inverseOdd(exponent, this.n);
        this.permutation = Uint32Array.from({length:this.n}, (_, i) => Number(BigInt(i)*inverse%BigInt(this.n)));
        this.reversal = Uint32Array.from({length:this.n}, (_, i) => {
            let out=0; for (let j=0;j<log;j++,i>>>=1) out=out*2+(i&1); return out;
        });
        this.timings = {};
    }
    add_commitment(basis,sigma) {
        if (basis.length%64 || basis.length!==sigma.length) throw new Error("Invalid commitment dimensions");
        this.commitments.push([basis.slice(),sigma.slice()]);
    }
    async quotient(a,b,c) {
        if (a.length !== b.length || a.length !== c.length || a.length%32 || a.length/32>this.n) throw new Error("Invalid quotient inputs");
        const F = curve.Fr;
        const transform = async values => {
            const padded = new Uint8Array(this.n*32);
            for (let i=0;i<this.n;i++) {
                const j=this.permutation[i];
                if (j*32<values.length) padded.set(values.subarray(j*32,(j+1)*32),i*32);
            }
            const coefficients=await F.ifft(padded);
            return F.fft(await F.batchApplyKey(coefficients,F.one,this.shift));
        };
        const transformed=await Promise.all([a,b,c].map(transform));
        const joined=await joinABC(...transformed);
        const coefficients=await F.batchApplyKey(await F.ifft(joined),this.den,this.shiftInv);
        const out=new Uint8Array((this.n-1)*32);
        for (let i=0;i<this.n-1;i++) out.set(coefficients.subarray(this.reversal[i]*32,(this.reversal[i]+1)*32),i*32);
        return out;
    }
    async parts(sa,sb,sk,a,b,c) {
        if ([sa,sb,sk].some((x,i)=>x.length!==this.points[i].length/2)) throw new Error("MSM dimensions differ");
        const start=performance.now();
        const h=await this.quotient(a,b,c), fft=performance.now()-start;
        const convert=performance.now();
        const scalars=await Promise.all([sa,sb,sk,h].map(x=>curve.Fr.batchFromMontgomery(x)));
        const conversion=performance.now()-convert;
        const t=performance.now(), names=["g1A","g1B","g1K","g1Z","g2B"];
        this.timings={fft,conversion};
        const values=await Promise.all([...scalars,scalars[1]].map(async (s,i)=>{
            const t=performance.now(), G=i===4?curve.G2:curve.G1;
            const point=await G.multiExpAffine(this.points[i],s);
            const bytes=G.toAffine(point);
            this.timings[names[i]]=performance.now()-t;
            // gnark uses (0,0) for infinity; ffjavascript has its own affine zero.
            return G.isZero(point)?new Uint8Array(i===4?128:64):bytes;
        }));
        this.timings.msmWall=performance.now()-t;
        this.timings.arithmeticWall=performance.now()-start;
        const out=new Uint8Array(384);let offset=0;
        for(const bytes of values){out.set(bytes,offset);offset+=bytes.length;}
        return out;
    }
    async commitment(index,knowledge,values) {
        const points=this.commitments[index]?.[Number(knowledge)];
        if(!points || points.length/2!==values.length) throw new Error("Invalid commitment witness");
        const scalars=await curve.Fr.batchFromMontgomery(values);
        const result=await curve.G1.multiExpAffine(points,scalars);
        return curve.G1.isZero(result)?new Uint8Array(64):curve.G1.toAffine(result);
    }
    profile(){return JSON.stringify(this.timings);}
    free(){this.points=[];this.commitments=[];this.permutation=null;this.reversal=null;}
}
export async function initAccelerator(options) {
    const threads=options.experimental?(options.threads??Math.min(16,navigator.hardwareConcurrency||4)):0;
    if(!threads || !globalThis.crossOriginIsolated || typeof SharedArrayBuffer!=="function")return 0;
    // The pinned library has no worker-count option. Override only this worker's
    // navigator during construction, restore it, then assert the actual pool.
    const original=Object.getOwnPropertyDescriptor(navigator,"hardwareConcurrency");
    Object.defineProperty(navigator,"hardwareConcurrency",{value:threads,configurable:true});
    try{curve=await buildBn128();}finally{
        if(original)Object.defineProperty(navigator,"hardwareConcurrency",original);
        else delete navigator.hardwareConcurrency;
    }
    if(curve.tm.concurrency!==threads)throw new Error("ffjavascript worker count mismatch");
    // Assert the raw Montgomery ABI before using Go's decoded arrays directly.
    for(const [F,p] of [[curve.Fr,curve.r],[curve.F1,curve.q]]){
        let one=(1n<<256n)%BigInt(p);const bytes=new Uint8Array(32);
        for(let i=0;i<32;i++,one>>=8n)bytes[i]=Number(one&255n);
        if(!F.eq(bytes,F.one))throw new Error("ffjavascript Montgomery representation changed");
    }
    globalThis.__moproGnarkKernel={Key};
    globalThis.__moproBenchRuntime={backend:"ffjavascript",pools:{ffjavascript:curve.tm.concurrency}};
    return threads;
}
