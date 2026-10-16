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
    let width: usize = match bases.len() {
        0..=1023 => 7,
        1024..=4095 => 9,
        4096..=16383 => 10,
        _ => 11,
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
    let mut products = Vec::with_capacity(points.len() / 2);
    while counts.iter().any(|n| *n > 1) {
        reduce_layer(points, starts, counts, &mut products);
    }
    let n = counts.len();
    let mut buckets = vec![Affine::<P>::identity(); n * 2 - 1];
    for j in 0..n {
        if counts[j] != 0 {
            buckets[j] = points[starts[j]];
        }
    }
    let weighted = weighted_sum(&mut buckets, n, &mut products);
    (weighted, buckets[0])
}

fn reduce_layer<P: SWCurveConfig>(
    points: &mut [Affine<P>],
    starts: &[usize],
    counts: &mut [usize],
    products: &mut Vec<<P::BaseField as BatchInverse>::Product>,
) where
    P::BaseField: BatchInverse,
{
    // Prefix products omit exceptional pairs. The inverse pass reads
    // every pair before the forward pass overwrites its lower slot.
    products.clear();
    let mut product = Fq::ONE;
    for index in 0..counts.len() {
        let offset = starts[index];
        let n = counts[index];
        for j in (0..n.saturating_sub(1)).step_by(2) {
            let a = points[offset + j];
            let b = points[offset + j + 1];
            let denominator = b.x - a.x;
            let valid = !a.infinity && !b.infinity && !denominator.is_zero();
            products.push(if valid {
                P::BaseField::prefix(&mut product, denominator)
            } else {
                Default::default()
            });
        }
    }
    let mut inverse = product.inverse().unwrap();
    let mut cursor = products.len();
    for index in (0..counts.len()).rev() {
        let offset = starts[index];
        let n = counts[index];
        for j in (0..n / 2).rev() {
            let a = points[offset + 2 * j];
            let b = points[offset + 2 * j + 1];
            let denominator = b.x - a.x;
            cursor -= 1;
            if !a.infinity && !b.infinity && !denominator.is_zero() {
                P::BaseField::reverse(&mut inverse, denominator, &mut products[cursor]);
            }
        }
    }
    let mut cursor = 0;
    for index in 0..counts.len() {
        let offset = starts[index];
        let n = counts[index];
        for j in 0..n / 2 {
            let a = points[offset + 2 * j];
            let b = points[offset + 2 * j + 1];
            points[offset + j] = if a.infinity {
                b
            } else if b.infinity {
                a
            }
            // Doubling and opposite points need the complete formulas;
            // their zero denominators were excluded from the inverse.
            else if a.x == b.x {
                (a + b).into_affine()
            } else {
                let slope =
                    (b.y - a.y) * P::BaseField::denominator_inverse(b.x - a.x, products[cursor]);
                let x = slope.square() - a.x - b.x;
                let y = slope * (a.x - x) - a.y;
                Affine::new_unchecked(x, y)
            };
            cursor += 1;
        }
        if !n.is_multiple_of(2) {
            points[offset + n / 2] = points[offset + n - 1];
        }
        counts[index] = n.div_ceil(2);
    }
}

// W(B) = 2*W(B[0]+B[1], B[2]+B[3], ...) - sum(B[0], B[2], ...).
// Reduce the odd-weight sums from all preceding levels together with the
// current paired buckets, sharing one inverse per level.
fn weighted_sum<P: SWCurveConfig>(
    points: &mut [Affine<P>],
    n: usize,
    products: &mut Vec<<P::BaseField as BatchInverse>::Product>,
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
        reduce_layer(points, &starts, &mut counts, products);
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
