const CHUNK: usize = 64 * 1024;

fn edge_whitespace(c: char) -> bool {
    // ECMAScript String.trim includes BOM and excludes the NEL control character.
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

fn trimmed<'a>(
    mut text: &'a str,
    check: &mut impl FnMut() -> Result<(), String>,
) -> Result<&'a str, String> {
    while !text.is_empty() {
        check()?;
        let end = text.floor_char_boundary(CHUNK.min(text.len()));
        let chunk = &text[..end];
        let kept = chunk.trim_start_matches(edge_whitespace);
        text = &text[end - kept.len()..];
        if !kept.is_empty() {
            break;
        }
    }
    while !text.is_empty() {
        check()?;
        let start = text.ceil_char_boundary(text.len().saturating_sub(CHUNK));
        let kept = text[start..].trim_end_matches(edge_whitespace);
        text = &text[..start + kept.len()];
        if !kept.is_empty() {
            break;
        }
    }
    Ok(text)
}

pub(super) fn equal(
    left: &str,
    right: &str,
    check: &mut impl FnMut() -> Result<(), String>,
) -> Result<bool, String> {
    let left = trimmed(left, check)?;
    let right = trimmed(right, check)?;
    if left.len() != right.len() {
        return Ok(false);
    }
    for (a, b) in left
        .as_bytes()
        .chunks(CHUNK)
        .zip(right.as_bytes().chunks(CHUNK))
    {
        check()?;
        if a != b {
            return Ok(false);
        }
    }
    Ok(true)
}
