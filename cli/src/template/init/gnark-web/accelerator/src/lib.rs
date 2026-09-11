use ark_bn254::{Fq, Fq2, Fr, G1Affine, G1Projective, G2Affine, G2Projective};
use ark_ec::{AdditiveGroup, CurveGroup, VariableBaseMSM};
use ark_ff::{BigInt, Field};
use ark_poly::{EvaluationDomain, Radix2EvaluationDomain};
use rayon::prelude::*;
use wasm_bindgen::prelude::*;
pub use wasm_bindgen_rayon::init_thread_pool;
mod fft;
#[cfg(feature = "hybrid")]
mod hybrid;
mod timing;

fn words(data: &[u8]) -> BigInt<4> {
    BigInt(core::array::from_fn(|i| {
        u64::from_le_bytes(data[i * 8..i * 8 + 8].try_into().unwrap())
    }))
}
fn fq(data: &[u8]) -> Fq {
    Fq::new_unchecked(words(data))
}
fn frs(data: &[u8]) -> Vec<Fr> {
    data.as_chunks::<32>()
        .0
        .iter()
        .map(|x| Fr::new_unchecked(words(x)))
        .collect()
}
fn g1s(data: &[u8]) -> Vec<G1Affine> {
    data.as_chunks::<64>()
        .0
        .iter()
        .map(|x| {
            let px = fq(x);
            let py = fq(&x[32..]);
            if px == Fq::ZERO && py == Fq::ZERO {
                G1Affine::identity()
            } else {
                G1Affine::new_unchecked(px, py)
            }
        })
        .collect()
}
fn g2s(data: &[u8]) -> Vec<G2Affine> {
    data.as_chunks::<128>()
        .0
        .iter()
        .map(|x| {
            let px = Fq2::new(fq(x), fq(&x[32..]));
            let py = Fq2::new(fq(&x[64..]), fq(&x[96..]));
            if px == Fq2::ZERO && py == Fq2::ZERO {
                G2Affine::identity()
            } else {
                G2Affine::new_unchecked(px, py)
            }
        })
        .collect()
}
fn output_fq(out: &mut Vec<u8>, v: Fq) {
    for w in v.0 .0 {
        out.extend_from_slice(&w.to_le_bytes())
    }
}
fn output_g1(out: &mut Vec<u8>, p: G1Projective) {
    let p = p.into_affine();
    if p.infinity {
        out.extend_from_slice(&[0; 64]);
    } else {
        output_fq(out, p.x);
        output_fq(out, p.y);
    }
}
fn output_g2(out: &mut Vec<u8>, p: G2Projective) {
    let p = p.into_affine();
    if p.infinity {
        out.extend_from_slice(&[0; 128]);
    } else {
        output_fq(out, p.x.c0);
        output_fq(out, p.x.c1);
        output_fq(out, p.y.c0);
        output_fq(out, p.y.c1);
    }
}

#[wasm_bindgen]
pub struct Key {
    a: Vec<G1Affine>,
    b: Vec<G1Affine>,
    k: Vec<G1Affine>,
    z: Vec<G1Affine>,
    b2: Vec<G2Affine>,
    domain: Radix2EvaluationDomain<Fr>,
    plan: fft::Plan,
    commitments: Vec<(Vec<G1Affine>, Vec<G1Affine>)>,
    profile: [f64; 9],
}
#[wasm_bindgen]
impl Key {
    #[wasm_bindgen(constructor)]
    pub fn new(
        a: &[u8],
        b: &[u8],
        k: &[u8],
        z: &[u8],
        b2: &[u8],
        domain_params: &[u8],
    ) -> Result<Key, JsError> {
        if [a, b, k, z].iter().any(|v| !v.len().is_multiple_of(64))
            || !b2.len().is_multiple_of(128)
            || b.len() / 64 != b2.len() / 128
            || domain_params.len() != 64
        {
            return Err(JsError::new("invalid gnark key dimensions"));
        }
        let n = z.len() / 64 + 1;
        if !n.is_power_of_two() {
            return Err(JsError::new("invalid gnark FFT domain"));
        }
        let params = frs(domain_params);
        let mut domain = Radix2EvaluationDomain::<Fr>::new(n)
            .ok_or_else(|| JsError::new("unsupported FFT domain"))?;
        domain.group_gen = params[0];
        domain.group_gen_inv = params[0]
            .inverse()
            .ok_or_else(|| JsError::new("invalid FFT generator"))?;
        let coset = domain
            .get_coset(params[1])
            .ok_or_else(|| JsError::new("invalid FFT coset"))?;
        let den = (params[1].pow([n as u64]) - Fr::ONE)
            .inverse()
            .ok_or_else(|| JsError::new("invalid quotient denominator"))?;
        Ok(Self {
            a: g1s(a),
            b: g1s(b),
            k: g1s(k),
            z: g1s(z),
            b2: g2s(b2),
            domain,
            plan: fft::Plan::new(domain, coset, den),
            commitments: Vec::new(),
            profile: [0.; 9],
        })
    }
    pub fn profile(&self) -> String {
        let names = [
            "rustDecode",
            "fft",
            "msmWall",
            "g1A",
            "g1B",
            "g1K",
            "g1Z",
            "g2B",
            "rustEncode",
        ];
        let entries: Vec<_> = names
            .iter()
            .zip(self.profile)
            .map(|(key, value)| format!("\"{key}\":{value}"))
            .collect();
        format!("{{{}}}", entries.join(","))
    }
    pub fn add_commitment(&mut self, basis: &[u8], sigma: &[u8]) -> Result<(), JsError> {
        if !basis.len().is_multiple_of(64) || basis.len() != sigma.len() {
            return Err(JsError::new("invalid commitment basis"));
        }
        self.commitments.push((g1s(basis), g1s(sigma)));
        Ok(())
    }
    pub fn commitment(
        &self,
        index: usize,
        knowledge: bool,
        values: &[u8],
    ) -> Result<Vec<u8>, JsError> {
        let key = self
            .commitments
            .get(index)
            .ok_or_else(|| JsError::new("unknown commitment key"))?;
        let basis = if knowledge { &key.1 } else { &key.0 };
        if values.len() != basis.len() * 32 {
            return Err(JsError::new("commitment witness length mismatch"));
        }
        let point = msm_g1(basis, &frs(values));
        let mut out = Vec::with_capacity(64);
        output_g1(&mut out, point);
        Ok(out)
    }

    pub fn parts(
        &mut self,
        sa: &[u8],
        sb: &[u8],
        sk: &[u8],
        a: &[u8],
        b: &[u8],
        c: &[u8],
    ) -> Result<Vec<u8>, JsError> {
        if sa.len() != self.a.len() * 32
            || sb.len() != self.b.len() * 32
            || sk.len() != self.k.len() * 32
            || a.len() != b.len()
            || a.len() != c.len()
            || !a.len().is_multiple_of(32)
            || a.len() / 32 > self.domain.size()
        {
            return Err(JsError::new("witness does not match prepared gnark key"));
        }
        let decode = timing::now();
        let sa = frs(sa);
        let sb = frs(sb);
        let sk = frs(sk);
        let (a, b, c) = (frs(a), frs(b), frs(c));
        let decode = timing::now() - decode;
        let (out, mut profile) = self.compute_parts(&sa, &sb, &sk, a, b, c);
        profile[0] = decode;
        self.profile = profile;
        Ok(out)
    }
}
impl Key {
    fn compute_parts(
        &self,
        sa: &[Fr],
        sb: &[Fr],
        sk: &[Fr],
        a: Vec<Fr>,
        b: Vec<Fr>,
        c: Vec<Fr>,
    ) -> (Vec<u8>, [f64; 9]) {
        let (h, fft_ms) = timing::run(|| self.quotient(a, b, c));
        let msm_start = timing::now();
        let ((ar, bs1), (kr, (hz, bs2))) = rayon::join(
            || {
                rayon::join(
                    || timing::run(|| msm_g1(&self.a, sa)),
                    || timing::run(|| msm_g1(&self.b, sb)),
                )
            },
            || {
                rayon::join(
                    || timing::run(|| msm_g1(&self.k, sk)),
                    || {
                        rayon::join(
                            || timing::run(|| msm_g1(&self.z, &h)),
                            || timing::run(|| msm_g2(&self.b2, sb)),
                        )
                    },
                )
            },
        );
        let msm_ms = timing::now() - msm_start;
        let encode = timing::now();
        let mut out = Vec::with_capacity(384);
        for p in [ar.0, bs1.0, kr.0, hz.0] {
            output_g1(&mut out, p)
        }
        output_g2(&mut out, bs2.0);
        (
            out,
            [
                0.,
                fft_ms,
                msm_ms,
                ar.1,
                bs1.1,
                kr.1,
                hz.1,
                bs2.1,
                timing::now() - encode,
            ],
        )
    }
    fn quotient(&self, a: Vec<Fr>, b: Vec<Fr>, c: Vec<Fr>) -> Vec<Fr> {
        let transform = |values| self.plan.transform(values);
        let (mut a, (b, c)) = rayon::join(
            || transform(a),
            || rayon::join(|| transform(b), || transform(c)),
        );
        a.par_iter_mut()
            .zip(&b)
            .zip(&c)
            .for_each(|((a, b), c)| *a = *a * b - c);
        // The plan applies the quotient denominator and leaves coefficients
        // in the bit-reversed order of gnark's Z key.
        self.plan.finish(&mut a);
        a.pop();
        a
    }
}

fn msm_g1(bases: &[G1Affine], scalars: &[Fr]) -> G1Projective {
    if bases.is_empty() {
        G1Projective::ZERO
    } else {
        G1Projective::msm(bases, scalars).expect("validated MSM dimensions")
    }
}
fn msm_g2(bases: &[G2Affine], scalars: &[Fr]) -> G2Projective {
    if bases.is_empty() {
        G2Projective::ZERO
    } else {
        G2Projective::msm(bases, scalars).expect("validated MSM dimensions")
    }
}
