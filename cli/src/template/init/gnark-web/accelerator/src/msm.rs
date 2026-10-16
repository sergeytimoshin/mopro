use crate::simd::{fq_mul4, FqLanes, FqLimbs};
use ark_bn254::{Fq, Fq2, Fr};
use ark_ec::{
    short_weierstrass::{Affine, Projective, SWCurveConfig},
    AdditiveGroup, AffineRepr, CurveGroup, VariableBaseMSM,
};
use ark_ff::{BigInt, Field, PrimeField, Zero};
use rayon::prelude::*;
// Signed-window Pippenger. Reduce each bucket as an affine addition tree,
// sharing one inversion across all buckets at each level. Keep bucket storage
// contiguous: thousands of small allocations contend on the WASM allocator.
pub(super) fn msm<P: SWCurveConfig<ScalarField = Fr>>(
    bases: &[Affine<P>],
    scalars: &[Fr],
) -> Projective<P>
where
    P::BaseField: BatchInverse,
{
    assert_eq!(bases.len(), scalars.len(), "validated MSM dimensions");
    if bases.len() < 256 {
        return Projective::<P>::msm(bases, scalars).unwrap();
    }
    // Wider windows mean fewer digits per scalar, so fewer bucket additions,
    // at the cost of more buckets to sum per window; past 2^16 points the
    // additions dominate.
    let width: usize = match bases.len() {
        0..=1023 => 7,
        1024..=4095 => 9,
        4096..=16383 => 10,
        16384..=65535 => 12,
        65536..=262143 => 14,
        _ => 15,
    };
    // Leave room for the final carry when recoding into signed digits.
    let count = (Fr::MODULUS_BIT_SIZE as usize + 1).div_ceil(width);
    let radix = 1u64 << width;
    let mask = radix - 1;
    // Preserve indexed writes into one allocation. Collecting a flattened
    // parallel iterator creates temporary vectors and contends on the WASM heap.
    let mut digits = vec![0i16; scalars.len() * count];
    digits
        .par_chunks_mut(count)
        .zip(scalars)
        .for_each(|(digits, s)| {
            let BigInt(words) = s.into_bigint();
            let mut carry = 0;
            for (i, digit) in digits.iter_mut().enumerate() {
                let offset = i * width;
                let word = offset / 64;
                let shift = offset % 64;
                let mut value = words[word] >> shift;
                if shift + width > 64 && word < 3 {
                    value |= words[word + 1] << (64 - shift);
                }
                let value = (value & mask) + carry;
                carry = (value + radix / 2) >> width;
                *digit = (value as i32 - (carry * radix) as i32) as i16;
            }
        });
    let sums: Vec<Projective<P>> = (0..count)
        .into_par_iter()
        .map(|i| {
            let bucket_count = (radix / 2) as usize;
            let mut counts = vec![0usize; bucket_count];
            for (base, digits) in bases.iter().zip(digits.chunks(count)) {
                let digit = digits[i];
                if digit != 0 && !base.infinity {
                    counts[digit.unsigned_abs() as usize - 1] += 1;
                }
            }
            let mut starts = vec![0; bucket_count + 1];
            for j in 0..bucket_count {
                starts[j + 1] = starts[j] + counts[j];
            }
            let mut positions = starts[..bucket_count].to_vec();
            let mut points = vec![LimbPoint::<P::BaseField>::infinity(); starts[bucket_count]];
            for (base, digits) in bases.iter().zip(digits.chunks(count)) {
                let digit = digits[i];
                if digit != 0 && !base.infinity {
                    let j = digit.unsigned_abs() as usize - 1;
                    points[positions[j]] = limbs::<P>(if digit < 0 { -*base } else { *base });
                    positions[j] += 1;
                }
            }
            sum_buckets(&mut points, &starts, &mut counts)
        })
        .collect();
    let mut result = Projective::<P>::ZERO;
    for sum in sums.iter().rev() {
        for _ in 0..width {
            result.double_in_place();
        }
        result += sum;
    }
    result
}

// Smaller bucket groups let idle workers help a long-running window and keep
// each affine inversion batch's working set small. Each task owns disjoint
// point and count slices; no shared writes or additional prepared key data.
fn sum_buckets<P: SWCurveConfig>(
    points: &mut [LimbPoint<P::BaseField>],
    starts: &[usize],
    counts: &mut [usize],
) -> Projective<P>
where
    P::BaseField: BatchInverse,
{
    const GROUP: usize = 256;
    if counts.len() <= GROUP || rayon::current_num_threads() == 1 {
        return sum_bucket_group(points, starts, counts).0;
    }
    let mut tasks = Vec::with_capacity(counts.len() / GROUP);
    let mut remaining = points;
    for (i, counts) in counts.chunks_mut(GROUP).enumerate() {
        let offset = starts[i * GROUP];
        let size = starts[(i + 1) * GROUP] - offset;
        let (chunk, rest) = remaining.split_at_mut(size);
        remaining = rest;
        let starts: Vec<_> = starts[i * GROUP..(i + 1) * GROUP]
            .iter()
            .map(|s| s - offset)
            .collect();
        tasks.push((chunk, starts, counts));
    }
    let partials: Vec<_> = tasks
        .into_par_iter()
        .map(|(points, starts, counts)| sum_bucket_group(points, &starts, counts))
        .collect();
    // Local weights restart at one. Group i therefore needs an additional
    // i * GROUP times its unweighted total. Reverse running sums compute
    // those offsets without a separate scalar multiplication for each group.
    let mut sum = Projective::<P>::ZERO;
    let mut offset = Projective::<P>::ZERO;
    let mut running = Projective::<P>::ZERO;
    for (weighted, total) in partials.into_iter().rev() {
        sum += weighted;
        offset += running;
        running += total;
    }
    for _ in 0..GROUP.ilog2() {
        offset.double_in_place();
    }
    sum + offset
}

fn sum_bucket_group<P: SWCurveConfig>(
    points: &mut [LimbPoint<P::BaseField>],
    starts: &[usize],
    counts: &mut [usize],
) -> (Projective<P>, Affine<P>)
where
    P::BaseField: BatchInverse,
{
    let mut scratch = Scratch::default();
    while counts.iter().any(|n| *n > 1) {
        reduce_layer(points, starts, counts, &mut scratch);
    }
    let n = counts.len();
    let mut buckets = vec![LimbPoint::<P::BaseField>::infinity(); n * 2 - 1];
    for j in 0..n {
        if counts[j] != 0 {
            buckets[j] = points[starts[j]];
        }
    }
    let weighted = weighted_sum(&mut buckets, n, &mut scratch);
    (weighted, affine::<P>(buckets[0]))
}

// A bucket point, coordinates in limb form; the identity is (0, 0), which
// is not on either curve.
#[derive(Clone, Copy)]
struct LimbPoint<F: BatchInverse> {
    x: F::Limbs,
    y: F::Limbs,
}
impl<F: BatchInverse> LimbPoint<F> {
    #[inline(always)]
    fn infinity() -> Self {
        Self {
            x: Default::default(),
            y: Default::default(),
        }
    }
    #[inline(always)]
    fn is_infinity(&self) -> bool {
        F::limbs_zero(&self.x) && F::limbs_zero(&self.y)
    }
}
#[inline(always)]
fn limbs<P: SWCurveConfig>(point: Affine<P>) -> LimbPoint<P::BaseField>
where
    P::BaseField: BatchInverse,
{
    if point.infinity {
        LimbPoint::infinity()
    } else {
        LimbPoint {
            x: P::BaseField::to_limbs(point.x),
            y: P::BaseField::to_limbs(point.y),
        }
    }
}
#[inline(always)]
fn affine<P: SWCurveConfig>(point: LimbPoint<P::BaseField>) -> Affine<P>
where
    P::BaseField: BatchInverse,
{
    if point.is_infinity() {
        Affine::identity()
    } else {
        Affine::new_unchecked(
            P::BaseField::from_limbs(point.x),
            P::BaseField::from_limbs(point.y),
        )
    }
}

// Buffers of one layer, reused across the layers of a bucket group.
struct Scratch<P: SWCurveConfig>
where
    P::BaseField: BatchInverse,
{
    // The first point of each pair the affine formula adds, in bucket order.
    pairs: Vec<usize>,
    // Groups of four pairs in lane form, then the scalar tail.
    groups: Vec<Group<P::BaseField>>,
    denominators: Vec<P::BaseField>,
    products: Vec<<P::BaseField as BatchInverse>::Product>,
    sums: Vec<LimbPoint<P::BaseField>>,
}
// Four pairs a + b: coordinates, denominator b.x - a.x, the norm the chains
// multiply by, and their chains' prefix, later 1/norm.
#[derive(Clone, Copy)]
struct Group<F: BatchInverse> {
    ax: F::Lanes,
    ay: F::Lanes,
    bx: F::Lanes,
    by: F::Lanes,
    denominator: F::Lanes,
    norm: FqLanes,
    prefix: FqLanes,
}
impl<P: SWCurveConfig> Default for Scratch<P>
where
    P::BaseField: BatchInverse,
{
    fn default() -> Self {
        Self {
            pairs: Vec::new(),
            groups: Vec::new(),
            denominators: Vec::new(),
            products: Vec::new(),
            sums: Vec::new(),
        }
    }
}

// Adds each bucket's points in pairs. Pair k joins prefix chain k % 4: four
// chains of one batch inversion, so that a group of four pairs runs every
// operation on four lanes, in lane form from loading its coordinates to its
// sums. Pairs with an identity, a doubling or opposite points take the
// complete formulas instead.
fn reduce_layer<P: SWCurveConfig>(
    points: &mut [LimbPoint<P::BaseField>],
    starts: &[usize],
    counts: &mut [usize],
    s: &mut Scratch<P>,
) where
    P::BaseField: BatchInverse,
{
    s.pairs.clear();
    for index in 0..counts.len() {
        let offset = starts[index];
        for j in (0..counts[index].saturating_sub(1)).step_by(2) {
            let (a, b) = (&points[offset + j], &points[offset + j + 1]);
            if !a.is_infinity() && !b.is_infinity() && a.x != b.x {
                s.pairs.push(offset + j);
            }
        }
    }
    type F<P> = <P as ark_ec::CurveConfig>::BaseField;
    let m = s.pairs.len();
    let full = m / 4 * 4;
    s.groups.clear();
    let mut chains = FqLanes::one();
    for k in (0..full).step_by(4) {
        let a: [&LimbPoint<F<P>>; 4] = core::array::from_fn(|i| &points[s.pairs[k + i]]);
        let b: [&LimbPoint<F<P>>; 4] = core::array::from_fn(|i| &points[s.pairs[k + i] + 1]);
        let (ax, ay) = (F::<P>::load(a.map(|p| &p.x)), F::<P>::load(a.map(|p| &p.y)));
        let (bx, by) = (F::<P>::load(b.map(|p| &p.x)), F::<P>::load(b.map(|p| &p.y)));
        let denominator = F::<P>::lanes_sub(&bx, &ax);
        let norm = F::<P>::lanes_norm(&denominator);
        let prefix = chains;
        chains = chains.mul(&norm);
        s.groups.push(Group {
            ax,
            ay,
            bx,
            by,
            denominator,
            norm,
            prefix,
        });
    }
    s.denominators.clear();
    s.products.clear();
    let mut tail = chains.get();
    for k in full..m {
        let (a, b) = (affine::<P>(points[s.pairs[k]]), affine::<P>(points[s.pairs[k] + 1]));
        let denominator = b.x - a.x;
        s.denominators.push(denominator);
        s.products.push(F::<P>::prefix(&mut tail[k % 4], denominator));
    }
    let mut inverses = invert4(tail);
    for k in (full..m).rev() {
        let t = k - full;
        F::<P>::reverse(&mut inverses[k % 4], s.denominators[t], &mut s.products[t]);
    }
    let mut inverses = FqLanes::new(&inverses);
    for group in s.groups.iter_mut().rev() {
        let inverse_norm = inverses.mul(&group.prefix);
        inverses = inverses.mul(&group.norm);
        group.prefix = inverse_norm;
    }
    s.sums.clear();
    for group in &s.groups {
        let inverse = F::<P>::lanes_inverse(&group.denominator, &group.prefix);
        let slope = F::<P>::lanes_mul(&F::<P>::lanes_sub(&group.by, &group.ay), &inverse);
        let x = F::<P>::lanes_sub(
            &F::<P>::lanes_sub(&F::<P>::lanes_square(&slope), &group.ax),
            &group.bx,
        );
        let y = F::<P>::lanes_sub(
            &F::<P>::lanes_mul(&slope, &F::<P>::lanes_sub(&group.ax, &x)),
            &group.ay,
        );
        let (x, y) = (F::<P>::store(&x), F::<P>::store(&y));
        for i in 0..4 {
            s.sums.push(LimbPoint { x: x[i], y: y[i] });
        }
    }
    for k in full..m {
        let t = k - full;
        let (a, b) = (affine::<P>(points[s.pairs[k]]), affine::<P>(points[s.pairs[k] + 1]));
        let slope =
            (b.y - a.y) * F::<P>::denominator_inverse(s.denominators[t], s.products[t]);
        let x = slope.square() - a.x - b.x;
        let y = slope * (a.x - x) - a.y;
        s.sums.push(limbs::<P>(Affine::new_unchecked(x, y)));
    }
    // Each bucket keeps its pair sums in front, in order. A write lands
    // before the pairs still to be read.
    let mut cursor = 0;
    for index in 0..counts.len() {
        let offset = starts[index];
        let n = counts[index];
        for j in 0..n / 2 {
            points[offset + j] = if s.pairs.get(cursor) == Some(&(offset + 2 * j)) {
                cursor += 1;
                s.sums[cursor - 1]
            } else {
                let (a, b) = (points[offset + 2 * j], points[offset + 2 * j + 1]);
                if a.is_infinity() {
                    b
                } else if b.is_infinity() {
                    a
                } else {
                    // Doubling and opposite points need the complete formulas.
                    limbs::<P>((affine::<P>(a) + affine::<P>(b)).into_affine())
                }
            };
        }
        if !n.is_multiple_of(2) {
            points[offset + n / 2] = points[offset + n - 1];
        }
        counts[index] = n.div_ceil(2);
    }
}

// The inverses of four chains with one field inversion: inverting is far
// dearer than the few products that share it. Every denominator is
// nonzero, so is every chain.
fn invert4(c: [Fq; 4]) -> [Fq; 4] {
    let (c01, c23) = (c[0] * c[1], c[2] * c[3]);
    let inverse = (c01 * c23).inverse().unwrap();
    let (i01, i23) = (inverse * c23, inverse * c01);
    fq_mul4(&[i01, i01, i23, i23], &[c[1], c[0], c[3], c[2]])
}

// W(B) = 2*W(B[0]+B[1], B[2]+B[3], ...) - sum(B[0], B[2], ...).
// Reduce the odd-weight sums from all preceding levels together with the
// current paired buckets, sharing one inverse per level.
fn weighted_sum<P: SWCurveConfig>(
    points: &mut [LimbPoint<P::BaseField>],
    n: usize,
    scratch: &mut Scratch<P>,
) -> Projective<P>
where
    P::BaseField: BatchInverse,
{
    debug_assert!(n.is_power_of_two());
    debug_assert_eq!(points.len(), 2 * n - 1);
    let mut starts = Vec::with_capacity(n.ilog2() as usize + 1);
    let mut counts = Vec::with_capacity(n.ilog2() as usize + 1);
    starts.push(0);
    counts.push(n);
    let mut next = n;
    while counts[0] > 1 {
        let half = counts[0] / 2;
        for i in 0..half {
            points[next + i] = points[2 * i];
        }
        reduce_layer(points, &starts, &mut counts, scratch);
        starts.push(next);
        counts.push(half);
        next += half;
    }
    let mut sum = affine::<P>(points[0]).into_group();
    for start in starts[1..].iter().rev() {
        sum.double_in_place();
        sum -= affine::<P>(points[*start]);
    }
    sum
}

// In Fq2 = Fq[u]/(u^2 + 1), invert x + y*u using
// (x - y*u)/(x^2 + y^2). Batch the norms in Fq instead of multiplying
// Fq2 prefix products. Keep the norm beside each prefix so the reverse
// pass does not compute it again. Exceptional pairs are excluded above.

pub(super) trait BatchInverse: Field {
    type Product: Copy + Send + Default;
    fn prefix(accumulator: &mut Fq, value: Self) -> Self::Product;
    fn reverse(inverse: &mut Fq, value: Self, product: &mut Self::Product);
    fn denominator_inverse(value: Self, product: Self::Product) -> Self;
    /// One element as bucket points store it.
    type Limbs: Copy + Default + PartialEq + Send + Sync;
    fn to_limbs(value: Self) -> Self::Limbs;
    fn from_limbs(limbs: Self::Limbs) -> Self;
    fn limbs_zero(limbs: &Self::Limbs) -> bool;
    /// Four elements in lane form.
    type Lanes: Copy;
    fn load(values: [&Self::Limbs; 4]) -> Self::Lanes;
    fn store(lanes: &Self::Lanes) -> [Self::Limbs; 4];
    fn lanes_sub(a: &Self::Lanes, b: &Self::Lanes) -> Self::Lanes;
    fn lanes_mul(a: &Self::Lanes, b: &Self::Lanes) -> Self::Lanes;
    fn lanes_square(a: &Self::Lanes) -> Self::Lanes;
    /// What the inversion chains multiply by: the value, or its Fq norm.
    fn lanes_norm(value: &Self::Lanes) -> FqLanes;
    /// 1/value from the inverse of its norm.
    fn lanes_inverse(value: &Self::Lanes, inverse_norm: &FqLanes) -> Self::Lanes;
}
impl BatchInverse for Fq {
    type Product = Fq;
    #[inline(always)]
    fn prefix(accumulator: &mut Fq, value: Self) -> Fq {
        let prefix = *accumulator;
        *accumulator *= value;
        prefix
    }
    #[inline(always)]
    fn reverse(inverse: &mut Fq, value: Self, product: &mut Fq) {
        *product *= *inverse;
        *inverse *= value;
    }
    #[inline(always)]
    fn denominator_inverse(_: Self, product: Fq) -> Self {
        product
    }
    type Limbs = FqLimbs;
    #[inline(always)]
    fn to_limbs(value: Self) -> FqLimbs {
        FqLimbs::new(value)
    }
    #[inline(always)]
    fn from_limbs(limbs: FqLimbs) -> Self {
        limbs.get()
    }
    #[inline(always)]
    fn limbs_zero(limbs: &FqLimbs) -> bool {
        limbs.is_zero()
    }
    type Lanes = FqLanes;
    #[inline(always)]
    fn load(values: [&FqLimbs; 4]) -> FqLanes {
        FqLanes::load(values)
    }
    #[inline(always)]
    fn store(lanes: &FqLanes) -> [FqLimbs; 4] {
        lanes.store()
    }
    #[inline(always)]
    fn lanes_sub(a: &FqLanes, b: &FqLanes) -> FqLanes {
        a.sub(b)
    }
    #[inline(always)]
    fn lanes_mul(a: &FqLanes, b: &FqLanes) -> FqLanes {
        a.mul(b)
    }
    #[inline(always)]
    fn lanes_square(a: &FqLanes) -> FqLanes {
        a.mul(a)
    }
    #[inline(always)]
    fn lanes_norm(value: &FqLanes) -> FqLanes {
        *value
    }
    #[inline(always)]
    fn lanes_inverse(_: &FqLanes, inverse_norm: &FqLanes) -> FqLanes {
        *inverse_norm
    }
}
impl BatchInverse for Fq2 {
    type Product = [Fq; 2];
    #[inline(always)]
    fn prefix(accumulator: &mut Fq, value: Self) -> Self::Product {
        let norm = Fq::sum_of_products(&[value.c0, value.c1], &[value.c0, value.c1]);
        let prefix = *accumulator;
        *accumulator *= norm;
        [prefix, norm]
    }
    #[inline(always)]
    fn reverse(inverse: &mut Fq, _: Self, product: &mut Self::Product) {
        product[0] *= *inverse;
        *inverse *= product[1];
    }
    #[inline(always)]
    fn denominator_inverse(value: Self, product: Self::Product) -> Self {
        Fq2::new(value.c0 * product[0], -(value.c1 * product[0]))
    }
    // c0 and c1 of four elements. Fq2 = Fq[u]/(u^2 + 1).
    type Limbs = [FqLimbs; 2];
    #[inline(always)]
    fn to_limbs(value: Self) -> [FqLimbs; 2] {
        [FqLimbs::new(value.c0), FqLimbs::new(value.c1)]
    }
    #[inline(always)]
    fn from_limbs(limbs: [FqLimbs; 2]) -> Self {
        Fq2::new(limbs[0].get(), limbs[1].get())
    }
    #[inline(always)]
    fn limbs_zero(limbs: &[FqLimbs; 2]) -> bool {
        limbs[0].is_zero() && limbs[1].is_zero()
    }
    type Lanes = [FqLanes; 2];
    #[inline(always)]
    fn load(values: [&[FqLimbs; 2]; 4]) -> [FqLanes; 2] {
        [
            FqLanes::load(values.map(|v| &v[0])),
            FqLanes::load(values.map(|v| &v[1])),
        ]
    }
    #[inline(always)]
    fn store(lanes: &[FqLanes; 2]) -> [[FqLimbs; 2]; 4] {
        let (c0, c1) = (lanes[0].store(), lanes[1].store());
        core::array::from_fn(|i| [c0[i], c1[i]])
    }
    #[inline(always)]
    fn lanes_sub(a: &[FqLanes; 2], b: &[FqLanes; 2]) -> [FqLanes; 2] {
        [a[0].sub(&b[0]), a[1].sub(&b[1])]
    }
    // Karatsuba: three Fq products.
    #[inline(always)]
    fn lanes_mul(a: &[FqLanes; 2], b: &[FqLanes; 2]) -> [FqLanes; 2] {
        let v0 = a[0].mul(&b[0]);
        let v1 = a[1].mul(&b[1]);
        let v2 = a[0].add(&a[1]).mul(&b[0].add(&b[1]));
        [v0.sub(&v1), v2.sub(&v0).sub(&v1)]
    }
    // (c0 + c1 u)^2 = (c0 + c1)(c0 - c1) + 2 c0 c1 u.
    #[inline(always)]
    fn lanes_square(a: &[FqLanes; 2]) -> [FqLanes; 2] {
        let product = a[0].mul(&a[1]);
        [a[0].add(&a[1]).mul(&a[0].sub(&a[1])), product.add(&product)]
    }
    #[inline(always)]
    fn lanes_norm(value: &[FqLanes; 2]) -> FqLanes {
        value[0].mul(&value[0]).add(&value[1].mul(&value[1]))
    }
    // (c0 - c1 u) / norm.
    #[inline(always)]
    fn lanes_inverse(value: &[FqLanes; 2], inverse_norm: &FqLanes) -> [FqLanes; 2] {
        [
            value[0].mul(inverse_norm),
            FqLanes::zero().sub(&value[1].mul(inverse_norm)),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_ec::PrimeGroup;
    fn check<P: SWCurveConfig<ScalarField = Fr>>()
    where
        P::BaseField: BatchInverse,
    {
        let mut seed = 41u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let scalars: Vec<Fr> = (0..16385)
            .map(|i| match i % 20 {
                0 => Fr::ZERO,
                1 => Fr::ONE,
                2 => -Fr::ONE,
                _ => Fr::from_le_bytes_mod_order(
                    &[
                        next().to_le_bytes(),
                        next().to_le_bytes(),
                        next().to_le_bytes(),
                        next().to_le_bytes(),
                    ]
                    .concat(),
                ),
            })
            .collect();
        let bases: Vec<Affine<P>> = (0..16385)
            .map(|i| match i % 20 {
                0 => Affine::identity(),
                1..=3 => Projective::<P>::generator().into_affine(),
                4 => (-Projective::<P>::generator()).into_affine(),
                _ => (Projective::<P>::generator() * Fr::from(next())).into_affine(),
            })
            .collect();
        for n in [
            0, 1, 255, 256, 511, 1023, 1024, 1025, 4095, 4096, 16383, 16384, 16385,
        ] {
            assert_eq!(
                msm::<P>(&bases[..n], &scalars[..n]),
                Projective::<P>::msm(&bases[..n], &scalars[..n]).unwrap(),
                "size {n}"
            );
        }
        let generator = Projective::<P>::generator().into_affine();
        for scalar in [Fr::ZERO, Fr::ONE, -Fr::ONE] {
            let scalars = vec![scalar; 4096];
            // Force exceptional additions at multiple levels of a bucket tree.
            for bases in [
                vec![generator; 4096],
                (0..4096)
                    .map(|i| if i % 2 == 0 { generator } else { -generator })
                    .collect(),
                vec![Affine::identity(); 4096],
            ] {
                assert_eq!(
                    msm::<P>(&bases, &scalars),
                    Projective::<P>::msm(&bases, &scalars).unwrap()
                );
            }
        }
    }

    #[test]
    fn extension_field_batch_inverses() {
        let mut seed = 79u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut field = || {
            Fq::from_le_bytes_mod_order(
                &[
                    next().to_le_bytes(),
                    next().to_le_bytes(),
                    next().to_le_bytes(),
                    next().to_le_bytes(),
                ]
                .concat(),
            )
        };
        let values: Vec<_> = (0..1024)
            .map(|i| match i % 8 {
                0 => Fq2::ZERO,
                1 => Fq2::ONE,
                2 => -Fq2::ONE,
                3 => Fq2::new(Fq::ZERO, field()),
                4 => Fq2::new(field(), Fq::ZERO),
                _ => Fq2::new(field(), field()),
            })
            .collect();
        let mut accumulator = Fq::ONE;
        let mut products: Vec<_> = values
            .iter()
            .map(|value| {
                if value.is_zero() {
                    Default::default()
                } else {
                    Fq2::prefix(&mut accumulator, *value)
                }
            })
            .collect();
        let mut inverse = accumulator.inverse().unwrap();
        for (value, product) in values.iter().zip(&mut products).rev() {
            if !value.is_zero() {
                Fq2::reverse(&mut inverse, *value, product);
            }
        }
        for (value, product) in values.into_iter().zip(products) {
            if !value.is_zero() {
                assert_eq!(
                    Fq2::denominator_inverse(value, product),
                    value.inverse().unwrap()
                );
            }
        }
    }
    #[test]
    fn g1() {
        check::<ark_bn254::g1::Config>();
    }
    #[test]
    fn g2() {
        check::<ark_bn254::g2::Config>();
    }
}
