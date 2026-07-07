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

const BETA_ITMAX: usize = 300;
const BETA_EPS: f64 = 3.0e-14;
const BETA_TINY: f64 = 1.0e-300;

/// Gauss error function `erf(x)` via the regularised lower incomplete gamma
/// identity `erf(x) = sign(x)·P(1/2, x²)`. Exact at `x = 0`; fails closed only
/// if the underlying incomplete-gamma evaluation does (it never does for a
/// finite `x`, since `x² ≥ 0` is always in-domain).
pub(crate) fn erf(x: f64) -> Result<f64> {
    if !x.is_finite() {
        return Err(domain(format!("erf requires a finite argument, got {x}")));
    }
    if x == 0.0 {
        return Ok(0.0);
    }
    let p = gammp(0.5, x * x)?;
    Ok(if x < 0.0 { -p } else { p })
}

/// Two-sided standard-normal tail probability `P(|Z| ≥ |z|) = erfc(|z|/√2)`.
pub(crate) fn normal_two_sided_p(z: f64) -> Result<f64> {
    if !z.is_finite() {
        return Err(domain(format!(
            "normal tail requires a finite z-statistic, got {z}"
        )));
    }
    let e = erf(z.abs() / std::f64::consts::SQRT_2)?;
    Ok((1.0 - e).clamp(0.0, 1.0))
}

/// Regularised incomplete beta `I_x(a, b)` (Numerical Recipes: closed-form
/// tails plus the Lentz continued fraction `betacf`, symmetry-reflected at the
/// convergence boundary `x = (a+1)/(a+b+2)`). Fails closed outside `x ∈ [0,1]`
/// or for non-positive shape parameters.
pub(crate) fn betai(a: f64, b: f64, x: f64) -> Result<f64> {
    if !a.is_finite() || a <= 0.0 || !b.is_finite() || b <= 0.0 {
        return Err(domain(format!(
            "incomplete beta requires a > 0 and b > 0, got a={a}, b={b}"
        )));
    }
    if !x.is_finite() || !(0.0..=1.0).contains(&x) {
        return Err(domain(format!(
            "incomplete beta requires x ∈ [0, 1], got x={x}"
        )));
    }
    if x == 0.0 {
        return Ok(0.0);
    }
    if x == 1.0 {
        return Ok(1.0);
    }
    let ln_front = ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b) + a * x.ln() + b * (1.0 - x).ln();
    let front = ln_front.exp();
    if x < (a + 1.0) / (a + b + 2.0) {
        Ok((front * betacf(a, b, x)? / a).clamp(0.0, 1.0))
    } else {
        Ok((1.0 - front * betacf(b, a, 1.0 - x)? / b).clamp(0.0, 1.0))
    }
}

fn betacf(a: f64, b: f64, x: f64) -> Result<f64> {
    let qab = a + b;
    let qap = a + 1.0;
    let qam = a - 1.0;
    let mut c = 1.0;
    let mut d = 1.0 - qab * x / qap;
    if d.abs() < BETA_TINY {
        d = BETA_TINY;
    }
    d = 1.0 / d;
    let mut h = d;
    for m in 1..=BETA_ITMAX {
        let m = m as f64;
        let m2 = 2.0 * m;
        // Even step.
        let aa = m * (b - m) * x / ((qam + m2) * (a + m2));
        d = 1.0 + aa * d;
        if d.abs() < BETA_TINY {
            d = BETA_TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < BETA_TINY {
            c = BETA_TINY;
        }
        d = 1.0 / d;
        h *= d * c;
        // Odd step.
        let aa = -(a + m) * (qab + m) * x / ((a + m2) * (qap + m2));
        d = 1.0 + aa * d;
        if d.abs() < BETA_TINY {
            d = BETA_TINY;
        }
        c = 1.0 + aa / c;
        if c.abs() < BETA_TINY {
            c = BETA_TINY;
        }
        d = 1.0 / d;
        let del = d * c;
        h *= del;
        if (del - 1.0).abs() < BETA_EPS {
            return Ok(h);
        }
    }
    Err(domain(
        "incomplete beta continued fraction did not converge",
    ))
}

/// Two-sided Student-t tail probability `P(|T_df| ≥ |t|)` via the incomplete
/// beta: `I_{df/(df+t²)}(df/2, 1/2)`. `t = 0 → 1.0` (no evidence).
pub(crate) fn student_t_two_sided_p(t: f64, df: f64) -> Result<f64> {
    if !t.is_finite() {
        return Err(domain(format!(
            "student-t tail requires a finite t-statistic, got {t}"
        )));
    }
    if !df.is_finite() || df <= 0.0 {
        return Err(domain(format!(
            "student-t tail requires df > 0, got df={df}"
        )));
    }
    let x = df / (df + t * t);
    betai(df / 2.0, 0.5, x)
}

/// Upper-tail probability `P(F ≥ f)` for the F-distribution with `(df1, df2)`
/// degrees of freedom, via the incomplete beta. Using the CDF identity
/// `P(F ≤ f) = I_{df1·f/(df1·f+df2)}(df1/2, df2/2)` and the beta symmetry
/// `I_x(a,b) = 1 − I_{1−x}(b,a)`, the tail is `I_{df2/(df2+df1·f)}(df2/2, df1/2)`.
/// `f = 0 → 1.0` (no evidence). Fails closed on non-positive degrees of freedom
/// or a non-finite / negative statistic.
pub(crate) fn f_upper_tail_p(f: f64, df1: f64, df2: f64) -> Result<f64> {
    if !f.is_finite() || f < 0.0 {
        return Err(domain(format!("F tail requires a finite F ≥ 0, got f={f}")));
    }
    if !df1.is_finite() || df1 <= 0.0 || !df2.is_finite() || df2 <= 0.0 {
        return Err(domain(format!(
            "F tail requires df1 > 0 and df2 > 0, got df1={df1}, df2={df2}"
        )));
    }
    if f == 0.0 {
        return Ok(1.0);
    }
    let x = df2 / (df2 + df1 * f);
    betai(df2 / 2.0, df1 / 2.0, x)
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
    fn erf_and_normal_cdf_match_known_values() {
        // erf(0)=0, erf(1)=0.842700792949715, erf(-1) is its negative.
        approx(erf(0.0).unwrap(), 0.0, 1e-15, "erf(0)");
        approx(erf(1.0).unwrap(), 0.842_700_792_949_715, 1e-12, "erf(1)");
        approx(erf(-1.0).unwrap(), -0.842_700_792_949_715, 1e-12, "erf(-1)");
        // Two-sided normal tail: p(0)=1, and p at the 5% critical z ≈ 0.05.
        approx(normal_two_sided_p(0.0).unwrap(), 1.0, 1e-15, "normal p(0)");
        approx(
            normal_two_sided_p(1.959_963_985).unwrap(),
            0.05,
            1e-8,
            "normal two-sided p(1.96)",
        );
    }

    #[test]
    fn incomplete_beta_matches_known_values() {
        // I_x(1,1) = x (uniform CDF).
        approx(betai(1.0, 1.0, 0.3).unwrap(), 0.3, 1e-12, "I_0.3(1,1)");
        // Symmetry: I_x(a,b) = 1 - I_{1-x}(b,a).
        approx(
            betai(2.0, 3.0, 0.4).unwrap(),
            1.0 - betai(3.0, 2.0, 0.6).unwrap(),
            1e-12,
            "beta symmetry",
        );
        // I_x(0.5,0.5) = (2/π)·asin(√x): at x=0.5 → 0.5.
        approx(betai(0.5, 0.5, 0.5).unwrap(), 0.5, 1e-12, "I_0.5(0.5,0.5)");
    }

    #[test]
    fn student_t_two_sided_matches_known_values() {
        // t = 0 → no evidence, p = 1.
        approx(student_t_two_sided_p(0.0, 10.0).unwrap(), 1.0, 1e-12, "t=0");
        // df=1 is Cauchy: P(|T| ≥ 1) = 0.5 (quartiles at ±1).
        approx(
            student_t_two_sided_p(1.0, 1.0).unwrap(),
            0.5,
            1e-9,
            "cauchy quartile",
        );
        // Classic table value: two-sided p at t=2.228, df=10 ≈ 0.05.
        approx(
            student_t_two_sided_p(2.228_138_852, 10.0).unwrap(),
            0.05,
            1e-6,
            "t(10) 5% critical",
        );
    }

    #[test]
    fn f_upper_tail_matches_known_values() {
        // f = 0 → no evidence, p = 1.
        approx(f_upper_tail_p(0.0, 3.0, 10.0).unwrap(), 1.0, 1e-12, "F f=0");
        // Classic table value: F(3,10) 5% critical point ≈ 3.708265 → tail 0.05.
        approx(
            f_upper_tail_p(3.708_265, 3.0, 10.0).unwrap(),
            0.05,
            1e-5,
            "F(3,10) 5% critical",
        );
        // F(1, df) equals a two-sided t: P(F_{1,df} ≥ t²) = P(|T_df| ≥ |t|).
        approx(
            f_upper_tail_p(4.0, 1.0, 20.0).unwrap(),
            student_t_two_sided_p(2.0, 20.0).unwrap(),
            1e-9,
            "F(1,20) vs t(20)²",
        );
    }

    #[test]
    fn special_fn_fail_closed_on_bad_domain() {
        assert_eq!(
            betai(0.0, 1.0, 0.5).unwrap_err().code,
            "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
        );
        assert_eq!(
            betai(1.0, 1.0, 1.5).unwrap_err().code,
            "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
        );
        assert_eq!(
            student_t_two_sided_p(1.0, 0.0).unwrap_err().code,
            "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
        );
        assert_eq!(
            erf(f64::NAN).unwrap_err().code,
            "CALYX_ASSAY_INSUFFICIENT_SAMPLES"
        );
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
