// Four independent field products at once. In WebAssembly with SIMD,
// mont29_simd.s (scripts/mont29_simd.py) runs ark-bn254's mont29
// multiplication on four lanes; elsewhere the lanes run one by one.
#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
use ark_bn254::{FqConfig, FrConfig};
use ark_bn254::{Fq, Fr};
#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
use ark_ff::MontConfig;
use ark_ff::{BigInt, BigInteger};

/// `a[i] * b[i]` in Fq for four lanes.
#[inline(always)]
pub(crate) fn fq_mul4(a: &[Fq; 4], b: &[Fq; 4]) -> [Fq; 4] {
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        lanes::run(
            lanes::mont29_mul4_fq,
            a.map(|v| v.0),
            b.map(|v| v.0),
            <FqConfig as MontConfig<4>>::MODULUS,
        )
        .map(Fq::new_unchecked)
    }
    #[cfg(not(all(target_arch = "wasm32", target_feature = "simd128")))]
    {
        core::array::from_fn(|i| a[i] * b[i])
    }
}

/// `a[i] * b[i]` in Fr for four lanes.
#[inline(always)]
pub(crate) fn fr_mul4(a: &[Fr; 4], b: &[Fr; 4]) -> [Fr; 4] {
    #[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
    {
        lanes::run(
            lanes::mont29_mul4_fr,
            a.map(|v| v.0),
            b.map(|v| v.0),
            <FrConfig as MontConfig<4>>::MODULUS,
        )
        .map(Fr::new_unchecked)
    }
    #[cfg(not(all(target_arch = "wasm32", target_feature = "simd128")))]
    {
        core::array::from_fn(|i| a[i] * b[i])
    }
}

// The 29-bit limbs of mont29.
#[allow(dead_code)]
#[inline(always)]
fn split(a: &BigInt<4>) -> [u32; 9] {
    const MASK: u64 = (1 << 29) - 1;
    let a = a.0;
    [
        a[0] & MASK,
        ((a[0] >> 29) | (a[1] << 35)) & MASK,
        ((a[0] >> 58) | (a[1] << 6)) & MASK,
        ((a[1] >> 23) | (a[2] << 41)) & MASK,
        ((a[1] >> 52) | (a[2] << 12)) & MASK,
        ((a[2] >> 17) | (a[3] << 47)) & MASK,
        ((a[2] >> 46) | (a[3] << 18)) & MASK,
        (a[3] >> 11) & MASK,
        (a[3] >> 40) & MASK,
    ]
    .map(|limb| limb as u32)
}

// mont29's final shift by 2^24 and conditional subtraction.
#[allow(dead_code)]
#[inline(always)]
fn join(t: [u64; 10], p: BigInt<4>) -> BigInt<4> {
    let mut out = BigInt([
        (t[0] >> 24) | (t[1] << 5) | (t[2] << 34) | (t[3] << 63),
        (t[3] >> 1) | (t[4] << 28) | (t[5] << 57),
        (t[5] >> 7) | (t[6] << 22) | (t[7] << 51),
        (t[7] >> 13) | (t[8] << 16) | (t[9] << 45),
    ]);
    if out >= p {
        out.sub_with_borrow(&p);
    }
    out
}

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
mod lanes {
    use super::{join, split};
    use ark_ff::BigInt;
    use core::arch::wasm32::*;

    core::arch::global_asm!(include_str!("mont29_simd.s"));
    // Arguments point at [v128; 9] (x, y) and [v128; 10] (out).
    extern "C" {
        pub(super) fn mont29_mul4_fq(x: *const u32, y: *const u32, out: *mut u32);
        pub(super) fn mont29_mul4_fr(x: *const u32, y: *const u32, out: *mut u32);
    }

    #[inline(always)]
    fn limbs(a: &[BigInt<4>; 4]) -> [v128; 9] {
        let l = [split(&a[0]), split(&a[1]), split(&a[2]), split(&a[3])];
        core::array::from_fn(|j| u32x4(l[0][j], l[1][j], l[2][j], l[3][j]))
    }

    #[inline(always)]
    pub(super) fn run(
        core: unsafe extern "C" fn(*const u32, *const u32, *mut u32),
        x: [BigInt<4>; 4],
        y: [BigInt<4>; 4],
        p: BigInt<4>,
    ) -> [BigInt<4>; 4] {
        let (x, y) = (limbs(&x), limbs(&y));
        let mut t = [u64x2_splat(0); 10];
        // The core reads nine and writes ten whole vectors through these.
        unsafe { core(x.as_ptr().cast(), y.as_ptr().cast(), t.as_mut_ptr().cast()) };
        let lane = |i: usize| -> [u64; 10] {
            core::array::from_fn(|j| {
                (match i {
                    0 => u32x4_extract_lane::<0>(t[j]),
                    1 => u32x4_extract_lane::<1>(t[j]),
                    2 => u32x4_extract_lane::<2>(t[j]),
                    _ => u32x4_extract_lane::<3>(t[j]),
                }) as u64
            })
        };
        [join(lane(0), p), join(lane(1), p), join(lane(2), p), join(lane(3), p)]
    }
}
