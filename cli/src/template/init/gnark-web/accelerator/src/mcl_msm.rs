//! Experimental mcl MSMs. Arkworks still owns FFT and the gnark wire format.
use ark_bn254::{Fq, Fq2, Fr, G1Affine, G1Projective, G2Affine, G2Projective};
use ark_ec::AdditiveGroup;
use ark_ff::{BigInteger, PrimeField};
use mcl_rust::{self as mcl, Fp, Fp2};
use rayon::prelude::*;
use std::sync::OnceLock;
pub type G1 = mcl::G1;
pub type G2 = mcl::G2;

pub fn init() -> bool {
    static READY: OnceLock<bool> = OnceLock::new();
    // Initialize the global curve before workers use it. SNARK is gnark BN254;
    // the mcl curve named BN254 has different parameters.
    *READY.get_or_init(|| {
        #[cfg(target_arch = "wasm32")]
        unsafe {
            extern "C" {
                fn __wasm_call_ctors();
            }
            // SAFETY: this private module owns mcl initialization. Rust's WASM
            // entry does not run the linked C++ constructors. Run them once,
            // before curve setup or any arithmetic can reach Rayon workers.
            __wasm_call_ctors();
        }
        mcl::init(mcl::CurveType::SNARK)
    })
}
fn fp(x: Fq) -> Fp {
    let mut out = Fp::zero();
    assert!(out.deserialize(&x.into_bigint().to_bytes_le()));
    out
}
fn fp2(x: Fq2) -> Fp2 {
    Fp2 {
        d: [fp(x.c0), fp(x.c1)],
    }
}
fn fq(x: &Fp) -> Fq {
    Fq::from_le_bytes_mod_order(&x.serialize())
}
fn fq2(x: &Fp2) -> Fq2 {
    Fq2::new(fq(&x.d[0]), fq(&x.d[1]))
}
pub fn g1(p: G1Affine) -> G1 {
    if p.infinity {
        G1::zero()
    } else {
        G1 {
            x: fp(p.x),
            y: fp(p.y),
            z: Fp::from_int(1),
        }
    }
}
pub fn g2(p: G2Affine) -> G2 {
    if p.infinity {
        G2::zero()
    } else {
        G2 {
            x: fp2(p.x),
            y: fp2(p.y),
            z: Fp2 {
                d: [Fp::from_int(1), Fp::zero()],
            },
        }
    }
}
pub fn scalars(values: &[Fr]) -> Vec<mcl::Fr> {
    values
        .par_iter()
        .map(|x| {
            let mut out = mcl::Fr::zero();
            // Canonical bytes avoid assumptions about private Montgomery layouts.
            assert!(out.deserialize(&x.into_bigint().to_bytes_le()));
            out
        })
        .collect()
}
extern "C" {
    // Upstream's Rust mul_vec accepts shared slices, but the C API may normalize
    // points in place. Bind these two operations with exclusive point buffers.
    fn mclBnG1_mulVec(out: *mut G1, bases: *mut G1, scalars: *const mcl::Fr, n: usize);
    fn mclBnG2_mulVec(out: *mut G2, bases: *mut G2, scalars: *const mcl::Fr, n: usize);
}
fn chunk_size(n: usize) -> usize {
    // Schedule complete upstream MSMs on the existing Rayon pool. Keep at least
    // 1024 bases in each chunk to amortize buckets; no native OpenMP is used.
    n.div_ceil(rayon::current_num_threads()).max(1024)
}
pub fn msm_g1(bases: &mut [G1], scalars: &[mcl::Fr]) -> G1Projective {
    assert_eq!(bases.len(), scalars.len());
    let chunk = chunk_size(bases.len());
    let p = bases
        .par_chunks_mut(chunk)
        .zip(scalars.par_chunks(chunk))
        .map(|(p, s)| {
            let mut out = G1::zero();
            // SAFETY: initialization has completed; lengths match; repr(C) types
            // come from the pinned dependency. Output and inputs do not alias, and
            // each worker exclusively owns its point chunk. No reinit can occur.
            unsafe { mclBnG1_mulVec(&mut out, p.as_mut_ptr(), s.as_ptr(), p.len()) };
            out
        })
        .reduce(G1::zero, |a, b| &a + &b);
    if p.is_zero() {
        G1Projective::ZERO
    } else {
        let mut q = G1::zero();
        G1::normalize(&mut q, &p);
        G1Affine::new_unchecked(fq(&q.x), fq(&q.y)).into()
    }
}
pub fn msm_g2(bases: &mut [G2], scalars: &[mcl::Fr]) -> G2Projective {
    assert_eq!(bases.len(), scalars.len());
    let chunk = chunk_size(bases.len());
    let p = bases
        .par_chunks_mut(chunk)
        .zip(scalars.par_chunks(chunk))
        .map(|(p, s)| {
            let mut out = G2::zero();
            // SAFETY: same initialization, dimensions, ABI and exclusive chunk
            // ownership as msm_g1 above.
            unsafe { mclBnG2_mulVec(&mut out, p.as_mut_ptr(), s.as_ptr(), p.len()) };
            out
        })
        .reduce(G2::zero, |a, b| &a + &b);
    if p.is_zero() {
        G2Projective::ZERO
    } else {
        let mut q = G2::zero();
        G2::normalize(&mut q, &p);
        G2Affine::new_unchecked(fq2(&q.x), fq2(&q.y)).into()
    }
}
