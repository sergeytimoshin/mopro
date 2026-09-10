// Copyright 2020-2025 Consensys Software Inc.
// Licensed under Apache-2.0; see LICENSE-APACHE.
// Modified for Mopro: prepare a validated execution plan for hint-free gnark
// v0.14.0 R1CS systems, and solve it using arkworks field arithmetic.
// The arithmetic follows constraint/bn254/solver.go's solveR1C semantics.

use ark_bn254::Fr;
use ark_ff::{AdditiveGroup, Field, PrimeField};
use std::collections::HashMap;
use std::ops::Range;

#[derive(Clone, Copy)]
struct Term {
    coefficient: usize,
    wire: usize,
}

struct Assignment {
    side: usize,
    wire: usize,
    inverse: Fr,
    coefficient: usize,
}
enum Expression {
    Zero,
    Wire(usize),
    Constant(usize),
    Term(usize),
    Sum(Range<usize>),
}
struct Step {
    constraint: usize,
    sums: Range<usize>,
    expressions: [Expression; 3],
    assignment: Option<Assignment>,
}
struct Sum {
    previous: usize,
    term: Term,
}
const NO_SUM: usize = usize::MAX;
pub(crate) struct Plan {
    n_wires: usize,
    n_inputs: usize,
    pub(crate) n_public: usize,
    pub(crate) a_indices: Vec<usize>,
    pub(crate) b_indices: Vec<usize>,
    coefficients: Vec<Fr>,
    terms: Vec<Term>,
    sums: Vec<Sum>,
    steps: Vec<Step>,
}
pub(crate) struct Solution {
    pub(crate) w: Vec<Fr>,
    pub(crate) a: Vec<Fr>,
    pub(crate) b: Vec<Fr>,
    pub(crate) c: Vec<Fr>,
}

impl Plan {
    // Header: version, wires, inputs (including ONE), public (including ONE),
    // constraints, A index count, B index count; followed by both index lists
    // and constraint records [ID, len(L), len(R), len(O), (coefficient, wire)...].
    // Records arrive in gnark's level order, while ID selects the FFT row.
    pub(crate) fn new(program: &[u8], coefficients: &[u8]) -> Result<Self, &'static str> {
        if !program.len().is_multiple_of(4) || !coefficients.len().is_multiple_of(32) {
            return Err("invalid solver encoding");
        }
        let mut words = program
            .as_chunks::<4>()
            .0
            .iter()
            .map(|v| u32::from_le_bytes(*v) as usize);
        let mut read = || words.next().ok_or("truncated solver program");
        if read()? != 1 {
            return Err("unsupported solver program version");
        }
        let n_wires = read()?;
        let n_inputs = read()?;
        let n_public = read()?;
        let n_constraints = read()?;
        let n_a = read()?;
        let n_b = read()?;
        // Bound all allocations by the supplied encoding. Every internal wire
        // needs its own constraint, and every record occupies at least 4 words.
        if n_public == 0
            || n_public > n_inputs
            || n_inputs > n_wires
            || n_wires - n_inputs > n_constraints
            || n_constraints > program.len() / 16
            || n_inputs > program.len() / 4
        {
            return Err("invalid solver dimensions");
        }
        let mut indices = |n: usize| -> Result<Vec<usize>, &'static str> {
            if n > n_wires || n > program.len() / 4 {
                return Err("invalid solver index count");
            }
            let mut out = Vec::with_capacity(n);
            for _ in 0..n {
                let index = read()?;
                if index >= n_wires || out.last().is_some_and(|previous| *previous >= index) {
                    return Err("invalid solver wire index");
                }
                out.push(index);
            }
            Ok(out)
        };
        let a_indices = indices(n_a)?;
        let b_indices = indices(n_b)?;
        let mut coefficients = super::frs(coefficients);
        if coefficients.len() < 5
            || coefficients[..5] != [Fr::ZERO, Fr::ONE, Fr::from(2u64), -Fr::ONE, -Fr::from(2u64)]
            || coefficients.iter().any(|c| c.0 >= Fr::MODULUS)
        {
            return Err("invalid solver coefficients");
        }
        let n_coefficients = coefficients.len();
        let mut solved = vec![false; n_wires];
        solved[..n_inputs].fill(true);
        let mut seen = vec![false; n_constraints];
        let mut terms = Vec::new();
        let mut sums = Vec::new();
        let mut prefixes = HashMap::new();
        let mut steps = Vec::with_capacity(n_constraints);
        for _ in 0..n_constraints {
            let constraint = read()?;
            if constraint >= n_constraints || seen[constraint] {
                return Err("duplicate or out-of-range constraint");
            }
            seen[constraint] = true;
            let lengths = [read()?, read()?, read()?];
            let sum_start = sums.len();
            let mut expressions = [Expression::Zero, Expression::Zero, Expression::Zero];
            let mut assignment = None;
            for (side, len) in lengths.into_iter().enumerate() {
                let start = terms.len();
                let mut constant = Fr::ZERO;
                for _ in 0..len {
                    let coefficient = read()?;
                    let wire = read()?;
                    if coefficient >= n_coefficients || wire >= n_wires {
                        return Err("invalid solver term");
                    }
                    if solved[wire] {
                        if wire == 0 {
                            constant += coefficients[coefficient];
                        } else if coefficient != 0 {
                            terms.push(Term { coefficient, wire });
                        }
                    } else {
                        if assignment.is_some() {
                            return Err("constraint has multiple unsolved terms");
                        }
                        let inverse = match coefficient {
                            1 => Fr::ONE,
                            3 => -Fr::ONE,
                            _ => coefficients[coefficient]
                                .inverse()
                                .ok_or("unsolved term has zero coefficient")?,
                        };
                        assignment = Some(Assignment {
                            side,
                            wire,
                            inverse,
                            coefficient,
                        });
                    }
                }
                if constant != Fr::ZERO {
                    terms.push(Term {
                        coefficient: coefficients.len(),
                        wire: 0,
                    });
                    coefficients.push(constant);
                }
                expressions[side] = match &terms[start..] {
                    [] => Expression::Zero,
                    [term] if term.wire == 0 => Expression::Constant(term.coefficient),
                    [term] if term.coefficient == 1 => Expression::Wire(term.wire),
                    [_] => Expression::Term(start),
                    _ if terms.len() - start <= 2 => Expression::Sum(start..terms.len()),
                    _ => {
                        // Immutable wires let later expressions reuse sums from
                        // earlier constraints. Intern prefixes so long common
                        // linear expressions are evaluated only once per proof.
                        // Every new sum is scheduled at its first use, after
                        // all of its circuit wires have been solved.
                        let mut previous = NO_SUM;
                        for term in &terms[start..] {
                            previous = *prefixes
                                .entry((previous, term.coefficient, term.wire))
                                .or_insert_with(|| {
                                    let id = n_wires + sums.len();
                                    sums.push(Sum {
                                        previous,
                                        term: *term,
                                    });
                                    id
                                });
                        }
                        // The DAG owns its terms; repeated source expressions
                        // need not occupy the prepared key's memory.
                        terms.truncate(start);
                        Expression::Wire(previous)
                    }
                };
            }
            if let Some(assignment) = &assignment {
                solved[assignment.wire] = true;
            }
            steps.push(Step {
                constraint,
                sums: sum_start..sums.len(),
                expressions,
                assignment,
            });
        }
        if words.next().is_some() || solved.contains(&false) {
            return Err("incomplete solver program");
        }
        Ok(Self {
            n_wires,
            n_inputs,
            n_public,
            a_indices,
            b_indices,
            coefficients,
            terms,
            sums,
            steps,
        })
    }

    pub(crate) fn matches(&self, a: usize, b: usize, k: usize, domain: usize) -> bool {
        self.a_indices.len() == a
            && self.b_indices.len() == b
            && self.n_wires - self.n_public == k
            && self.steps.len() <= domain
    }

    pub(crate) fn solve(&self, witness: &[Fr]) -> Result<Solution, String> {
        if witness.len() != self.n_inputs - 1 {
            return Err("invalid witness size".into());
        }
        let mut w = vec![Fr::ZERO; self.n_wires + self.sums.len()];
        w[0] = Fr::ONE;
        w[1..self.n_inputs].copy_from_slice(witness);
        let mut a = vec![Fr::ZERO; self.steps.len()];
        let mut b = a.clone();
        let mut c = a.clone();
        for step in &self.steps {
            for index in step.sums.clone() {
                let sum = &self.sums[index];
                let term = self.evaluate_term(&sum.term, &w);
                w[self.n_wires + index] = if sum.previous == NO_SUM {
                    term
                } else {
                    w[sum.previous] + term
                };
            }
            let mut values = [Fr::ZERO; 3];
            for (value, expression) in values.iter_mut().zip(&step.expressions) {
                *value = match expression {
                    Expression::Zero => Fr::ZERO,
                    Expression::Wire(wire) => w[*wire],
                    Expression::Constant(coefficient) => self.coefficients[*coefficient],
                    Expression::Term(term) => self.evaluate_term(&self.terms[*term], &w),
                    Expression::Sum(range) => {
                        let mut sum = Fr::ZERO;
                        for term in &self.terms[range.clone()] {
                            sum += self.evaluate_term(term, &w);
                        }
                        sum
                    }
                };
            }
            let fail = || format!("constraint #{} is not satisfied", step.constraint);
            if let Some(assignment) = &step.assignment {
                let term = match assignment.side {
                    0 | 1 => {
                        let other = values[1 - assignment.side];
                        if other == Fr::ZERO {
                            if values[2] != Fr::ZERO {
                                return Err(fail());
                            }
                            Fr::ZERO
                        } else {
                            let quotient = if other == Fr::ONE {
                                values[2]
                            } else {
                                values[2] * other.inverse().unwrap()
                            };
                            let term = quotient - values[assignment.side];
                            values[assignment.side] = quotient;
                            term
                        }
                    }
                    _ => {
                        let product = values[0] * values[1];
                        let term = product - values[2];
                        values[2] = product;
                        term
                    }
                };
                w[assignment.wire] = match assignment.coefficient {
                    1 => term,
                    3 => -term,
                    _ => term * assignment.inverse,
                };
            } else if values[0] * values[1] != values[2] {
                return Err(fail());
            }
            a[step.constraint] = values[0];
            b[step.constraint] = values[1];
            c[step.constraint] = values[2];
        }
        w.truncate(self.n_wires);
        Ok(Solution { w, a, b, c })
    }
    #[inline]
    fn evaluate_term(&self, term: &Term, w: &[Fr]) -> Fr {
        if term.wire == 0 {
            return self.coefficients[term.coefficient];
        }
        match term.coefficient {
            1 => w[term.wire],
            2 => w[term.wire].double(),
            3 => -w[term.wire],
            _ => self.coefficients[term.coefficient] * w[term.wire],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_words(words: &[u32]) -> Vec<u8> {
        words.iter().flat_map(|v| v.to_le_bytes()).collect()
    }
    fn encode_fields(values: &[Fr]) -> Vec<u8> {
        values
            .iter()
            .flat_map(|v| v.0 .0.iter().flat_map(|w| w.to_le_bytes()))
            .collect()
    }
    fn small() -> (Vec<u32>, Vec<u8>) {
        // ONE, public Y, secret X, internal X²; X² = Y is checked before
        // row zero in storage, but after it in dependency order.
        (
            vec![
                1, 4, 3, 2, 2, 3, 2, 0, 2, 3, 2, 3, 1, 1, 1, 1, 1, 2, 1, 2, 1, 3, 0, 1, 1, 1, 1, 3,
                1, 0, 1, 1,
            ],
            encode_fields(&[Fr::ZERO, Fr::ONE, Fr::from(2u64), -Fr::ONE, -Fr::from(2u64)]),
        )
    }

    #[test]
    fn solves_and_rejects_invalid_witnesses() {
        let (words, coefficients) = small();
        let plan = Plan::new(&encode_words(&words), &coefficients).unwrap();
        assert!(plan.matches(3, 2, 2, 2));
        assert!(!plan.matches(3, 2, 1, 2));
        for x in [Fr::ZERO, Fr::ONE, -Fr::ONE, Fr::from(123456u64)] {
            let solution = plan.solve(&[x * x, x]).unwrap();
            assert_eq!(solution.w, [Fr::ONE, x * x, x, x * x]);
            assert_eq!(solution.a, [x * x, x]);
            assert_eq!(solution.b, [Fr::ONE, x]);
            assert_eq!(solution.c, [x * x, x * x]);
            assert_eq!(
                plan.solve(&[x * x + Fr::ONE, x]).err().unwrap(),
                "constraint #0 is not satisfied"
            );
            // Failure does not contaminate the next witness.
            assert!(plan.solve(&[x * x, x]).is_ok());
        }
        assert!(plan.solve(&[]).is_err());
        assert!(plan.solve(&[Fr::ZERO; 3]).is_err());
    }

    #[test]
    fn unknown_input_with_zero_divisor() {
        let mut coefficients = small().1;
        coefficients.extend(encode_fields(&[Fr::from(7u64)]));
        for side in 0..2 {
            let mut words = vec![1, 4, 3, 1, 1, 4, 4, 0, 1, 2, 3, 0, 1, 2, 3, 0];
            let mut expressions = [vec![1, 0, 5, 3], vec![1, 2], vec![1, 1]];
            if side == 1 {
                expressions.swap(0, 1);
            }
            words.extend(
                expressions
                    .iter()
                    .map(|expression| (expression.len() / 2) as u32),
            );
            for expression in expressions {
                words.extend(expression);
            }
            let plan = Plan::new(&encode_words(&words), &coefficients).unwrap();
            let result = plan.solve(&[Fr::ZERO, Fr::ZERO]).unwrap();
            assert_eq!(result.w[3], Fr::ZERO);
            assert!(plan.solve(&[Fr::ONE, Fr::ZERO]).is_err());
            let result = plan.solve(&[Fr::from(24u64), Fr::from(3u64)]).unwrap();
            assert_eq!(result.w[3], Fr::ONE); // (1 + 7*1) * 3 = 24
        }
    }

    #[test]
    fn repeated_linear_expressions_use_fresh_witness_values() {
        let mut coefficients = small().1;
        coefficients.extend(encode_fields(&[
            Fr::from(3u64),
            Fr::from(5u64),
            Fr::from(7u64),
        ]));
        let mut program = vec![
            1, 7, 4, 1, 3, 7, 7, 0, 1, 2, 3, 4, 5, 6, 0, 1, 2, 3, 4, 5, 6,
        ];
        let base = vec![2, 1, 3, 2, 5, 3]; // 2X - Y + 3Z
        let mut first = base.clone();
        first.extend([6, 0]);
        let mut second = base;
        second.extend([1, 4, 7, 0]);
        for (row, expressions) in [
            [first.clone(), first, vec![1, 4]],
            [second.clone(), vec![1, 0], vec![1, 5]],
            [second, vec![1, 5], vec![1, 6]],
        ]
        .into_iter()
        .enumerate()
        {
            program.push(row as u32);
            program.extend(
                expressions
                    .iter()
                    .map(|expression| (expression.len() / 2) as u32),
            );
            for expression in expressions {
                program.extend(expression);
            }
        }
        let plan = Plan::new(&encode_words(&program), &coefficients).unwrap();
        for sample in 0..50u64 {
            let x = Fr::from(sample);
            let y = -x;
            let z = x * x;
            let base = x.double() - y + Fr::from(3u64) * z;
            let u = (base + Fr::from(5u64)).square();
            let v = base + u + Fr::from(7u64);
            let solution = plan.solve(&[x, y, z]).unwrap();
            assert_eq!(solution.w, [Fr::ONE, x, y, z, u, v, v * v]);
            assert_eq!(solution.a, [base + Fr::from(5u64), v, v]);
            assert_eq!(solution.b, [base + Fr::from(5u64), Fr::ONE, v]);
            assert_eq!(solution.c, [u, v, v * v]);
        }
    }

    #[test]
    fn validates_program_before_solving() {
        let (words, coefficients) = small();
        let bytes = encode_words(&words);
        for len in 0..bytes.len() {
            assert!(
                Plan::new(&bytes[..len], &coefficients).is_err(),
                "length {len}"
            );
        }
        let mut changed = words.clone();
        changed.push(0);
        assert!(Plan::new(&encode_words(&changed), &coefficients).is_err());
        for (index, value) in [
            (0, 2),
            (1, u32::MAX),
            (2, 0),
            (3, 4),
            (4, u32::MAX),
            (5, u32::MAX),
            (7, 4),
            (8, 0),
            (12, 0),
            (20, 0),
            (21, 4),
            (22, 1),
            (19, 4),
            (16, 5),
            (17, 3),
            (18, 5),
        ] {
            let mut changed = words.clone();
            changed[index] = value;
            assert!(
                Plan::new(&encode_words(&changed), &coefficients).is_err(),
                "word {index} = {value}"
            );
        }
        let mut changed = coefficients.clone();
        changed[0] = 1;
        assert!(Plan::new(&bytes, &changed).is_err());
        assert!(Plan::new(&bytes, &coefficients[..coefficients.len() - 1]).is_err());
        // Noncanonical table entries cannot enter field arithmetic.
        let mut changed = coefficients.clone();
        changed.extend(Fr::MODULUS.0.iter().flat_map(|w| w.to_le_bytes()));
        assert!(Plan::new(&bytes, &changed).is_err());
    }

    #[test]
    fn gnark_solver_fixtures() {
        let Some(dir) = std::env::var_os("MOPRO_GNARK_SOLVER_FIXTURES") else {
            eprintln!(
                "set MOPRO_GNARK_SOLVER_FIXTURES to compare with native gnark (enabled in CI)"
            );
            return;
        };
        let mut count = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|ext| ext != "case") {
                continue;
            }
            let encoded = std::fs::read(&path).unwrap();
            let mut data = &encoded[..];
            let mut read = || {
                let n = u32::from_le_bytes(data[..4].try_into().unwrap()) as usize;
                let value = &data[4..4 + n];
                data = &data[4 + n..];
                value
            };
            let program = read();
            let coefficients = read();
            let witness = super::super::frs(read());
            let plan = Plan::new(program, coefficients).unwrap();
            let solution = plan.solve(&witness).unwrap();
            for actual in [&solution.w, &solution.a, &solution.b, &solution.c] {
                assert_eq!(encode_fields(actual), read(), "{}", path.display());
            }
            let invalid = super::super::frs(read());
            assert!(plan.solve(&invalid).is_err(), "{}", path.display());
            assert!(data.is_empty());
            count += 1;
        }
        assert_eq!(
            count, 16,
            "generate fixtures with go test -run TestKernelSolverFixtures first"
        );
    }
}
