use plist::Value;
use roxmltree::Node;
use semver::Version;

const APPLE_DOCTYPE: &str = r#"<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">"#;

const XML_SPACE: [char; 4] = [' ', '\t', '\r', '\n'];

fn pseudo_attribute<'a>(input: &mut &'a str, name: &str) -> Option<&'a str> {
    let after_space = input.strip_prefix(XML_SPACE)?.trim_start_matches(XML_SPACE);
    let quoted = after_space
        .strip_prefix(name)?
        .trim_start_matches(XML_SPACE)
        .strip_prefix('=')?
        .trim_start_matches(XML_SPACE);
    let quote = quoted.chars().next().filter(|c| matches!(c, '\'' | '"'))?;
    let (value, rest) = quoted[1..].split_once(quote)?;
    *input = rest;
    Some(value)
}

fn document_body(xml: &str) -> Result<&str, &'static str> {
    let xml = xml.strip_prefix('\u{feff}').unwrap_or(xml);
    let Some(mut decl) = xml
        .strip_prefix("<?xml")
        .filter(|s| s.starts_with(XML_SPACE))
    else {
        return Ok(xml);
    };
    if pseudo_attribute(&mut decl, "version") != Some("1.0")
        || pseudo_attribute(&mut decl, "encoding").is_some_and(|v| !v.eq_ignore_ascii_case("UTF-8"))
        || pseudo_attribute(&mut decl, "standalone").is_some_and(|v| !matches!(v, "yes" | "no"))
    {
        return Err("Expected UTF-8 XML 1.0 bundle metadata");
    }
    decl.trim_start_matches(XML_SPACE)
        .strip_prefix("?>")
        .ok_or("Invalid XML declaration")
}

fn normalized_xml(mut body: &str) -> String {
    let mut xml = String::with_capacity(body.len() + 1);
    // Keep the document's declaration slot closed after consuming the optional BOM and prolog.
    xml.push('\n');
    while let Some((line, rest)) = body.split_once('\r') {
        xml.push_str(line);
        xml.push('\n');
        body = rest.strip_prefix('\n').unwrap_or(rest);
    }
    xml.push_str(body);
    xml
}

fn decoded_key_count(value: &Value) -> usize {
    match value {
        Value::Dictionary(dict) => dict.len() + dict.values().map(decoded_key_count).sum::<usize>(),
        Value::Array(values) => values.iter().map(decoded_key_count).sum(),
        _ => 0,
    }
}

fn parse(bytes: &[u8]) -> Result<Value, String> {
    if bytes.len() > super::HEADER_LIMIT {
        return Err("Bundle metadata too large.".into());
    }
    let xml = std::str::from_utf8(bytes).map_err(|_| "Invalid bundle metadata")?;
    let xml = normalized_xml(document_body(xml)?);
    if xml.contains("<![CDATA[") {
        return Err("Unsupported CDATA in bundle plist".into());
    }
    let doctype = xml.contains(APPLE_DOCTYPE)
        && xml.matches("<!DOCTYPE").count() == 1
        && !xml.contains("<!ENTITY");
    if xml.contains("<!DOCTYPE") && !doctype {
        return Err("Unsupported bundle plist declaration".into());
    }
    let doc = roxmltree::Document::parse_with_options(
        &xml,
        roxmltree::ParsingOptions {
            allow_dtd: doctype,
            nodes_limit: 4096,
            ..Default::default()
        },
    )
    .map_err(|_| "Invalid bundle plist")?;
    if doc
        .descendants()
        .filter_map(|n| n.pi())
        .any(|pi| pi.target.eq_ignore_ascii_case("xml"))
    {
        return Err("Misplaced XML declaration".into());
    }
    let root = doc.root_element();
    if !root.has_tag_name("plist")
        || root.attributes().len() != 1
        || root.attribute("version") != Some("1.0")
    {
        return Err("Invalid bundle plist root".into());
    }
    // plist's XML event reader uses local names and emits strings for both keys and values.
    // Check these XML distinctions and nesting before constructing its typed value tree.
    let mut key_count = 0;
    for node in root.descendants().filter(Node::is_element) {
        if node.ancestors().take(34).count() > 33
            || node.namespaces().next().is_some()
            || node.tag_name().namespace().is_some()
            || (node != root && node.attributes().len() != 0)
        {
            return Err("Unsupported bundle plist structure".into());
        }
        let mut children = node.children().filter(Node::is_element);
        let tag = node.tag_name().name();
        if matches!(tag, "plist" | "dict" | "array")
            && node.children().filter(Node::is_text).any(|v| {
                !v.text()
                    .unwrap_or_default()
                    .bytes()
                    .all(|b| matches!(b, b' ' | b'\t' | b'\r' | b'\n'))
            })
        {
            return Err("Unexpected text in bundle plist container".into());
        }
        match tag {
            "plist" if node == root => {
                if !children.next().is_some_and(|v| v.has_tag_name("dict"))
                    || children.next().is_some()
                {
                    return Err("Expected a single bundle dictionary".into());
                }
            }
            "dict" => {
                while let Some(key) = children.next() {
                    key_count += 1;
                    if !key.has_tag_name("key")
                        || !children.next().is_some_and(|v| !v.has_tag_name("key"))
                    {
                        return Err("Invalid bundle dictionary entry".into());
                    }
                }
            }
            "array" if !children.any(|v| v.has_tag_name("key")) => {}
            "key" | "string" | "data" | "date" | "integer" | "real"
                if children.next().is_none() => {}
            "true" | "false" if node.children().all(|v| v.is_comment() || v.is_pi()) => {}
            _ => return Err("Invalid bundle plist value".into()),
        }
    }
    let value = Value::from_reader_xml(xml.as_bytes()).map_err(|_| "Invalid bundle plist value")?;
    // Every replaced dictionary key removes at least one entry from the decoded tree.
    if decoded_key_count(&value) != key_count {
        return Err("Duplicate bundle dictionary key".into());
    }
    Ok(value)
}

pub(in crate::update) fn bundle(
    bytes: &[u8],
    version: &str,
    revision: &str,
) -> Result<Version, String> {
    let value = parse(bytes)?;
    let fields = value.as_dictionary().ok_or("Expected bundle dictionary")?;
    let string = |k| fields.get(k).and_then(Value::as_string);
    for (key, expected) in [
        ("CFBundleIdentifier", rsql_core::update::APP_ID),
        ("CFBundleName", "RSQL"),
        ("CFBundleDisplayName", "RSQL"),
        ("CFBundleExecutable", "rsql-desktop"),
        ("CFBundlePackageType", "APPL"),
        ("CFBundleShortVersionString", version),
        ("RSQLBuildRevision", revision),
    ] {
        if string(key) != Some(expected) {
            return Err(format!("Bundle identity mismatch ({key})."));
        }
    }
    let floor = string("LSMinimumSystemVersion").ok_or("Missing bundle OS version")?;
    let version = if floor.split('.').count() == 2 {
        format!("{floor}.0")
    } else {
        floor.into()
    };
    Version::parse(&version).map_err(|_| "Invalid bundle OS version".into())
}
