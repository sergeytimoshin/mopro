// Modified for Mopro: preserve field constants and representation, using
// mixed-radix multiplication for browser WASM. Original arkworks code is
// licensed under MIT OR Apache-2.0; see the accompanying license files.
use ark_ff::fields::{Fp256, MontBackend, MontConfig};

#[derive(MontConfig)]
#[modulus = "21888242871839275222246405745257275088548364400416034343698204186575808495617"]
#[generator = "5"]
#[small_subgroup_base = "3"]
#[small_subgroup_power = "2"]
pub struct OriginalFrConfig;
pub type Fr = Fp256<MontBackend<FrConfig, 4>>;

pub struct FrConfig;
impl MontConfig<4> for FrConfig {
    const MODULUS: ark_ff::BigInt<4> = OriginalFrConfig::MODULUS;
    const GENERATOR: Fr = Fr::new_unchecked(OriginalFrConfig::GENERATOR.0);
    const TWO_ADIC_ROOT_OF_UNITY: Fr =
        Fr::new_unchecked(OriginalFrConfig::TWO_ADIC_ROOT_OF_UNITY.0);
    const SMALL_SUBGROUP_BASE: Option<u32> = OriginalFrConfig::SMALL_SUBGROUP_BASE;
    const SMALL_SUBGROUP_BASE_ADICITY: Option<u32> = OriginalFrConfig::SMALL_SUBGROUP_BASE_ADICITY;
    const LARGE_SUBGROUP_ROOT_OF_UNITY: Option<Fr> =
        match OriginalFrConfig::LARGE_SUBGROUP_ROOT_OF_UNITY {
            Some(v) => Some(Fr::new_unchecked(v.0)),
            None => None,
        };
    #[inline(always)]
    fn mul_assign(a: &mut Fr, b: &Fr) {
        a.0 = crate::mont29::multiply(a.0, b.0, Self::MODULUS, Self::INV)
    }
    #[inline(always)]
    fn square_in_place(a: &mut Fr) {
        a.0 = crate::mont29::multiply(a.0, a.0, Self::MODULUS, Self::INV)
    }
}
#[cfg(test)]
mod mont_tests {
    use super::*;
    use ark_ff::{Field, UniformRand};
    #[test]
    fn reference() {
        let mut rng = ark_std::test_rng();
        type Original = Fp256<MontBackend<OriginalFrConfig, 4>>;
        for _ in 0..10000 {
            let a = Original::rand(&mut rng);
            let b = Original::rand(&mut rng);
            let x = Fr::new_unchecked(a.0);
            let y = Fr::new_unchecked(b.0);
            assert_eq!((x * y).0, (a * b).0);
            assert_eq!(x.square().0, a.square().0);
        }
    }
}
