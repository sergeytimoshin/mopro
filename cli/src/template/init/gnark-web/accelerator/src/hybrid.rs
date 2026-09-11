// Arithmetic and encoding helpers for the opt-in JavaScript MSM experiments.
use super::*;
use ark_ff::PrimeField;

#[wasm_bindgen]
pub struct HybridKey {
    b2: Vec<G2Affine>,
    domain: Radix2EvaluationDomain<Fr>,
    plan: fft::Plan,
}
#[wasm_bindgen]
impl HybridKey {
    #[wasm_bindgen(constructor)]
    pub fn new(b2: &[u8], n: usize, params: &[u8]) -> Result<Self, JsError> {
        if !b2.len().is_multiple_of(128) || !n.is_power_of_two() || params.len() != 64 {
            return Err(JsError::new("invalid hybrid key dimensions"));
        }
        let p = frs(params);
        let mut domain =
            Radix2EvaluationDomain::<Fr>::new(n).ok_or_else(|| JsError::new("invalid FFT size"))?;
        domain.group_gen = p[0];
        domain.group_gen_inv = p[0]
            .inverse()
            .ok_or_else(|| JsError::new("zero FFT root"))?;
        let coset = domain
            .get_coset(p[1])
            .ok_or_else(|| JsError::new("invalid coset"))?;
        let den = (p[1].pow([n as u64]) - Fr::ONE)
            .inverse()
            .ok_or_else(|| JsError::new("invalid quotient denominator"))?;
        Ok(Self {
            b2: g2s(b2),
            domain,
            plan: fft::Plan::new(domain, coset, den),
        })
    }
    pub fn quotient(&self, a: &[u8], b: &[u8], c: &[u8]) -> Result<Vec<u8>, JsError> {
        if a.len() != b.len()
            || a.len() != c.len()
            || !a.len().is_multiple_of(32)
            || a.len() / 32 > self.domain.size()
        {
            return Err(JsError::new("invalid quotient input dimensions"));
        }
        let (mut a, (b, c)) = rayon::join(
            || self.plan.transform(frs(a)),
            || {
                rayon::join(
                    || self.plan.transform(frs(b)),
                    || self.plan.transform(frs(c)),
                )
            },
        );
        a.par_iter_mut()
            .zip(b)
            .zip(c)
            .for_each(|((a, b), c)| *a = *a * b - c);
        self.plan.finish(&mut a);
        a.pop();
        Ok(a.iter()
            .flat_map(|v| v.0 .0.iter().flat_map(|w| w.to_le_bytes()))
            .collect())
    }
    pub fn g2(&self, values: &[u8]) -> Result<Vec<u8>, JsError> {
        if values.len() != self.b2.len() * 32 {
            return Err(JsError::new("G2 scalar length mismatch"));
        }
        let mut out = Vec::with_capacity(128);
        output_g2(&mut out, msm_g2(&self.b2, &frs(values)));
        Ok(out)
    }
}
#[wasm_bindgen]
pub fn canonical_fields(data: &[u8], base: bool) -> Result<Vec<u8>, JsError> {
    if !data.len().is_multiple_of(32) {
        return Err(JsError::new("invalid field byte length"));
    }
    Ok(data
        .as_chunks::<32>()
        .0
        .iter()
        .flat_map(|v| {
            let words = if base {
                fq(v).into_bigint()
            } else {
                Fr::new_unchecked(words(v)).into_bigint()
            };
            words.0.into_iter().flat_map(|w| w.to_le_bytes())
        })
        .collect())
}
#[wasm_bindgen]
pub fn montgomery_fields(data: &[u8], base: bool) -> Result<Vec<u8>, JsError> {
    if !data.len().is_multiple_of(32) {
        return Err(JsError::new("invalid field byte length"));
    }
    let mut out = Vec::with_capacity(data.len());
    for v in data.as_chunks::<32>().0 {
        let n = words(v);
        let n = if base {
            Fq::from_bigint(n).map(|v| v.0)
        } else {
            Fr::from_bigint(n).map(|v| v.0)
        }
        .ok_or_else(|| JsError::new("noncanonical field element"))?;
        for w in n.0 {
            out.extend_from_slice(&w.to_le_bytes());
        }
    }
    Ok(out)
}
