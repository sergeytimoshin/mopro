//! Decompression of gnark's compressed BN254 points on the thread pool. A key
//! file stores every point compressed, and reading it costs a square root per
//! point, which the single-threaded Go wasm takes seconds to do for a wide
//! key. The bridge hands the compressed points here and gets gnark's raw
//! encoding back, which it decodes without square roots.
//!
//! gnark's compressed encoding is the big-endian x coordinate with two flag
//! bits in the first byte: `10` for the lexicographically smaller y, `11` for
//! the larger, `01` for the point at infinity (the rest zero). A G2 point
//! stores x as `A1 | A0`. The raw encoding is x then y, each coordinate in the
//! same order, and the point at infinity is all zero.
use ark_bn254::{g1, g2, Fq, Fq2};
use ark_ec::short_weierstrass::SWCurveConfig;
use ark_ff::{BigInteger, Field, PrimeField, Zero};
use rayon::prelude::*;
use wasm_bindgen::prelude::*;

const MASK: u8 = 0b11 << 6;
const SMALLEST: u8 = 0b10 << 6;
const LARGEST: u8 = 0b11 << 6;
const INFINITY: u8 = 0b01 << 6;

/// `compressed` G1 points (32 bytes each) as raw points (64 bytes each).
#[wasm_bindgen]
pub fn decompress_g1(compressed: &[u8]) -> Result<Vec<u8>, JsError> {
    g1(compressed).map_err(JsError::new)
}

/// `compressed` G2 points (64 bytes each) as raw points (128 bytes each).
#[wasm_bindgen]
pub fn decompress_g2(compressed: &[u8]) -> Result<Vec<u8>, JsError> {
    g2(compressed).map_err(JsError::new)
}

fn g1(compressed: &[u8]) -> Result<Vec<u8>, &'static str> {
    decompress(compressed, 32, 64, g1_point)
}

fn g2(compressed: &[u8]) -> Result<Vec<u8>, &'static str> {
    decompress(compressed, 64, 128, g2_point)
}

fn decompress(
    compressed: &[u8],
    compressed_size: usize,
    raw_size: usize,
    point: fn(&[u8], &mut [u8]) -> Result<(), &'static str>,
) -> Result<Vec<u8>, &'static str> {
    if !compressed.len().is_multiple_of(compressed_size) {
        return Err("compressed points of a partial length");
    }
    let mut raw = vec![0u8; compressed.len() / compressed_size * raw_size];
    compressed
        .par_chunks(compressed_size)
        .zip(raw.par_chunks_mut(raw_size))
        .try_for_each(|(compressed, raw)| point(compressed, raw))?;
    Ok(raw)
}

/// Whether `value` is larger than its negation: above (q - 1) / 2.
fn largest(value: &Fq) -> bool {
    value.into_bigint() > Fq::MODULUS_MINUS_ONE_DIV_TWO
}

/// As gnark orders E2: by the second coordinate, by the first when it is zero.
fn largest2(value: &Fq2) -> bool {
    if value.c1.is_zero() {
        largest(&value.c0)
    } else {
        largest(&value.c1)
    }
}

/// The canonical big-endian field element in `bytes`.
fn coordinate(bytes: &[u8]) -> Result<Fq, &'static str> {
    let value = Fq::from_be_bytes_mod_order(bytes);
    if value.into_bigint().to_bytes_be() != bytes {
        return Err("coordinate not below the field modulus");
    }
    Ok(value)
}

fn write(out: &mut [u8], value: &Fq) {
    out.copy_from_slice(&value.into_bigint().to_bytes_be());
}

/// The flags of a compressed point, or `None` for the point at infinity,
/// which must be zero otherwise.
fn flags(compressed: &[u8]) -> Result<Option<u8>, &'static str> {
    let mask = compressed[0] & MASK;
    if mask == INFINITY {
        if compressed[0] & !MASK != 0 || compressed[1..].iter().any(|&byte| byte != 0) {
            return Err("point at infinity with coordinates");
        }
        return Ok(None);
    }
    if mask != SMALLEST && mask != LARGEST {
        return Err("not a compressed point");
    }
    Ok(Some(mask))
}

fn g1_point(compressed: &[u8], raw: &mut [u8]) -> Result<(), &'static str> {
    let Some(mask) = flags(compressed)? else {
        raw.fill(0);
        return Ok(());
    };
    let mut x_bytes = [0u8; 32];
    x_bytes.copy_from_slice(compressed);
    x_bytes[0] &= !MASK;
    let x = coordinate(&x_bytes)?;
    let mut y = (x.square() * x + g1::Config::COEFF_B)
        .sqrt()
        .ok_or("x is not on the curve")?;
    if largest(&y) != (mask == LARGEST) {
        y = -y;
    }
    write(&mut raw[..32], &x);
    write(&mut raw[32..], &y);
    Ok(())
}

fn g2_point(compressed: &[u8], raw: &mut [u8]) -> Result<(), &'static str> {
    let Some(mask) = flags(compressed)? else {
        raw.fill(0);
        return Ok(());
    };
    let mut a1 = [0u8; 32];
    a1.copy_from_slice(&compressed[..32]);
    a1[0] &= !MASK;
    let x = Fq2::new(coordinate(&compressed[32..])?, coordinate(&a1)?);
    let mut y = (x.square() * x + g2::Config::COEFF_B)
        .sqrt()
        .ok_or("x is not on the twist")?;
    if largest2(&y) != (mask == LARGEST) {
        y = -y;
    }
    write(&mut raw[..32], &x.c1);
    write(&mut raw[32..64], &x.c0);
    write(&mut raw[64..96], &y.c1);
    write(&mut raw[96..], &y.c0);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    mod vectors {
        include!("decompress_vectors.rs");
    }

    fn hex(text: &str) -> Vec<u8> {
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn g1_points_decompress_as_gnark_encodes_them() {
        let compressed: Vec<u8> = vectors::G1.iter().flat_map(|(c, _)| hex(c)).collect();
        let raw: Vec<u8> = vectors::G1.iter().flat_map(|(_, r)| hex(r)).collect();
        assert_eq!(g1(&compressed).unwrap(), raw);
    }

    #[test]
    fn g2_points_decompress_as_gnark_encodes_them() {
        let compressed: Vec<u8> = vectors::G2.iter().flat_map(|(c, _)| hex(c)).collect();
        let raw: Vec<u8> = vectors::G2.iter().flat_map(|(_, r)| hex(r)).collect();
        assert_eq!(g2(&compressed).unwrap(), raw);
    }

    #[test]
    fn malformed_points_are_refused() {
        let mut off_curve = hex(vectors::G1[0].0);
        off_curve[31] ^= 1;
        assert!(g1(&off_curve).is_err());
        let mut uncompressed = hex(vectors::G1[0].0);
        uncompressed[0] &= !MASK;
        assert!(g1(&uncompressed).is_err());
        let mut infinity_with_x = hex(vectors::G1[8].0);
        infinity_with_x[5] = 1;
        assert!(g1(&infinity_with_x).is_err());
        assert!(g1(&[0; 31]).is_err());
        assert!(g2(&hex(vectors::G1[0].0)).is_err());
    }
}
