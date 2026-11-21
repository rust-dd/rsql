pub(crate) fn fixed(value: f64, places: usize) -> String {
    if value == 0. {
        return format!("{:.places$}", 0.);
    }
    if value.abs() >= 1e21 {
        if value.is_infinite() {
            return if value.is_sign_negative() {
                "-Infinity"
            } else {
                "Infinity"
            }
            .into();
        }
        return format!("{value:e}").replace('e', "e+");
    }
    // JavaScript toFixed rounds exact decimal midpoints away from zero.
    let fraction = value.abs().fract();
    if (fraction * (1 << places) as f64).fract() == 0.5 {
        let scale = 10_u64.pow(places as u32);
        let digits = (fraction * scale as f64).round() as u64;
        let whole = value.abs() as u64 + digits / scale;
        let sign = if value.is_sign_negative() { "-" } else { "" };
        if places == 0 {
            return format!("{sign}{whole}");
        }
        return format!("{sign}{whole}.{:0places$}", digits % scale);
    }
    format!("{value:.places$}")
}
