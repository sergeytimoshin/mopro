// Modified for Mopro: preserve field constants and representation, using
// mixed-radix multiplication for browser WASM. Original arkworks code is
// licensed under MIT OR Apache-2.0; see the accompanying license files.
use ark_ff::fields::{Fp256, MontBackend, MontConfig};

#[derive(MontConfig)]
#[modulus = "21888242871839275222246405745257275088696311157297823662689037894645226208583"]
#[generator = "3"]
pub struct OriginalFqConfig;
pub type Fq = Fp256<MontBackend<FqConfig, 4>>;

pub struct FqConfig;
impl MontConfig<4> for FqConfig {
    const MODULUS: ark_ff::BigInt<4> = OriginalFqConfig::MODULUS;
    const GENERATOR: Fq = Fq::new_unchecked(OriginalFqConfig::GENERATOR.0);
    const TWO_ADIC_ROOT_OF_UNITY: Fq =
        Fq::new_unchecked(OriginalFqConfig::TWO_ADIC_ROOT_OF_UNITY.0);
    #[inline(always)]
    fn mul_assign(a: &mut Fq, b: &Fq) {
        a.0 = crate::mont29::multiply(a.0, b.0, Self::MODULUS, Self::INV)
    }
    #[inline(always)]
    fn square_in_place(a: &mut Fq) {
        a.0 = crate::mont29::multiply(a.0, a.0, Self::MODULUS, Self::INV)
    }
    #[inline(always)]
    fn sum_of_products<const M: usize>(a: &[Fq; M], b: &[Fq; M]) -> Fq {
        if M == 2 {
            return Fq::new_unchecked(crate::mont29::sum2(
                [a[0].0, a[1].0],
                [b[0].0, b[1].0],
                Self::MODULUS,
                Self::INV,
            ));
        }
        let mut out = Fq::new_unchecked(ark_ff::BigInt([0; 4]));
        for i in 0..M {
            out += a[i] * b[i]
        }
        out
    }
}
#[cfg(test)]
mod mont_tests {
    use super::*;
    use ark_ff::{AdditiveGroup, BigInt, BigInteger, Field, UniformRand};
    type Original = Fp256<MontBackend<OriginalFqConfig, 4>>;

    #[test]
    fn sum_of_products_boundaries() {
        let mut last = OriginalFqConfig::MODULUS;
        last.sub_with_borrow(&BigInt::from(1u64));
        let values = [
            Original::ZERO,
            Original::ONE,
            -Original::ONE,
            Original::new_unchecked(last),
            Original::new_unchecked(BigInt([u64::MAX, u64::MAX, u64::MAX, 0])),
        ];
        let convert = |x: Original| Fq::new_unchecked(x.0);
        assert_eq!(Fq::sum_of_products(&[], &[]), Fq::ZERO);
        for a in values {
            for b in values {
                assert_eq!(
                    Fq::sum_of_products(&[convert(a)], &[convert(b)]).0,
                    (a * b).0
                );
                for c in values {
                    for d in values {
                        assert_eq!(
                            Fq::sum_of_products(
                                &[convert(a), convert(c)],
                                &[convert(b), convert(d)]
                            )
                            .0,
                            (a * b + c * d).0
                        );
                        assert_eq!(
                            Fq::sum_of_products(
                                &[convert(a), convert(c), convert(a)],
                                &[convert(b), convert(d), convert(d)]
                            )
                            .0,
                            (a * b + c * d + a * d).0
                        );
                    }
                }
            }
        }
    }
    #[test]
    fn reference() {
        let mut rng = ark_std::test_rng();
        for _ in 0..10000 {
            let a = Original::rand(&mut rng);
            let b = Original::rand(&mut rng);
            let x = Fq::new_unchecked(a.0);
            let y = Fq::new_unchecked(b.0);
            assert_eq!((x * y).0, (a * b).0);
            let c = Original::rand(&mut rng);
            let d = Original::rand(&mut rng);
            assert_eq!(
                Fq::sum_of_products(&[x, Fq::new_unchecked(c.0)], &[y, Fq::new_unchecked(d.0)]).0,
                (a * b + c * d).0
            );
            assert_eq!(x.square().0, a.square().0);
        }
    }
}
