// Mixed-radix CIOS multiplication with R=2^256. Eight radix-2^29
// reductions followed by one radix-2^24 reduction preserve gnark's encoding.
// Inputs must be reduced modulo p. For M <= 2, each accumulator contains
// at most 27 products of 29-bit limbs plus carries, which fits in u64.
// Both BN254 moduli satisfy p < R/4. The unreduced result is below
// p + M*p^2/R < 2p, so one conditional subtraction produces a reduced result.
// Unrolled so LLVM can specialize the modulus and eliminate limb indexing.
use ark_ff::{BigInt, BigInteger};
#[inline(always)]
fn dot<const M: usize>(a: [BigInt<4>; M], b: [BigInt<4>; M], p: BigInt<4>, inv: u64) -> BigInt<4> {
    assert!(M == 1 || M == 2);
    const MASK: u64 = (1 << 29) - 1;
    let x0: [u64; M] = core::array::from_fn(|i| a[i].0[0] & MASK);
    let x1: [u64; M] = core::array::from_fn(|i| ((a[i].0[0] >> 29) | (a[i].0[1] << 35)) & MASK);
    let x2: [u64; M] = core::array::from_fn(|i| ((a[i].0[0] >> 58) | (a[i].0[1] << 6)) & MASK);
    let x3: [u64; M] = core::array::from_fn(|i| ((a[i].0[1] >> 23) | (a[i].0[2] << 41)) & MASK);
    let x4: [u64; M] = core::array::from_fn(|i| ((a[i].0[1] >> 52) | (a[i].0[2] << 12)) & MASK);
    let x5: [u64; M] = core::array::from_fn(|i| ((a[i].0[2] >> 17) | (a[i].0[3] << 47)) & MASK);
    let x6: [u64; M] = core::array::from_fn(|i| ((a[i].0[2] >> 46) | (a[i].0[3] << 18)) & MASK);
    let x7: [u64; M] = core::array::from_fn(|i| (a[i].0[3] >> 11) & MASK);
    let x8: [u64; M] = core::array::from_fn(|i| (a[i].0[3] >> 40) & MASK);
    let y0: [u64; M] = core::array::from_fn(|i| b[i].0[0] & MASK);
    let y1: [u64; M] = core::array::from_fn(|i| ((b[i].0[0] >> 29) | (b[i].0[1] << 35)) & MASK);
    let y2: [u64; M] = core::array::from_fn(|i| ((b[i].0[0] >> 58) | (b[i].0[1] << 6)) & MASK);
    let y3: [u64; M] = core::array::from_fn(|i| ((b[i].0[1] >> 23) | (b[i].0[2] << 41)) & MASK);
    let y4: [u64; M] = core::array::from_fn(|i| ((b[i].0[1] >> 52) | (b[i].0[2] << 12)) & MASK);
    let y5: [u64; M] = core::array::from_fn(|i| ((b[i].0[2] >> 17) | (b[i].0[3] << 47)) & MASK);
    let y6: [u64; M] = core::array::from_fn(|i| ((b[i].0[2] >> 46) | (b[i].0[3] << 18)) & MASK);
    let y7: [u64; M] = core::array::from_fn(|i| (b[i].0[3] >> 11) & MASK);
    let y8: [u64; M] = core::array::from_fn(|i| (b[i].0[3] >> 40) & MASK);
    let p0 = p.0[0] & MASK;
    let p1 = ((p.0[0] >> 29) | (p.0[1] << 35)) & MASK;
    let p2 = ((p.0[0] >> 58) | (p.0[1] << 6)) & MASK;
    let p3 = ((p.0[1] >> 23) | (p.0[2] << 41)) & MASK;
    let p4 = ((p.0[1] >> 52) | (p.0[2] << 12)) & MASK;
    let p5 = ((p.0[2] >> 17) | (p.0[3] << 47)) & MASK;
    let p6 = ((p.0[2] >> 46) | (p.0[3] << 18)) & MASK;
    let p7 = (p.0[3] >> 11) & MASK;
    let p8 = (p.0[3] >> 40) & MASK;
    let mut t0 = 0u64;
    let mut t1 = 0u64;
    let mut t2 = 0u64;
    let mut t3 = 0u64;
    let mut t4 = 0u64;
    let mut t5 = 0u64;
    let mut t6 = 0u64;
    let mut t7 = 0u64;
    let mut t8 = 0u64;
    let mut t9 = 0u64;
    for i in 0..M {
        t0 += x0[i] * y0[i];
    }
    for i in 0..M {
        t1 += x1[i] * y0[i];
    }
    for i in 0..M {
        t2 += x2[i] * y0[i];
    }
    for i in 0..M {
        t3 += x3[i] * y0[i];
    }
    for i in 0..M {
        t4 += x4[i] * y0[i];
    }
    for i in 0..M {
        t5 += x5[i] * y0[i];
    }
    for i in 0..M {
        t6 += x6[i] * y0[i];
    }
    for i in 0..M {
        t7 += x7[i] * y0[i];
    }
    for i in 0..M {
        t8 += x8[i] * y0[i];
    }
    let q = t0.wrapping_mul(inv) & MASK;
    t0 += q * p0;
    t1 += q * p1;
    t2 += q * p2;
    t3 += q * p3;
    t4 += q * p4;
    t5 += q * p5;
    t6 += q * p6;
    t7 += q * p7;
    t8 += q * p8;
    t1 += t0 >> 29;
    t0 = t1;
    t1 = t2;
    t2 = t3;
    t3 = t4;
    t4 = t5;
    t5 = t6;
    t6 = t7;
    t7 = t8;
    t8 = 0;
    for i in 0..M {
        t0 += x0[i] * y1[i];
    }
    for i in 0..M {
        t1 += x1[i] * y1[i];
    }
    for i in 0..M {
        t2 += x2[i] * y1[i];
    }
    for i in 0..M {
        t3 += x3[i] * y1[i];
    }
    for i in 0..M {
        t4 += x4[i] * y1[i];
    }
    for i in 0..M {
        t5 += x5[i] * y1[i];
    }
    for i in 0..M {
        t6 += x6[i] * y1[i];
    }
    for i in 0..M {
        t7 += x7[i] * y1[i];
    }
    for i in 0..M {
        t8 += x8[i] * y1[i];
    }
    let q = t0.wrapping_mul(inv) & MASK;
    t0 += q * p0;
    t1 += q * p1;
    t2 += q * p2;
    t3 += q * p3;
    t4 += q * p4;
    t5 += q * p5;
    t6 += q * p6;
    t7 += q * p7;
    t8 += q * p8;
    t1 += t0 >> 29;
    t0 = t1;
    t1 = t2;
    t2 = t3;
    t3 = t4;
    t4 = t5;
    t5 = t6;
    t6 = t7;
    t7 = t8;
    t8 = 0;
    for i in 0..M {
        t0 += x0[i] * y2[i];
    }
    for i in 0..M {
        t1 += x1[i] * y2[i];
    }
    for i in 0..M {
        t2 += x2[i] * y2[i];
    }
    for i in 0..M {
        t3 += x3[i] * y2[i];
    }
    for i in 0..M {
        t4 += x4[i] * y2[i];
    }
    for i in 0..M {
        t5 += x5[i] * y2[i];
    }
    for i in 0..M {
        t6 += x6[i] * y2[i];
    }
    for i in 0..M {
        t7 += x7[i] * y2[i];
    }
    for i in 0..M {
        t8 += x8[i] * y2[i];
    }
    let q = t0.wrapping_mul(inv) & MASK;
    t0 += q * p0;
    t1 += q * p1;
    t2 += q * p2;
    t3 += q * p3;
    t4 += q * p4;
    t5 += q * p5;
    t6 += q * p6;
    t7 += q * p7;
    t8 += q * p8;
    t1 += t0 >> 29;
    t0 = t1;
    t1 = t2;
    t2 = t3;
    t3 = t4;
    t4 = t5;
    t5 = t6;
    t6 = t7;
    t7 = t8;
    t8 = 0;
    for i in 0..M {
        t0 += x0[i] * y3[i];
    }
    for i in 0..M {
        t1 += x1[i] * y3[i];
    }
    for i in 0..M {
        t2 += x2[i] * y3[i];
    }
    for i in 0..M {
        t3 += x3[i] * y3[i];
    }
    for i in 0..M {
        t4 += x4[i] * y3[i];
    }
    for i in 0..M {
        t5 += x5[i] * y3[i];
    }
    for i in 0..M {
        t6 += x6[i] * y3[i];
    }
    for i in 0..M {
        t7 += x7[i] * y3[i];
    }
    for i in 0..M {
        t8 += x8[i] * y3[i];
    }
    let q = t0.wrapping_mul(inv) & MASK;
    t0 += q * p0;
    t1 += q * p1;
    t2 += q * p2;
    t3 += q * p3;
    t4 += q * p4;
    t5 += q * p5;
    t6 += q * p6;
    t7 += q * p7;
    t8 += q * p8;
    t1 += t0 >> 29;
    t0 = t1;
    t1 = t2;
    t2 = t3;
    t3 = t4;
    t4 = t5;
    t5 = t6;
    t6 = t7;
    t7 = t8;
    t8 = 0;
    for i in 0..M {
        t0 += x0[i] * y4[i];
    }
    for i in 0..M {
        t1 += x1[i] * y4[i];
    }
    for i in 0..M {
        t2 += x2[i] * y4[i];
    }
    for i in 0..M {
        t3 += x3[i] * y4[i];
    }
    for i in 0..M {
        t4 += x4[i] * y4[i];
    }
    for i in 0..M {
        t5 += x5[i] * y4[i];
    }
    for i in 0..M {
        t6 += x6[i] * y4[i];
    }
    for i in 0..M {
        t7 += x7[i] * y4[i];
    }
    for i in 0..M {
        t8 += x8[i] * y4[i];
    }
    let q = t0.wrapping_mul(inv) & MASK;
    t0 += q * p0;
    t1 += q * p1;
    t2 += q * p2;
    t3 += q * p3;
    t4 += q * p4;
    t5 += q * p5;
    t6 += q * p6;
    t7 += q * p7;
    t8 += q * p8;
    t1 += t0 >> 29;
    t0 = t1;
    t1 = t2;
    t2 = t3;
    t3 = t4;
    t4 = t5;
    t5 = t6;
    t6 = t7;
    t7 = t8;
    t8 = 0;
    for i in 0..M {
        t0 += x0[i] * y5[i];
    }
    for i in 0..M {
        t1 += x1[i] * y5[i];
    }
    for i in 0..M {
        t2 += x2[i] * y5[i];
    }
    for i in 0..M {
        t3 += x3[i] * y5[i];
    }
    for i in 0..M {
        t4 += x4[i] * y5[i];
    }
    for i in 0..M {
        t5 += x5[i] * y5[i];
    }
    for i in 0..M {
        t6 += x6[i] * y5[i];
    }
    for i in 0..M {
        t7 += x7[i] * y5[i];
    }
    for i in 0..M {
        t8 += x8[i] * y5[i];
    }
    let q = t0.wrapping_mul(inv) & MASK;
    t0 += q * p0;
    t1 += q * p1;
    t2 += q * p2;
    t3 += q * p3;
    t4 += q * p4;
    t5 += q * p5;
    t6 += q * p6;
    t7 += q * p7;
    t8 += q * p8;
    t1 += t0 >> 29;
    t0 = t1;
    t1 = t2;
    t2 = t3;
    t3 = t4;
    t4 = t5;
    t5 = t6;
    t6 = t7;
    t7 = t8;
    t8 = 0;
    for i in 0..M {
        t0 += x0[i] * y6[i];
    }
    for i in 0..M {
        t1 += x1[i] * y6[i];
    }
    for i in 0..M {
        t2 += x2[i] * y6[i];
    }
    for i in 0..M {
        t3 += x3[i] * y6[i];
    }
    for i in 0..M {
        t4 += x4[i] * y6[i];
    }
    for i in 0..M {
        t5 += x5[i] * y6[i];
    }
    for i in 0..M {
        t6 += x6[i] * y6[i];
    }
    for i in 0..M {
        t7 += x7[i] * y6[i];
    }
    for i in 0..M {
        t8 += x8[i] * y6[i];
    }
    let q = t0.wrapping_mul(inv) & MASK;
    t0 += q * p0;
    t1 += q * p1;
    t2 += q * p2;
    t3 += q * p3;
    t4 += q * p4;
    t5 += q * p5;
    t6 += q * p6;
    t7 += q * p7;
    t8 += q * p8;
    t1 += t0 >> 29;
    t0 = t1;
    t1 = t2;
    t2 = t3;
    t3 = t4;
    t4 = t5;
    t5 = t6;
    t6 = t7;
    t7 = t8;
    t8 = 0;
    for i in 0..M {
        t0 += x0[i] * y7[i];
    }
    for i in 0..M {
        t1 += x1[i] * y7[i];
    }
    for i in 0..M {
        t2 += x2[i] * y7[i];
    }
    for i in 0..M {
        t3 += x3[i] * y7[i];
    }
    for i in 0..M {
        t4 += x4[i] * y7[i];
    }
    for i in 0..M {
        t5 += x5[i] * y7[i];
    }
    for i in 0..M {
        t6 += x6[i] * y7[i];
    }
    for i in 0..M {
        t7 += x7[i] * y7[i];
    }
    for i in 0..M {
        t8 += x8[i] * y7[i];
    }
    let q = t0.wrapping_mul(inv) & MASK;
    t0 += q * p0;
    t1 += q * p1;
    t2 += q * p2;
    t3 += q * p3;
    t4 += q * p4;
    t5 += q * p5;
    t6 += q * p6;
    t7 += q * p7;
    t8 += q * p8;
    t1 += t0 >> 29;
    t0 = t1;
    t1 = t2;
    t2 = t3;
    t3 = t4;
    t4 = t5;
    t5 = t6;
    t6 = t7;
    t7 = t8;
    t8 = 0;
    for i in 0..M {
        t0 += x0[i] * y8[i];
    }
    for i in 0..M {
        t1 += x1[i] * y8[i];
    }
    for i in 0..M {
        t2 += x2[i] * y8[i];
    }
    for i in 0..M {
        t3 += x3[i] * y8[i];
    }
    for i in 0..M {
        t4 += x4[i] * y8[i];
    }
    for i in 0..M {
        t5 += x5[i] * y8[i];
    }
    for i in 0..M {
        t6 += x6[i] * y8[i];
    }
    for i in 0..M {
        t7 += x7[i] * y8[i];
    }
    for i in 0..M {
        t8 += x8[i] * y8[i];
    }
    let q = t0.wrapping_mul(inv) & ((1 << 24) - 1);
    t0 += q * p0;
    t1 += q * p1;
    t2 += q * p2;
    t3 += q * p3;
    t4 += q * p4;
    t5 += q * p5;
    t6 += q * p6;
    t7 += q * p7;
    t8 += q * p8;
    t1 += t0 >> 29;
    t0 &= MASK;
    t2 += t1 >> 29;
    t1 &= MASK;
    t3 += t2 >> 29;
    t2 &= MASK;
    t4 += t3 >> 29;
    t3 &= MASK;
    t5 += t4 >> 29;
    t4 &= MASK;
    t6 += t5 >> 29;
    t5 &= MASK;
    t7 += t6 >> 29;
    t6 &= MASK;
    t8 += t7 >> 29;
    t7 &= MASK;
    t9 += t8 >> 29;
    t8 &= MASK;
    let mut words = [0u64; 4];
    words[0] = t0 >> 24;
    words[0] |= t1 << 5;
    words[0] |= t2 << 34;
    words[0] |= t3 << 63;
    words[1] |= t3 >> 1;
    words[1] |= t4 << 28;
    words[1] |= t5 << 57;
    words[2] |= t5 >> 7;
    words[2] |= t6 << 22;
    words[2] |= t7 << 51;
    words[3] |= t7 >> 13;
    words[3] |= t8 << 16;
    words[3] |= t9 << 45;
    let mut out = BigInt(words);
    if out >= p {
        out.sub_with_borrow(&p);
    }
    out
}

#[inline(always)]
pub fn multiply(x: BigInt<4>, y: BigInt<4>, p: BigInt<4>, inv: u64) -> BigInt<4> {
    dot([x], [y], p, inv)
}
#[inline(always)]
pub fn sum2(x: [BigInt<4>; 2], y: [BigInt<4>; 2], p: BigInt<4>, inv: u64) -> BigInt<4> {
    dot(x, y, p, inv)
}
