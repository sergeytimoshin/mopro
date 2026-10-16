use ark_bn254::Fr;
use ark_ff::{AdditiveGroup, Field};
use rayon::prelude::*;

use crate::simd::fr_mul4;

// Elements per parallel task in the wide passes.
const TASK: usize = 1024;

// Cached twiddles and coset powers for a prepared proving key. An inverse
// DIF transform emits bit-reversed coefficients; the forward DIT transform
// consumes that order directly. The final inverse transform already matches
// gnark's bit-reversed Z key, so no proof-time permutation is needed.
pub struct Plan {
    n: usize,
    // roots[k][j] is generator^(j·2^k): each layer's twiddles in one
    // contiguous table, since strided reads of a single table miss the cache
    // on every access in the deep layers of a wide domain.
    roots: Vec<Vec<Fr>>,
    inverse_roots: Vec<Vec<Fr>>,
    forward_weights: Vec<Fr>,
    reverse_weights: Vec<Fr>,
}
impl Plan {
    pub fn new(n: usize, generator: Fr, coset: Fr, den: Fr) -> Self {
        assert!(n.is_power_of_two());
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
        let layers = |mut layer: Vec<Fr>| {
            let mut out = Vec::with_capacity(n.trailing_zeros() as usize);
            for _ in 0..n.trailing_zeros() {
                let next = layer.iter().step_by(2).copied().collect();
                out.push(layer);
                layer = next;
            }
            out
        };
        let roots = layers(powers(generator, n / 2));
        let inverse_roots = layers(powers(generator.inverse().unwrap(), n / 2));
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
        if log > 0 {
            for i in 0..n {
                let j = i.reverse_bits() >> (usize::BITS - log);
                if i < j {
                    forward_weights.swap(i, j);
                    reverse_weights.swap(i, j);
                }
            }
        }
        Self {
            n,
            roots,
            inverse_roots,
            forward_weights,
            reverse_weights,
        }
    }
    pub fn transform(&self, mut values: Vec<Fr>) -> Vec<Fr> {
        assert!(values.len() <= self.n);
        values.resize(self.n, Fr::ZERO);
        dif(&mut values, &self.inverse_roots, 0);
        scale(&mut values, &self.forward_weights);
        dit(&mut values, &self.roots, 0);
        values
    }
    pub fn finish(&self, values: &mut [Fr]) {
        assert_eq!(values.len(), self.n);
        dif(values, &self.inverse_roots, 0);
        scale(values, &self.reverse_weights);
    }
}

// values[i] *= weights[i], four lanes at a time.
fn scale(values: &mut [Fr], weights: &[Fr]) {
    values
        .par_chunks_mut(TASK)
        .zip(weights.par_chunks(TASK))
        .for_each(|(values, weights)| {
            let full = values.len() / 4 * 4;
            for j in (0..full).step_by(4) {
                let product = fr_mul4(&lanes(values, j), &lanes(weights, j));
                values[j..j + 4].copy_from_slice(&product);
            }
            for j in full..values.len() {
                values[j] *= weights[j];
            }
        });
}

#[inline(always)]
fn lanes(values: &[Fr], j: usize) -> [Fr; 4] {
    core::array::from_fn(|i| values[j + i])
}

// lo, hi = lo + hi, (lo - hi) * twiddle. A layer's first twiddle is one,
// which the four-lane products multiply by and the scalar tail skips.
#[inline(always)]
fn dif_butterflies(lo: &mut [Fr], hi: &mut [Fr], twiddles: &[Fr], first: bool) {
    let full = lo.len() / 4 * 4;
    for j in (0..full).step_by(4) {
        let difference: [Fr; 4] = core::array::from_fn(|i| lo[j + i] - hi[j + i]);
        for i in 0..4 {
            lo[j + i] += hi[j + i];
        }
        hi[j..j + 4].copy_from_slice(&fr_mul4(&difference, &lanes(twiddles, j)));
    }
    for j in full..lo.len() {
        let difference = lo[j] - hi[j];
        lo[j] += hi[j];
        hi[j] = if first && j == 0 {
            difference
        } else {
            difference * twiddles[j]
        };
    }
}

// lo, hi = lo + hi * twiddle, lo - hi * twiddle.
#[inline(always)]
fn dit_butterflies(lo: &mut [Fr], hi: &mut [Fr], twiddles: &[Fr], first: bool) {
    let full = lo.len() / 4 * 4;
    for j in (0..full).step_by(4) {
        let product = fr_mul4(&lanes(hi, j), &lanes(twiddles, j));
        for i in 0..4 {
            hi[j + i] = lo[j + i] - product[i];
            lo[j + i] += product[i];
        }
    }
    for j in full..lo.len() {
        let product = if first && j == 0 {
            hi[j]
        } else {
            hi[j] * twiddles[j]
        };
        hi[j] = lo[j] - product;
        lo[j] += product;
    }
}

// `level` selects the twiddle layer of the outermost butterflies.
fn dif(values: &mut [Fr], roots: &[Vec<Fr>], level: usize) {
    if values.len() > 1024 {
        let mid = values.len() / 2;
        let (lo, hi) = values.split_at_mut(mid);
        lo.par_chunks_mut(TASK)
            .zip(hi.par_chunks_mut(TASK))
            .zip(roots[level].par_chunks(TASK))
            .enumerate()
            .for_each(|(task, ((lo, hi), twiddles))| dif_butterflies(lo, hi, twiddles, task == 0));
        rayon::join(|| dif(lo, roots, level + 1), || dif(hi, roots, level + 1));
    } else {
        let mut gap = values.len() / 2;
        let mut level = level;
        while gap > 0 {
            let twiddles = &roots[level][..gap];
            for chunk in values.chunks_exact_mut(gap * 2) {
                let (lo, hi) = chunk.split_at_mut(gap);
                dif_butterflies(lo, hi, twiddles, true);
            }
            gap /= 2;
            level += 1;
        }
    }
}
fn dit(values: &mut [Fr], roots: &[Vec<Fr>], level: usize) {
    if values.len() > 1024 {
        let mid = values.len() / 2;
        let (lo, hi) = values.split_at_mut(mid);
        rayon::join(|| dit(lo, roots, level + 1), || dit(hi, roots, level + 1));
        lo.par_chunks_mut(TASK)
            .zip(hi.par_chunks_mut(TASK))
            .zip(roots[level].par_chunks(TASK))
            .enumerate()
            .for_each(|(task, ((lo, hi), twiddles))| dit_butterflies(lo, hi, twiddles, task == 0));
    } else {
        // Butterflies `gap` apart use the layer of stride n / (2 gap).
        let mut gap = 1;
        while gap < values.len() {
            let twiddles = &roots[roots.len() - 1 - gap.trailing_zeros() as usize][..gap];
            for chunk in values.chunks_exact_mut(gap * 2) {
                let (lo, hi) = chunk.split_at_mut(gap);
                dit_butterflies(lo, hi, twiddles, true);
            }
            gap *= 2;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_poly::{EvaluationDomain, Radix2EvaluationDomain};

    #[test]
    fn matches_arkworks() {
        for n in [1, 2, 4, 16, 1024, 2048, 32768] {
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
                        domain.ifft_in_place(&mut expected);
                        coset.fft_in_place(&mut expected);
                        assert_eq!(plan.transform(input), expected);
                        let mut actual = expected.clone();
                        plan.finish(&mut actual);
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
                            assert_eq!(*value, expected[j]);
                        }
                    }
                }
            }
        }
    }
}
