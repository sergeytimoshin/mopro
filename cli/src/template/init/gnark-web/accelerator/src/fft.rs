use ark_bn254::Fr;
use ark_poly::{EvaluationDomain, Radix2EvaluationDomain};
use rayon::prelude::*;

// Gnark supplies its domain generator and coset. Arkworks returns coefficients
// in natural order; gnark's proving key stores the Z bases in bit-reversed order.
pub struct Plan {
    domain: Radix2EvaluationDomain<Fr>,
    coset: Radix2EvaluationDomain<Fr>,
    denominator: Fr,
}
impl Plan {
    pub fn new(
        domain: Radix2EvaluationDomain<Fr>,
        coset: Radix2EvaluationDomain<Fr>,
        denominator: Fr,
    ) -> Self {
        Self {
            domain,
            coset,
            denominator,
        }
    }
    pub fn transform(&self, mut values: Vec<Fr>) -> Vec<Fr> {
        self.domain.ifft_in_place(&mut values);
        self.coset.fft_in_place(&mut values);
        values
    }
    pub fn finish(&self, values: &mut Vec<Fr>) {
        self.coset.ifft_in_place(values);
        values.par_iter_mut().for_each(|v| *v *= self.denominator);
        let log = self.domain.log_size_of_group;
        if log > 0 {
            for i in 0..values.len() {
                let j = i.reverse_bits() >> (usize::BITS - log);
                if i < j {
                    values.swap(i, j);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ark_ff::Field;

    #[test]
    fn quotient_matches_gnark_key_order() {
        for n in [2, 4, 16, 1024] {
            for root_power in [1, 3] {
                let mut domain = Radix2EvaluationDomain::<Fr>::new(n).unwrap();
                domain.group_gen = domain.group_gen.pow([root_power]);
                domain.group_gen_inv = domain.group_gen.inverse().unwrap();
                let coset = domain.get_coset(Fr::from(5u64)).unwrap();
                let den = (coset.offset.pow([n as u64]) - Fr::ONE).inverse().unwrap();
                let plan = Plan::new(domain, coset, den);
                // A = X^(n-1), B = X^(n-1) + 2X, C = X^(n-2) + 2.
                // Therefore (A*B-C)/(X^n-1) = X^(n-2) + 2.
                let points: Vec<_> = domain.elements().collect();
                let a = plan.transform(points.iter().map(|x| x.pow([(n - 1) as u64])).collect());
                let b = plan.transform(
                    points
                        .iter()
                        .map(|x| x.pow([(n - 1) as u64]) + *x + x)
                        .collect(),
                );
                let c = plan.transform(
                    points
                        .iter()
                        .map(|x| x.pow([(n - 2) as u64]) + Fr::from(2u64))
                        .collect(),
                );
                let mut quotient: Vec<_> = a
                    .iter()
                    .zip(b)
                    .zip(c)
                    .map(|((a, b), c)| *a * b - c)
                    .collect();
                plan.finish(&mut quotient);
                for (i, value) in quotient.iter().enumerate() {
                    let degree = i.reverse_bits() >> (usize::BITS - n.trailing_zeros());
                    let expected =
                        Fr::from(u64::from(degree == n - 2) + 2 * u64::from(degree == 0));
                    assert_eq!(
                        *value, expected,
                        "n={n}, degree={degree}, root={root_power}"
                    );
                }
            }
        }
    }
}
