use super::*;
use ark_ec::{AffineRepr, VariableBaseMSM};

pub fn check(n: usize) -> Result<(), String> {
    assert!(msm::init());
    let a: Vec<_> = (0..n)
        .map(|i| (G1Affine::generator() * Fr::from((i % 97) as u64)).into_affine())
        .collect();
    let b: Vec<_> = (0..n)
        .map(|i| (G2Affine::generator() * Fr::from((i % 97) as u64)).into_affine())
        .collect();
    let mut ma: Vec<_> = a.iter().copied().map(msm::g1).collect();
    let mut mb: Vec<_> = b.iter().copied().map(msm::g2).collect();
    // Reuse keys with zero, one, negative and full-width scalars. In WASM the
    // n=3 negative-scalar case catches missing linked C++ constructors.
    for round in 0..3 {
        let s: Vec<_> = (0..n)
            .map(|i| match (i + round) % 4 {
                0 => Fr::ZERO,
                1 => Fr::ONE,
                2 => -Fr::ONE,
                _ => Fr::from((i + 5) as u64).inverse().unwrap(),
            })
            .collect();
        let scalars = msm::scalars(&s);
        let (got1, got2) = rayon::join(|| msm_g1(&mut ma, &scalars), || msm_g2(&mut mb, &scalars));
        if got1 != G1Projective::msm(&a, &s).unwrap() {
            return Err(format!("G1 mismatch n={n}, round={round}"));
        }
        if got2 != G2Projective::msm(&b, &s).unwrap() {
            return Err(format!("G2 mismatch n={n}, round={round}"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn msm_matches_arkworks_and_reuses_keys_across_threads() {
        for threads in [1, 4] {
            let pool = rayon::ThreadPoolBuilder::new()
                .num_threads(threads)
                .build()
                .unwrap();
            pool.install(|| {
                for n in [0, 1, 3, 31, 1025, 4097] {
                    check(n).unwrap();
                }
            });
        }
    }
    #[test]
    #[should_panic(expected = "assertion `left == right` failed")]
    fn mismatched_msm_is_rejected_before_ffi() {
        assert!(msm::init());
        let _ = msm_g1(&mut [msm::G1::zero()], &[]);
    }
}
