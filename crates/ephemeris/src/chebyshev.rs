//! Chebyshev polynomials, which is how an SPK segment stores a trajectory.
//!
//! A segment covers an interval and divides it into records; each record holds the coefficients of
//! a Chebyshev expansion of the position in each coordinate. Evaluating that expansion is the
//! whole numerical content of reading an ephemeris, so it lives here on its own, testable without
//! a file, a kernel or a body.
//!
//! # Why Clenshaw
//!
//! The direct form `Σ cₖ Tₖ(u)` needs `Tₖ` one at a time and loses precision as `k` grows; the
//! Clenshaw recurrence evaluates the same polynomial in one pass with no cancellation beyond the
//! polynomial's own. Both evaluations here use it, the derivative through its own coefficient set.

/// Evaluate `Σ cₖ Tₖ(u)`, the Chebyshev series with coefficients `coefficients` at `u`.
///
/// `u` is expected in `[-1, 1]`, which is what mapping a time inside a record onto the record
/// interval gives. Outside that range the series still evaluates — the polynomials are defined
/// everywhere — but it no longer describes the trajectory, so callers map first and check range
/// themselves.
#[must_use]
pub fn evaluate(coefficients: &[f64], u: f64) -> f64 {
    // Clenshaw: bₖ = 2u·bₖ₊₁ − bₖ₊₂ + cₖ, and the series is b₀ − u·b₁.
    let mut next = 0.0_f64;
    let mut current = 0.0_f64;
    for coefficient in coefficients.iter().rev() {
        let updated = 2.0 * u * current - next + coefficient;
        next = current;
        current = updated;
    }
    current - u * next
}

/// Evaluate the derivative `d/du Σ cₖ Tₖ(u)`.
///
/// The derivative of a Chebyshev series is another Chebyshev series, so this converts the
/// coefficients and then evaluates once; that is cheaper and steadier than differentiating the
/// recurrence. The recurrence used is `T'ₖ = 2k·Tₖ₋₁ + T'ₖ₋₂`, which is a standard identity for
/// these polynomials.
#[must_use]
pub fn evaluate_derivative(coefficients: &[f64], u: f64) -> f64 {
    let derivative = derivative_coefficients(coefficients);
    if derivative.is_empty() {
        return 0.0;
    }
    evaluate(&derivative, u)
}

/// The Chebyshev coefficients of the derivative series.
///
/// Uses the identity `Tₖ' = 2k·(Tₖ₋₁ + Tₖ₋₃ + …)`, except that the last term, when it is `T₀`,
/// carries `k` rather than `2k` — differentiating lowers the degree once, and the constant term
/// arrives at half weight. Writing the sum out is `O(m²)` for `m` coefficients; an ephemeris
/// record has fourteen, so the readable version is the right one.
fn derivative_coefficients(coefficients: &[f64]) -> Vec<f64> {
    let m = coefficients.len().saturating_sub(1);
    if m == 0 {
        return Vec::new();
    }
    let mut out = vec![0.0_f64; m];
    for (k, coefficient) in coefficients.iter().enumerate().skip(1) {
        let scale = 2.0 * (k as f64) * coefficient;
        // Walk down the output by two: `Tₖ' = 2k(Tₖ₋₁ + Tₖ₋₃ + …)`, and the last step onto `T₀`
        // carries half weight because differentiating lowers the degree once.
        let mut index = k - 1;
        while let Some(slot) = out.get_mut(index) {
            if index == 0 {
                *slot += scale * 0.5;
                break;
            }
            *slot += scale;
            if index < 2 {
                break;
            }
            index -= 2;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_constant_series_is_its_own_value() {
        assert!((evaluate(&[3.5], 0.37) - 3.5).abs() < 1e-15);
        assert_eq!(evaluate(&[], 0.5), 0.0);
        assert_eq!(evaluate_derivative(&[3.5], 0.37), 0.0);
    }

    #[test]
    fn a_first_degree_series_is_a_line_in_the_polynomial_basis() {
        // c₀ + c₁·T₁(u) = c₀ + c₁·u
        let c = [2.0, -0.5];
        assert!((evaluate(&c, 0.4) - (2.0 - 0.2)).abs() < 1e-15);
        assert!((evaluate_derivative(&c, 0.4) - -0.5).abs() < 1e-15);
    }

    #[test]
    fn a_second_degree_series_matches_the_closed_form() {
        // c₀ + c₁u + c₂(2u² − 1); its derivative is c₁ + 4c₂u.
        let c = [1.0, 2.0, 3.0];
        let (c0, c1, c2) = (1.0_f64, 2.0, 3.0);
        for u in [-1.0, -0.35, 0.0, 0.6, 1.0] {
            let expected = c0 + c1 * u + c2 * (2.0 * u * u - 1.0);
            assert!(
                (evaluate(&c, u) - expected).abs() < 1e-14,
                "at u={u}: {} against {expected}",
                evaluate(&c, u)
            );
            let expected_derivative = c1 + 4.0 * c2 * u;
            assert!(
                (evaluate_derivative(&c, u) - expected_derivative).abs() < 1e-13,
                "derivative at u={u}"
            );
        }
    }

    #[test]
    fn the_derivative_agrees_with_a_finite_difference() {
        // A numerical check on a longer series: it catches a coefficient index that is off by one
        // in the derivative recurrence, which the closed forms above would pass for low degree.
        let coefficients = [0.4, -1.2, 0.7, 0.05, -0.02, 0.004, -0.0007];
        let step = 1e-6;
        for u in [-0.8, -0.2, 0.1, 0.45, 0.9] {
            let numeric = (evaluate(&coefficients, u + step) - evaluate(&coefficients, u - step))
                / (2.0 * step);
            let analytic = evaluate_derivative(&coefficients, u);
            assert!(
                (numeric - analytic).abs() < 1e-8,
                "at u={u}: numeric {numeric} against analytic {analytic}"
            );
        }
    }

    #[test]
    fn clenshaw_beats_evaluating_the_polynomials_one_at_a_time() {
        // Tₙ(1) = 1 and Tₙ(−1) = (−1)ⁿ exactly, so a fifteen-term series with unit coefficients
        // sums to 15 and to 1 or 0 there. This is the property the recurrence exists to keep.
        let coefficients = [1.0_f64; 15];
        assert!((evaluate(&coefficients, 1.0) - 15.0).abs() < 1e-13);
        assert!((evaluate(&coefficients, -1.0) - 1.0).abs() < 1e-13);
    }
}
