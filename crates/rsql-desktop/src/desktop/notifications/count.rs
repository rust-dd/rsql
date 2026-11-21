#[cfg(target_os = "macos")]
pub(crate) fn localized(rows: usize) -> String {
    use objc2_foundation::{NSNumber, NSNumberFormatter, NSNumberFormatterStyle};
    NSNumberFormatter::localizedStringFromNumber_numberStyle(
        &NSNumber::numberWithUnsignedLongLong(rows as u64),
        NSNumberFormatterStyle::DecimalStyle,
    )
    .to_string()
}

#[cfg(windows)]
pub(crate) fn localized(rows: usize) -> String {
    use windows::Globalization::NumberFormatting::DecimalFormatter;
    DecimalFormatter::new()
        .and_then(|formatter| {
            formatter.SetFractionDigits(0)?;
            formatter.SetIsGrouped(true)?;
            formatter.FormatUInt(rows as u64)
        })
        .map(|count| count.to_string())
        .unwrap_or_else(|_| rows.to_string())
}

#[cfg(not(any(target_os = "macos", windows)))]
pub(crate) fn localized(rows: usize) -> String {
    use num_format::{Buffer, Locale};
    let locale = sys_locale::get_locale().unwrap_or_default();
    let locale = Locale::from_name(&locale)
        .ok()
        .or_else(|| Locale::from_name(locale.split('-').next().unwrap_or("en")).ok())
        .unwrap_or(Locale::en);
    let mut count = Buffer::default();
    count.write_formatted(&rows, &locale);
    count.as_str().to_owned()
}
