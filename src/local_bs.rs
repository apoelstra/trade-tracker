// Trade Tracker
// Written in 2021 by
//   Andrew Poelstra <tradetracker@wpsoftware.net>
//
// To the extent possible under law, the author(s) have dedicated all
// copyright and related and neighboring rights to this software to
// the public domain worldwide. This software is distributed without
// any warranty.
//
// You should have received a copy of the CC0 Public Domain Dedication
// along with this software.
// If not, see <http://creativecommons.org/publicdomain/zero/1.0/>.
//

//! "Local" Black-Scholes formulae
//!
//! Stuff that the `black_scholes_rust` crate doesn't do, and which I don't want to
//! PR upstream since I'm too lazy to write unit tests etc
//!

/// Computes the "dual delta" of an option
///
/// See <http://jtoll.com/post/duel-of-the-deltas/>
///
/// Note importantly that this is *only true in the absense of dividends*, mainly
/// because the `black_scholes` library assumes no dividends. This is directly
/// computing `N(d2)` but the correct formula is `exp(-qt)N(d2)` where `q` is the
/// dividend rate.
///
/// Note also that the above link appears to get this wrong, using `r` in place of
/// `q`. Caveat lector!
pub fn call_dual_delta(s: f64, k: f64, r: f64, sigma: f64, t: f64) -> f64 {
    let rho = black_scholes::call_rho(s, k, r, sigma, t);
    rho / k / (-r * t).exp() / t
}

/// Same as `call_dual_delta` but for puts
pub fn put_dual_delta(s: f64, k: f64, r: f64, sigma: f64, t: f64) -> f64 {
    call_dual_delta(s, k, r, sigma, t) - 1.0
}

#[cfg(test)]
mod tests {
    fn d1(s: f64, k: f64, discount: f64, sqrt_maturity_sigma: f64) -> f64 {
        (s / (k * discount)).ln() / sqrt_maturity_sigma + 0.5 * sqrt_maturity_sigma
    }

    // This turned out not to be useful, since we can compute d1 directly,
    // but I'm leaving it in.
    #[test]
    fn abuse_vanna_vomma_to_get_d1() {
        // Check that I can abuse vanna and vomma computations to extract a
        // version of d1
        let maturity = 0.1;
        let s = 1000.0; // stock price
        let k = 1200.0; // strike price
        let r = 0.04;
        let vol = 0.3;

        let vomma = black_scholes::call_vomma(s, k, r, vol, maturity);
        let vanna = black_scholes::call_vanna(s, k, r, vol, maturity);

        let est_d1 = -vomma / vanna / s / maturity.sqrt();
        let act_d1 = d1(s, k, (-r * maturity).exp(), maturity.sqrt() * vol);
        assert!((est_d1 - act_d1).abs() < 1.0e-10);
    }

    // Phi(d2) on the other hand we need erf and stuff, so computing it from
    // the BS library is more of a trick
    #[test]
    fn abuse_rho_to_get_cum_d2() {
        let maturity = 0.1;
        let s = 1000.0; // stock price
        let k = 1200.0; // strike price
        let r = 0.04;
        let vol = 0.3;

        let rho = black_scholes::call_rho(s, k, r, vol, maturity);
        let cum_d2 = rho / k / (-r * maturity).exp() / maturity;
        // lol nothing I can test against, but at least put a fixed vector here
        assert!((cum_d2 - 0.026983060057).abs() < 1.0e-10);
    }
}
