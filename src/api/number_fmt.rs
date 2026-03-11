//! Number formatting utilities for JavaScript value rendering.
//! Pure computational functions - no context dependencies.

pub(super) fn normalize_exponent(s: &str) -> String {
    let (base, exp) = match s.find('e').or_else(|| s.find('E')) {
        Some(idx) => (&s[..idx], &s[idx + 1..]),
        None => return s.to_string(),
    };
    let mut sign = '+';
    let mut digits = exp;
    if let Some(rest) = exp.strip_prefix('-') {
        sign = '-';
        digits = rest;
    } else if let Some(rest) = exp.strip_prefix('+') {
        digits = rest;
    }
    let digits = digits.trim_start_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    format!("{}e{}{}", base, sign, digits)
}

pub(super) fn format_fixed(n: f64, digits: i32) -> String {
    if n.is_nan() {
        return "NaN".to_string();
    }
    if n.is_infinite() {
        if n.is_sign_negative() {
            return "-Infinity".to_string();
        }
        return "Infinity".to_string();
    }
    let prec = digits.max(0) as i32;
    let factor = 10_f64.powi(prec);
    let rounded = round_half_away_from_zero(n * factor) / factor;
    if rounded == 0.0 && n.is_sign_negative() {
        if prec > 0 {
            return format!("-0.{:0width$}", 0, width = prec as usize);
        }
        return "-0".to_string();
    }
    format!("{:.*}", prec as usize, rounded)
}

pub(super) fn format_exponential(n: f64, digits: Option<i32>) -> String {
    if n.is_nan() {
        return "NaN".to_string();
    }
    if n.is_infinite() {
        if n.is_sign_negative() {
            return "-Infinity".to_string();
        }
        return "Infinity".to_string();
    }
    if let Some(d) = digits {
        return format_exponential_rounded(n, d);
    }
    let s = format!("{:e}", n);
    normalize_exponent(&s)
}

pub(super) fn format_radix_int(value: i64, radix: u32) -> String {
    let digits = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if radix < 2 || radix > 36 {
        return String::new();
    }
    if value == 0 {
        return "0".to_string();
    }
    let mut n = value as i128;
    let negative = n < 0;
    if negative {
        n = -n;
    }
    let radix_i = radix as i128;
    let mut out = Vec::new();
    while n > 0 {
        let rem = (n % radix_i) as usize;
        out.push(digits[rem]);
        n /= radix_i;
    }
    if negative {
        out.push(b'-');
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

pub(super) fn format_precision(n: f64, precision: i32) -> String {
    if n.is_nan() {
        return "NaN".to_string();
    }
    if n.is_infinite() {
        if n.is_sign_negative() {
            return "-Infinity".to_string();
        }
        return "Infinity".to_string();
    }
    if n == 0.0 {
        let mut out = String::from("0");
        if precision > 1 {
            out.push('.');
            for _ in 1..precision {
                out.push('0');
            }
        }
        return out;
    }
    let abs = n.abs();
    let exp = abs.log10().floor() as i32;
    if exp < -6 || exp >= precision {
        return format_exponential(n, Some(precision - 1));
    }
    let frac = (precision - exp - 1).max(0) as i32;
    let factor = 10_f64.powi(frac);
    let rounded = round_half_away_from_zero(n * factor) / factor;
    if rounded == 0.0 && n.is_sign_negative() {
        if frac > 0 {
            return format!("-0.{:0width$}", 0, width = frac as usize);
        }
        return "-0".to_string();
    }
    format!("{:.*}", frac as usize, rounded)
}

pub(super) fn round_half_away_from_zero(n: f64) -> f64 {
    if n.is_nan() || n.is_infinite() {
        return n;
    }
    if n.is_sign_negative() {
        return -round_half_away_from_zero(-n);
    }
    let floor = n.floor();
    let frac = n - floor;
    if frac > 0.5 {
        floor + 1.0
    } else if frac < 0.5 {
        floor
    } else {
        floor + 1.0
    }
}

pub(super) fn format_exponential_rounded(n: f64, digits: i32) -> String {
    if n.is_nan() {
        return "NaN".to_string();
    }
    if n.is_infinite() {
        if n.is_sign_negative() {
            return "-Infinity".to_string();
        }
        return "Infinity".to_string();
    }
    if n == 0.0 {
        let mut s = String::from("0");
        if digits > 0 {
            s.push('.');
            s.push_str(&"0".repeat(digits as usize));
        }
        s.push_str("e+0");
        return s;
    }
    let sign = if n.is_sign_negative() { "-" } else { "" };
    let abs = n.abs();
    let mut exp = abs.log10().floor() as i32;
    let mut normalized = abs / 10_f64.powi(exp);
    let factor = 10_f64.powi(digits);
    let rounded = round_half_away_from_zero(normalized * factor);
    normalized = rounded / factor;
    if normalized >= 10.0 {
        normalized /= 10.0;
        exp += 1;
    }
    let mut out = format!("{:.*}", digits as usize, normalized);
    out = normalize_exponent(&format!("{}{}e{:+}", sign, out, exp));
    out
}

