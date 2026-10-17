use ark_bn254::Fr;
use ark_ff::{AdditiveGroup, Field};
use ark_poly::{EvaluationDomain, Radix2EvaluationDomain};
use rayon::prelude::*;

use crate::simd::FrLanes;

/// A quad: four consecutive elements of a transform, in lane form.
pub type Quad = FrLanes;

// Quads per parallel task in the wide passes (1024 elements).
const TASK: usize = 256;

// Cached twiddles and coset powers for a prepared proving key, in quads: a
// transform converts its values to lane form once, runs every layer on
// four elements at a time and converts back once. An inverse DIF transform
// emits bit-reversed coefficients; the forward DIT transform consumes that
// order directly. The final inverse transform already matches gnark's
// bit-reversed Z key, so no proof-time permutation is needed. Domains
// below eight elements, which only tests use, go through arkworks.
pub struct Plan {
    n: usize,
    roots: Twiddles,
    inverse_roots: Twiddles,
    forward_weights: Vec<Quad>,
    reverse_weights: Vec<Quad>,
    small: Option<(Radix2EvaluationDomain<Fr>, Radix2EvaluationDomain<Fr>, Fr)>,
}

// One direction's twiddles. layers[k] holds generator^(j·2^k), four j per
// quad, for the layers whose butterflies are four or more elements apart;
// `pair` is (1, t, 1, t) for the layer two apart; the layer one apart
// multiplies by one.
struct Twiddles {
    layers: Vec<Vec<Quad>>,
    pair: Quad,
}
impl Twiddles {
    fn new(mut layer: Vec<Fr>) -> Self {
        let mut layers = Vec::new();
        while layer.len() >= 4 {
            let next = layer.iter().step_by(2).copied().collect();
            layers.push(quads(&layer));
            layer = next;
        }
        debug_assert_eq!(layer.len(), 2);
        Self {
            layers,
            pair: Quad::new(&[layer[0], layer[1], layer[0], layer[1]]),
        }
    }
    fn empty() -> Self {
        Self {
            layers: Vec::new(),
            pair: Quad::one(),
        }
    }
}

/// `values` as quads, the last padded with zeros.
pub fn quads(values: &[Fr]) -> Vec<Quad> {
    values
        .par_chunks(4)
        .map(|chunk| {
            let mut four = [Fr::ZERO; 4];
            four[..chunk.len()].copy_from_slice(chunk);
            Quad::new(&four)
        })
        .collect()
}

/// The first `n` elements of `quads`.
pub fn scalars(quads: &[Quad], n: usize) -> Vec<Fr> {
    let mut values: Vec<Fr> = quads.par_iter().flat_map_iter(|quad| quad.get()).collect();
    values.truncate(n);
    values
}

impl Plan {
    pub fn new(n: usize, generator: Fr, coset: Fr, den: Fr) -> Self {
        assert!(n.is_power_of_two());
        if n < 8 {
            let mut domain = Radix2EvaluationDomain::<Fr>::new(n).unwrap();
            domain.group_gen = generator;
            domain.group_gen_inv = generator.inverse().unwrap();
            let coset_domain = domain.get_coset(coset).unwrap();
            return Self {
                n,
                roots: Twiddles::empty(),
                inverse_roots: Twiddles::empty(),
                forward_weights: Vec::new(),
                reverse_weights: Vec::new(),
                small: Some((domain, coset_domain, den)),
            };
        }
        let powers = |base: Fr, len: usize| {
            let mut out = vec![Fr::ONE; len];
            out.par_chunks_mut(256).enumerate().for_each(|(i, chunk)| {
                let mut power = base.pow([(i * 256) as u64]);
                for value in chunk {
                    *value = power;
                    power *= base;
                }
            });
            out
        };
        let n_inv = Fr::from(n as u64).inverse().unwrap();
        let mut forward_weights = powers(coset, n);
        let mut reverse_weights = powers(coset.inverse().unwrap(), n);
        for v in &mut forward_weights {
            *v *= n_inv;
        }
        let reverse_scale = n_inv * den;
        for v in &mut reverse_weights {
            *v *= reverse_scale;
        }
        let log = n.trailing_zeros();
        for i in 0..n {
            let j = i.reverse_bits() >> (usize::BITS - log);
            if i < j {
                forward_weights.swap(i, j);
                reverse_weights.swap(i, j);
            }
        }
        Self {
            n,
            roots: Twiddles::new(powers(generator, n / 2)),
            inverse_roots: Twiddles::new(powers(generator.inverse().unwrap(), n / 2)),
            forward_weights: quads(&forward_weights),
            reverse_weights: quads(&reverse_weights),
            small: None,
        }
    }
    /// The coset evaluations of the polynomial with these domain evaluations.
    pub fn transform(&self, mut values: Vec<Fr>) -> Vec<Quad> {
        assert!(values.len() <= self.n);
        values.resize(self.n, Fr::ZERO);
        if let Some((domain, coset, _)) = &self.small {
            domain.ifft_in_place(&mut values);
            coset.fft_in_place(&mut values);
            return quads(&values);
        }
        let mut values = quads(&values);
        dif(&mut values, &self.inverse_roots, 0);
        scale(&mut values, &self.forward_weights);
        dit(&mut values, &self.roots, 0);
        values
    }
    /// The quotient's coefficients, times the denominator, in bit-reversed
    /// order, from its coset evaluations.
    pub fn finish(&self, mut values: Vec<Quad>) -> Vec<Fr> {
        if let Some((_, coset, den)) = &self.small {
            let mut values = scalars(&values, self.n);
            coset.ifft_in_place(&mut values);
            for v in &mut values {
                *v *= den;
            }
            let log = self.n.trailing_zeros();
            for i in 0..self.n {
                let j = if log == 0 { 0 } else { i.reverse_bits() >> (usize::BITS - log) };
                if i < j {
                    values.swap(i, j);
                }
            }
            return values;
        }
        assert_eq!(values.len() * 4, self.n);
        dif(&mut values, &self.inverse_roots, 0);
        scale(&mut values, &self.reverse_weights);
        scalars(&values, self.n)
    }
}

// values[i] *= weights[i].
fn scale(values: &mut [Quad], weights: &[Quad]) {
    values
        .par_chunks_mut(TASK)
        .zip(weights.par_chunks(TASK))
        .for_each(|(values, weights)| {
            for (value, weight) in values.iter_mut().zip(weights) {
                *value = value.mul(weight);
            }
        });
}

// lo, hi = lo + hi, (lo - hi) * twiddle, quad by quad.
#[inline(always)]
fn dif_quads(lo: &mut [Quad], hi: &mut [Quad], twiddles: &[Quad]) {
    for j in 0..lo.len() {
        let difference = lo[j].sub(&hi[j]);
        lo[j] = lo[j].add(&hi[j]);
        hi[j] = difference.mul(&twiddles[j]);
    }
}

// lo, hi = lo + hi * twiddle, lo - hi * twiddle, quad by quad.
#[inline(always)]
fn dit_quads(lo: &mut [Quad], hi: &mut [Quad], twiddles: &[Quad]) {
    for j in 0..lo.len() {
        let product = hi[j].mul(&twiddles[j]);
        hi[j] = lo[j].sub(&product);
        lo[j] = lo[j].add(&product);
    }
}

// The layers two and one apart act within a quad: on two quads a and b at
// a time, the butterflies' two sides are gathered into one quad each,
// combined, and scattered back.
#[inline(always)]
fn dif_within(a: &mut Quad, b: &mut Quad, pair: &Quad) {
    // Two apart: (a0 a1 b0 b1) with (a2 a3 b2 b3), twiddles (1, t, 1, t).
    let (lo, hi) = (Quad::shuffle::<0, 1, 4, 5>(a, b), Quad::shuffle::<2, 3, 6, 7>(a, b));
    let sum = lo.add(&hi);
    let difference = lo.sub(&hi).mul(pair);
    *a = Quad::shuffle::<0, 1, 4, 5>(&sum, &difference);
    *b = Quad::shuffle::<2, 3, 6, 7>(&sum, &difference);
    // One apart: (a0 a2 b0 b2) with (a1 a3 b1 b3), twiddle one.
    let (lo, hi) = (Quad::shuffle::<0, 2, 4, 6>(a, b), Quad::shuffle::<1, 3, 5, 7>(a, b));
    let (sum, difference) = (lo.add(&hi), lo.sub(&hi));
    *a = Quad::shuffle::<0, 4, 1, 5>(&sum, &difference);
    *b = Quad::shuffle::<2, 6, 3, 7>(&sum, &difference);
}
#[inline(always)]
fn dit_within(a: &mut Quad, b: &mut Quad, pair: &Quad) {
    let (lo, hi) = (Quad::shuffle::<0, 2, 4, 6>(a, b), Quad::shuffle::<1, 3, 5, 7>(a, b));
    let (sum, difference) = (lo.add(&hi), lo.sub(&hi));
    *a = Quad::shuffle::<0, 4, 1, 5>(&sum, &difference);
    *b = Quad::shuffle::<2, 6, 3, 7>(&sum, &difference);
    let (lo, hi) = (Quad::shuffle::<0, 1, 4, 5>(a, b), Quad::shuffle::<2, 3, 6, 7>(a, b));
    let product = hi.mul(pair);
    let (sum, difference) = (lo.add(&product), lo.sub(&product));
    *a = Quad::shuffle::<0, 1, 4, 5>(&sum, &difference);
    *b = Quad::shuffle::<2, 3, 6, 7>(&sum, &difference);
}

// `level` selects the twiddle layer of the outermost butterflies.
fn dif(values: &mut [Quad], twiddles: &Twiddles, level: usize) {
    if values.len() > TASK {
        let mid = values.len() / 2;
        let (lo, hi) = values.split_at_mut(mid);
        lo.par_chunks_mut(TASK)
            .zip(hi.par_chunks_mut(TASK))
            .zip(twiddles.layers[level].par_chunks(TASK))
            .for_each(|((lo, hi), twiddles)| dif_quads(lo, hi, twiddles));
        rayon::join(
            || dif(lo, twiddles, level + 1),
            || dif(hi, twiddles, level + 1),
        );
    } else {
        let mut gap = values.len() / 2;
        let mut level = level;
        while gap >= 1 {
            let layer = &twiddles.layers[level][..gap];
            for chunk in values.chunks_exact_mut(gap * 2) {
                let (lo, hi) = chunk.split_at_mut(gap);
                dif_quads(lo, hi, layer);
            }
            gap /= 2;
            level += 1;
        }
        for pair in values.chunks_exact_mut(2) {
            let (a, b) = pair.split_at_mut(1);
            dif_within(&mut a[0], &mut b[0], &twiddles.pair);
        }
    }
}
fn dit(values: &mut [Quad], twiddles: &Twiddles, level: usize) {
    if values.len() > TASK {
        let mid = values.len() / 2;
        let (lo, hi) = values.split_at_mut(mid);
        rayon::join(
            || dit(lo, twiddles, level + 1),
            || dit(hi, twiddles, level + 1),
        );
        lo.par_chunks_mut(TASK)
            .zip(hi.par_chunks_mut(TASK))
            .zip(twiddles.layers[level].par_chunks(TASK))
            .for_each(|((lo, hi), twiddles)| dit_quads(lo, hi, twiddles));
    } else {
        for pair in values.chunks_exact_mut(2) {
            let (a, b) = pair.split_at_mut(1);
            dit_within(&mut a[0], &mut b[0], &twiddles.pair);
        }
        // Butterflies `gap` quads apart use the layer of stride n / (8 gap).
        let mut gap = 1;
        while gap < values.len() {
            let layer = &twiddles.layers[twiddles.layers.len() - 1 - gap.trailing_zeros() as usize][..gap];
            for chunk in values.chunks_exact_mut(gap * 2) {
                let (lo, hi) = chunk.split_at_mut(gap);
                dit_quads(lo, hi, layer);
            }
            gap *= 2;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_arkworks() {
        for n in [1, 2, 4, 8, 16, 1024, 2048, 32768] {
            for root_power in [1, 3] {
                let mut domain = Radix2EvaluationDomain::<Fr>::new(n).unwrap();
                domain.group_gen = domain.group_gen.pow([root_power]);
                domain.group_gen_inv = domain.group_gen.inverse().unwrap();
                for offset in [5, 7] {
                    let coset = domain.get_coset(Fr::from(offset as u64)).unwrap();
                    let den = (coset.offset.pow([n as u64]) - Fr::ONE).inverse().unwrap();
                    let plan = Plan::new(n, domain.group_gen, coset.offset, den);
                    for len in [0, n / 2, n] {
                        let input: Vec<Fr> = (0..len)
                            .map(|i| match i % 4 {
                                0 => Fr::ZERO,
                                1 => -Fr::ONE,
                                _ => Fr::from(i as u64 + 1).pow([127]),
                            })
                            .collect();
                        let mut expected = input.clone();
                        expected.resize(n, Fr::ZERO);
                        domain.ifft_in_place(&mut expected);
                        coset.fft_in_place(&mut expected);
                        assert_eq!(scalars(&plan.transform(input), n), expected, "n {n} len {len}");
                        let actual = plan.finish(quads(&expected));
                        coset.ifft_in_place(&mut expected);
                        for v in &mut expected {
                            *v *= den;
                        }
                        let log = n.trailing_zeros();
                        for (i, value) in actual.iter().enumerate() {
                            let j = if log == 0 {
                                0
                            } else {
                                i.reverse_bits() >> (usize::BITS - log)
                            };
                            assert_eq!(*value, expected[j], "n {n} len {len} i {i}");
                        }
                    }
                }
            }
        }
    }
}
