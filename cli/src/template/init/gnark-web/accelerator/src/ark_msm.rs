use ark_bn254::{Fr, G1Affine, G1Projective, G2Affine, G2Projective};
use ark_ec::{AdditiveGroup, VariableBaseMSM};
pub type G1 = G1Affine;
pub type G2 = G2Affine;
pub fn init() -> bool {
    true
}
pub fn g1(p: G1Affine) -> G1 {
    p
}
pub fn g2(p: G2Affine) -> G2 {
    p
}
pub fn scalars(values: &[Fr]) -> std::borrow::Cow<'_, [Fr]> {
    std::borrow::Cow::Borrowed(values)
}
pub fn msm_g1(bases: &mut [G1], scalars: &[Fr]) -> G1Projective {
    if bases.is_empty() {
        assert!(scalars.is_empty());
        G1Projective::ZERO
    } else {
        G1Projective::msm(bases, scalars).expect("validated MSM dimensions")
    }
}
pub fn msm_g2(bases: &mut [G2], scalars: &[Fr]) -> G2Projective {
    if bases.is_empty() {
        assert!(scalars.is_empty());
        G2Projective::ZERO
    } else {
        G2Projective::msm(bases, scalars).expect("validated MSM dimensions")
    }
}
