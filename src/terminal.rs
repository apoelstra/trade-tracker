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

//! Terminal Support
//!
//! Utilities to output RGB to a terminal
//!

use std::fmt;

fn hsv_to_rgb(hue: usize, sat: f64, light: f64) -> (usize, usize, usize) {
    assert!(hue <= 360, "Hue must lie between 0 and 360 inclusive.");
    assert!(sat >= 0.0, "Saturation must be >= 0.0");
    assert!(sat <= 1.0, "Saturation must be <= 1.0");
    assert!(light >= 0.0, "Lightness must be >= 0.0");
    assert!(light <= 1.0, "Lightness must be <= 1.0");

    let chroma = (1.0 - (2.0 * light - 1.0).abs()) * sat;
    let x = chroma * (1.0 - ((hue % 120) as f64 / 60.0 - 1.0).abs());
    let m = light - chroma / 2.0;
    let ret_f64 = if hue <= 60 {
        (chroma + m, x + m, 0.0 + m)
    } else if hue <= 120 {
        (x + m, chroma + m, 0.0 + m)
    } else if hue <= 180 {
        (0.0 + m, chroma + m, x + m)
    } else if hue <= 240 {
        (0.0 + m, x + m, chroma + m)
    } else if hue <= 300 {
        (x + m, 0.0 + m, chroma + m)
    } else {
        (chroma + m, 0.0 + m, x + m)
    };
    (
        (255.0 * ret_f64.0) as usize,
        (255.0 * ret_f64.1) as usize,
        (255.0 * ret_f64.2) as usize,
    )
}

pub fn format_color<D: fmt::Display>(disp: D, red: usize, green: usize, blue: usize) -> String {
    format!("\x1b[38;2;{red};{green};{blue}m{disp}\x1b[0m")
}

pub fn format_redgreen<D: fmt::Display>(disp: D, val: f64, red: f64, green: f64) -> String {
    let mut percent_red = if green >= red {
        (val - red) / (green - red)
    } else {
        1.0 - (val - green) / (red - green)
    };
    if percent_red < 0.0 {
        percent_red = 0.0;
    }
    if percent_red > 1.0 {
        percent_red = 1.0;
    }
    let rgb = hsv_to_rgb((percent_red * 120.0) as usize, 1.0, 0.6);
    format_color(disp, rgb.0, rgb.1, rgb.2)
}
