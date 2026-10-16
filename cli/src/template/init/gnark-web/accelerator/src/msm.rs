use crate::simd::fq_mul4;
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
            let mut points = vec![Affine::<P>::identity(); starts[bucket_count]];
            for (base, digits) in bases.iter().zip(digits.chunks(count)) {
                let digit = digits[i];
                if digit != 0 && !base.infinity {
                    let j = digit.unsigned_abs() as usize - 1;
                    points[positions[j]] = if digit < 0 { -*base } else { *base };
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
    points: &mut [Affine<P>],
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
    points: &mut [Affine<P>],
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
    let mut buckets = vec![Affine::<P>::identity(); n * 2 - 1];
    for j in 0..n {
        if counts[j] != 0 {
            buckets[j] = points[starts[j]];
        }
    }
    let weighted = weighted_sum(&mut buckets, n, &mut scratch);
    (weighted, buckets[0])
}

// Buffers of one layer, reused across the layers of a bucket group.
struct Scratch<P: SWCurveConfig>
where
    P::BaseField: BatchInverse,
{
    // The first point of each pair the affine formula adds, in bucket order.
    pairs: Vec<usize>,
    denominators: Vec<P::BaseField>,
    products: Vec<<P::BaseField as BatchInverse>::Product>,
    sums: Vec<Affine<P>>,
}
impl<P: SWCurveConfig> Default for Scratch<P>
where
    P::BaseField: BatchInverse,
{
    fn default() -> Self {
        Self {
            pairs: Vec::new(),
            denominators: Vec::new(),
            products: Vec::new(),
            sums: Vec::new(),
        }
    }
}

// Adds each bucket's points in pairs. Pair k joins prefix chain k % 4, so
// that every multiplication of the batch inversion and of the slopes runs
// four lanes at once; each chain is inverted alone. Pairs with an identity,
// a doubling or opposite points take the complete formulas instead.
fn reduce_layer<P: SWCurveConfig>(
    points: &mut [Affine<P>],
    starts: &[usize],
    counts: &mut [usize],
    s: &mut Scratch<P>,
) where
    P::BaseField: BatchInverse,
{
    s.pairs.clear();
    s.denominators.clear();
    for index in 0..counts.len() {
        let offset = starts[index];
        for j in (0..counts[index].saturating_sub(1)).step_by(2) {
            let (a, b) = (points[offset + j], points[offset + j + 1]);
            if !a.infinity && !b.infinity && a.x != b.x {
                s.pairs.push(offset + j);
                s.denominators.push(b.x - a.x);
            }
        }
    }
    let m = s.pairs.len();
    let full = m / 4 * 4;
    let lanes = |values: &[P::BaseField], k: usize| -> [P::BaseField; 4] {
        core::array::from_fn(|i| values[k + i])
    };
    s.products.clear();
    s.products.resize(m, Default::default());
    let mut chains = [Fq::ONE; 4];
    for k in (0..full).step_by(4) {
        let prefixes = P::BaseField::prefix4(&mut chains, lanes(&s.denominators, k));
        s.products[k..k + 4].copy_from_slice(&prefixes);
    }
    for k in full..m {
        s.products[k] = P::BaseField::prefix(&mut chains[k % 4], s.denominators[k]);
    }
    let mut inverses = invert4(chains);
    for k in (full..m).rev() {
        P::BaseField::reverse(&mut inverses[k % 4], s.denominators[k], &mut s.products[k]);
    }
    for k in (0..full).step_by(4).rev() {
        let mut products: [_; 4] = core::array::from_fn(|i| s.products[k + i]);
        P::BaseField::reverse4(&mut inverses, lanes(&s.denominators, k), &mut products);
        s.products[k..k + 4].copy_from_slice(&products);
    }
    s.sums.clear();
    for k in (0..full).step_by(4) {
        let a: [Affine<P>; 4] = core::array::from_fn(|i| points[s.pairs[k + i]]);
        let b: [Affine<P>; 4] = core::array::from_fn(|i| points[s.pairs[k + i] + 1]);
        let inverse = P::BaseField::denominator_inverse4(
            lanes(&s.denominators, k),
            core::array::from_fn(|i| s.products[k + i]),
        );
        let slope = P::BaseField::mul4(core::array::from_fn(|i| b[i].y - a[i].y), inverse);
        let square = P::BaseField::square4(slope);
        let x: [P::BaseField; 4] = core::array::from_fn(|i| square[i] - a[i].x - b[i].x);
        let y = P::BaseField::mul4(slope, core::array::from_fn(|i| a[i].x - x[i]));
        for i in 0..4 {
            s.sums.push(Affine::new_unchecked(x[i], y[i] - a[i].y));
        }
    }
    for k in full..m {
        let (a, b) = (points[s.pairs[k]], points[s.pairs[k] + 1]);
        let slope =
            (b.y - a.y) * P::BaseField::denominator_inverse(s.denominators[k], s.products[k]);
        let x = slope.square() - a.x - b.x;
        let y = slope * (a.x - x) - a.y;
        s.sums.push(Affine::new_unchecked(x, y));
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
                if a.infinity {
                    b
                } else if b.infinity {
                    a
                } else {
                    // Doubling and opposite points need the complete formulas.
                    (a + b).into_affine()
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
    points: &mut [Affine<P>],
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
    let mut sum = points[0].into_group();
    for start in starts[1..].iter().rev() {
        sum.double_in_place();
        sum -= points[*start];
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
    /// `a[i] * b[i]` for four lanes.
    fn mul4(a: [Self; 4], b: [Self; 4]) -> [Self; 4];
    /// `a[i]^2` for four lanes.
    fn square4(a: [Self; 4]) -> [Self; 4];
    /// `denominator_inverse` for four lanes.
    fn denominator_inverse4(values: [Self; 4], products: [Self::Product; 4]) -> [Self; 4];
    /// `prefix` on four chains, one value each.
    fn prefix4(chains: &mut [Fq; 4], values: [Self; 4]) -> [Self::Product; 4];
    /// `reverse` on four chains, one value each.
    fn reverse4(inverses: &mut [Fq; 4], values: [Self; 4], products: &mut [Self::Product; 4]);
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
    #[inline(always)]
    fn mul4(a: [Self; 4], b: [Self; 4]) -> [Self; 4] {
        fq_mul4(&a, &b)
    }
    #[inline(always)]
    fn square4(a: [Self; 4]) -> [Self; 4] {
        fq_mul4(&a, &a)
    }
    #[inline(always)]
    fn denominator_inverse4(_: [Self; 4], products: [Fq; 4]) -> [Self; 4] {
        products
    }
    #[inline(always)]
    fn prefix4(chains: &mut [Fq; 4], values: [Self; 4]) -> [Fq; 4] {
        let prefixes = *chains;
        *chains = fq_mul4(chains, &values);
        prefixes
    }
    #[inline(always)]
    fn reverse4(inverses: &mut [Fq; 4], values: [Self; 4], products: &mut [Fq; 4]) {
        *products = fq_mul4(products, inverses);
        *inverses = fq_mul4(inverses, &values);
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
    // Fq2 = Fq[u]/(u^2 + 1). Karatsuba: three Fq products per Fq2 product.
    #[inline(always)]
    fn mul4(a: [Self; 4], b: [Self; 4]) -> [Self; 4] {
        let v0 = fq_mul4(&a.map(|x| x.c0), &b.map(|x| x.c0));
        let v1 = fq_mul4(&a.map(|x| x.c1), &b.map(|x| x.c1));
        let v2 = fq_mul4(&a.map(|x| x.c0 + x.c1), &b.map(|x| x.c0 + x.c1));
        core::array::from_fn(|i| Fq2::new(v0[i] - v1[i], v2[i] - v0[i] - v1[i]))
    }
    // (c0 + c1 u)^2 = (c0 + c1)(c0 - c1) + 2 c0 c1 u.
    #[inline(always)]
    fn square4(a: [Self; 4]) -> [Self; 4] {
        let real = fq_mul4(&a.map(|x| x.c0 + x.c1), &a.map(|x| x.c0 - x.c1));
        let imaginary = fq_mul4(&a.map(|x| x.c0), &a.map(|x| x.c1));
        core::array::from_fn(|i| Fq2::new(real[i], imaginary[i].double()))
    }
    #[inline(always)]
    fn denominator_inverse4(values: [Self; 4], products: [Self::Product; 4]) -> [Self; 4] {
        let inverse = products.map(|p| p[0]);
        let c0 = fq_mul4(&values.map(|v| v.c0), &inverse);
        let c1 = fq_mul4(&values.map(|v| v.c1), &inverse);
        core::array::from_fn(|i| Fq2::new(c0[i], -c1[i]))
    }
    #[inline(always)]
    fn prefix4(chains: &mut [Fq; 4], values: [Self; 4]) -> [Self::Product; 4] {
        let c0 = values.map(|v| v.c0);
        let c1 = values.map(|v| v.c1);
        let (s0, s1) = (fq_mul4(&c0, &c0), fq_mul4(&c1, &c1));
        let norms: [Fq; 4] = core::array::from_fn(|i| s0[i] + s1[i]);
        let prefixes = *chains;
        *chains = fq_mul4(chains, &norms);
        core::array::from_fn(|i| [prefixes[i], norms[i]])
    }
    #[inline(always)]
    fn reverse4(inverses: &mut [Fq; 4], _: [Self; 4], products: &mut [Self::Product; 4]) {
        let scaled = fq_mul4(&products.map(|p| p[0]), inverses);
        *inverses = fq_mul4(inverses, &products.map(|p| p[1]));
        for (product, scaled) in products.iter_mut().zip(scaled) {
            product[0] = scaled;
        }
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
