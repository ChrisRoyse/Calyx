//! Deterministic special functions shared by the statistical modules: the
//! regularised incomplete gamma integrals and `ln Γ`. Numerical Recipes
//! lineage (series + continued fraction); Lanczos `g = 7` for `ln Γ`. All
//! fail-closed on invalid domains — never a silent NaN.

use calyx_core::{CalyxError, Result};

const GAMMA_ITMAX: usize = 300;
const GAMMA_EPS: f64 = 3.0e-14;
const GAMMA_TINY: f64 = 1.0e-300;

/// Regularised upper incomplete gamma `Q(a, x) = Γ(a, x) / Γ(a)`.
pub(crate) fn gammq(a: f64, x: f64) -> Result<f64> {
    Ok(1.0 - gammp(a, x)?)
}

/// Regularised lower incomplete gamma `P(a, x) = γ(a, x) / Γ(a)`
/// (series for `x < a+1`, continued fraction otherwise).
pub(crate) fn gammp(a: f64, x: f64) -> Result<f64> {
    if !a.is_finite() || a <= 0.0 || !x.is_finite() || x < 0.0 {
        return Err(domain(format!(
            "incomplete gamma requires a > 0 and x ≥ 0, got a={a}, x={x}"
        )));
    }
    if x == 0.0 {
        return Ok(0.0);
    }
    if x < a + 1.0 {
        // Series representation of P(a, x).
        let mut ap = a;
        let mut sum = 1.0 / a;
        let mut del = sum;
        for _ in 0..GAMMA_ITMAX {
            ap += 1.0;
            del *= x / ap;
            sum += del;
            if del.abs() < sum.abs() * GAMMA_EPS {
                return Ok((sum * (-x + a * x.ln() - ln_gamma(a)).exp()).clamp(0.0, 1.0));
            }
        }
        Err(domain("incomplete gamma series did not converge"))
    } else {
        // Continued-fraction (Lentz) representation of Q(a, x) = 1 - P(a, x).
        let mut b = x + 1.0 - a;
        let mut c = 1.0 / GAMMA_TINY;
        let mut d = 1.0 / b;
        let mut h = d;
        for i in 1..GAMMA_ITMAX {
            let an = -(i as f64) * (i as f64 - a);
            b += 2.0;
            d = an * d + b;
            if d.abs() < GAMMA_TINY {
                d = GAMMA_TINY;
            }
            c = b + an / c;
            if c.abs() < GAMMA_TINY {
                c = GAMMA_TINY;
            }
            d = 1.0 / d;
            let del = d * c;
            h *= del;
            if (del - 1.0).abs() < GAMMA_EPS {
                let q = (-x + a * x.ln() - ln_gamma(a)).exp() * h;
                return Ok((1.0 - q).clamp(0.0, 1.0));
            }
        }
        Err(domain(
            "incomplete gamma continued fraction did not converge",
        ))
    }
}

/// Natural log of the gamma function via the Lanczos approximation (g = 7),
/// with the reflection formula for `z < 0.5`. Accurate to ~1e-13.
pub(crate) fn ln_gamma(z: f64) -> f64 {
    const G: f64 = 7.0;
    const COEFF: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if z < 0.5 {
        let pi = std::f64::consts::PI;
        return (pi / (pi * z).sin()).ln() - ln_gamma(1.0 - z);
    }
    let z = z - 1.0;
    let mut a = COEFF[0];
    let t = z + G + 0.5;
    for (i, &coeff) in COEFF.iter().enumerate().skip(1) {
        a += coeff / (z + i as f64);
    }
    0.5 * (2.0 * std::f64::consts::PI).ln() + (z + 0.5) * t.ln() - t + a.ln()
}

fn domain(message: impl Into<String>) -> CalyxError {
    CalyxError::assay_insufficient_samples(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(actual: f64, expected: f64, tol: f64, what: &str) {
        assert!(
            (actual - expected).abs() <= tol,
            "{what}: got {actual}, expected {expected} (tol {tol})"
        );
    }

    #[test]
    fn ln_gamma_matches_known_values() {
        // Γ(5) = 24, Γ(1/2) = √π, Γ(1) = Γ(2) = 1.
        approx(ln_gamma(5.0), 24.0_f64.ln(), 1e-10, "lnΓ(5)");
        approx(
            ln_gamma(0.5),
            std::f64::consts::PI.sqrt().ln(),
            1e-10,
            "lnΓ(1/2)",
        );
        approx(ln_gamma(1.0), 0.0, 1e-12, "lnΓ(1)");
        approx(ln_gamma(2.0), 0.0, 1e-12, "lnΓ(2)");
    }

    #[test]
    fn incomplete_gamma_matches_exponential_and_erlang() {
        // a = 1 is the exponential: Q(1, x) = e^{-x}, P(1, x) = 1 - e^{-x}.
        approx(gammq(1.0, 2.0).unwrap(), (-2.0_f64).exp(), 1e-12, "Q(1,2)");
        approx(
            gammp(1.0, 1.0).unwrap(),
            1.0 - (-1.0_f64).exp(),
            1e-12,
            "P(1,1)",
        );
        // a = 2 Erlang: P(2, 2) = 1 - 3 e^{-2} (continued-fraction branch).
        approx(
            gammp(2.0, 2.0).unwrap(),
            1.0 - 3.0 * (-2.0_f64).exp(),
            1e-12,
            "P(2,2)",
        );
        // Complementarity P + Q = 1 across both branches.
        for &(a, x) in &[(0.7, 0.2), (3.0, 5.0), (2.5, 2.5)] {
            approx(
                gammp(a, x).unwrap() + gammq(a, x).unwrap(),
                1.0,
                1e-12,
                "P+Q",
            );
        }
    }

    #[test]
    fn incomplete_gamma_fails_closed_on_bad_domain() {
        assert_eq!(
            gammp(0.0, 1.0).unwrap_err().code,
            "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
        );
        assert_eq!(
            gammp(1.0, -1.0).unwrap_err().code,
            "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
        );
    }
}
