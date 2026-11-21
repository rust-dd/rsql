use std::{collections::BTreeMap, ops::Range, sync::Arc};

use gpui::SharedString;
use markdown::{
    ParseOptions,
    mdast::{self, Node},
    unist::Point,
};

use crate::text::{
    document::ParsedDocument,
    markdown_ext::MarkdownParseContext,
    node::{
        self, BlockNode, CodeBlock, ImageNode, InlineNode, LinkMark, NodeContext, Paragraph,
        SourceSegment, Span, Table, TableRow, TextMark,
    },
};

/// Parse Markdown into a tree of nodes.
pub(crate) fn parse(source: &str, cx: &mut NodeContext) -> Result<ParsedDocument, SharedString> {
    let options = cx.markdown_extensions.parse_options();
    let mut root =
        markdown::to_mdast(source, &options).map_err(|e| SharedString::from(e.to_string()))?;
    let mut prose = options;
    prose.constructs.math_text = false;
    prose.constructs.math_flow = false;
    flatten_unclaimed_math(&mut root, source, &prose, cx);
    Ok(ast_to_document(source, root, cx))
}

/// Math parsing is on by default, so prose that merely contains two dollar
/// signs ("$5 and $10") parses as an inline math node. When no plugin claims
/// such a node, the text between the dollars must stay ordinary Markdown:
/// emphasis, inline HTML and links inside it render as they would anywhere
/// else, and inline HTML tags may pair with tags outside the span. Re-parse
/// the node's source without the math constructs and, when it holds any
/// markup, splice the result into its parent with positions shifted back into
/// the document. Plain prose keeps the literal node, whose atomic source
/// mapping the selection tests rely on.
fn flatten_unclaimed_math(node: &mut Node, source: &str, options: &ParseOptions, cx: &NodeContext) {
    let Some(children) = node.children_mut() else {
        return;
    };
    let mut ix = 0;
    while ix < children.len() {
        let prose = match &children[ix] {
            Node::InlineMath(_) => {
                let parse_cx = MarkdownParseContext::new(source, cx.offset);
                if cx
                    .markdown_extensions
                    .parse_inline(&children[ix], &parse_cx)
                    .is_some()
                {
                    None
                } else {
                    reparse_as_prose(&children[ix], source, options)
                }
            }
            _ => None,
        };
        if let Some(prose) = prose {
            let count = prose.len();
            children.splice(ix..=ix, prose);
            ix += count;
            continue;
        }
        flatten_unclaimed_math(&mut children[ix], source, options, cx);
        ix += 1;
    }
}

fn reparse_as_prose(node: &Node, source: &str, options: &ParseOptions) -> Option<Vec<Node>> {
    let position = node.position()?.clone();
    let literal = source.get(position.start.offset..position.end.offset)?;
    if !may_hold_inline_markup(literal) {
        return None;
    }
    let Ok(Node::Root(mut root)) = markdown::to_mdast(literal, options) else {
        return None;
    };
    let [Node::Paragraph(paragraph)] = root.children.as_mut_slice() else {
        return None;
    };
    if paragraph
        .children
        .iter()
        .all(|child| matches!(child, Node::Text(_)))
    {
        return None;
    }
    let mut children = std::mem::take(&mut paragraph.children);
    for child in &mut children {
        shift_positions(child, &position.start);
    }
    Some(children)
}

/// A cheap gate before re-parsing: every inline construct starts with one of
/// these bytes (tags, emphasis, code, links, images, escapes, entities,
/// strikethrough) or is a GFM autolink literal.
fn may_hold_inline_markup(literal: &str) -> bool {
    literal.bytes().any(|byte| {
        matches!(
            byte,
            b'<' | b'*' | b'_' | b'[' | b'`' | b'~' | b'\\' | b'!' | b'&'
        )
    }) || literal.contains("://")
        || literal.contains("www.")
}

fn shift_positions(node: &mut Node, origin: &Point) {
    if let Some(position) = node.position_mut() {
        for point in [&mut position.start, &mut position.end] {
            let first_line = point.line == 1;
            point.offset += origin.offset;
            point.line += origin.line - 1;
            if first_line {
                point.column += origin.column - 1;
            }
        }
    }
    if let Some(children) = node.children_mut() {
        for child in children {
            shift_positions(child, origin);
        }
    }
}

enum InlineGroup<'a> {
    Node(&'a Node),
    Marked(TextMark, &'a [Node]),
}

/// CommonMark hands each raw inline tag over as its own `Html` node, so
/// `<strong>x</strong>` arrives as three siblings and the tags alone carry no
/// text. Pair the formatting tags this renderer knows with their closing tag
/// among the siblings and treat what lies between as a marked run, the same
/// way `**x**` is handled.
fn inline_groups(children: &[Node]) -> Vec<InlineGroup<'_>> {
    let mut groups = Vec::with_capacity(children.len());
    let mut ix = 0;
    while ix < children.len() {
        if let Some((false, name, attrs)) = inline_html_tag(&children[ix])
            && let Some(mark) = inline_html_mark(&name, attrs)
            && let Some(close) = matching_close_tag(children, ix, &name)
        {
            groups.push(InlineGroup::Marked(mark, &children[ix + 1..close]));
            ix = close + 1;
            continue;
        }
        groups.push(InlineGroup::Node(&children[ix]));
        ix += 1;
    }
    groups
}

fn inline_html_tag(node: &Node) -> Option<(bool, String, &str)> {
    let Node::Html(html) = node else {
        return None;
    };
    let inner = html.value.trim().strip_prefix('<')?.strip_suffix('>')?;
    if inner.ends_with('/') {
        return None;
    }
    let (closing, rest) = match inner.strip_prefix('/') {
        Some(rest) => (true, rest),
        None => (false, inner),
    };
    let name_len = rest.chars().take_while(char::is_ascii_alphanumeric).count();
    let name = rest[..name_len].to_ascii_lowercase();
    let attrs = rest[name_len..].trim();
    (!name.is_empty()).then_some((closing, name, attrs))
}

fn inline_html_attr<'a>(attrs: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = attrs;
    while !rest.is_empty() {
        rest = rest.trim_start();
        let key_len = rest
            .find(|c: char| c.is_whitespace() || matches!(c, '=' | '/' | '>'))
            .unwrap_or(rest.len());
        if key_len == 0 {
            break;
        }
        let key = &rest[..key_len];
        rest = rest[key_len..].trim_start();
        let value = if let Some(after_eq) = rest.strip_prefix('=') {
            rest = after_eq.trim_start();
            if let Some(quote) = rest
                .as_bytes()
                .first()
                .copied()
                .filter(|b| *b == b'"' || *b == b'\'')
            {
                rest = &rest[1..];
                let end = rest.find(quote as char)?;
                let value = &rest[..end];
                rest = &rest[end + 1..];
                value
            } else {
                let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
                let value = &rest[..end];
                rest = &rest[end..];
                value
            }
        } else {
            ""
        };
        if key.eq_ignore_ascii_case(name) {
            return Some(value);
        }
    }
    None
}

fn inline_html_mark(name: &str, attrs: &str) -> Option<TextMark> {
    Some(match name {
        "strong" | "b" => TextMark::default().bold(),
        "em" | "i" => TextMark::default().italic(),
        "u" => TextMark::default().underline(),
        "s" | "del" | "strike" => TextMark::default().strikethrough(),
        "mark" => {
            let color = super::html::mark_color_from_values(
                inline_html_attr(attrs, "color"),
                inline_html_attr(attrs, "style"),
            )
            .unwrap_or_else(super::html::default_mark_color);
            TextMark::default().highlight(color)
        }
        _ => return None,
    })
}

fn matching_close_tag(children: &[Node], open: usize, name: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (ix, child) in children.iter().enumerate().skip(open + 1) {
        match inline_html_tag(child) {
            Some((false, tag, _)) if tag == name => depth += 1,
            Some((true, tag, _)) if tag == name => {
                if depth == 0 {
                    return Some(ix);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

fn parse_inline_children(
    source: &str,
    paragraph: &mut Paragraph,
    children: &[Node],
    cx: &mut NodeContext,
) -> String {
    for group in inline_groups(children) {
        match group {
            InlineGroup::Node(child) => {
                parse_paragraph(source, paragraph, child, cx);
            }
            InlineGroup::Marked(mark, nodes) => {
                merge_children_with_mark(source, paragraph, nodes, mark, cx);
            }
        }
    }
    normalize_inline_boundaries(paragraph)
}

/// Drops a text node's leading white space when the previous rendered
/// character is white space or a line break, so a run folded inside its node
/// stays one space across a dropped HTML comment, and nothing follows a
/// `<br>` or a hard break indented by the space the line ending became
/// (#3710). Returns the paragraph's rendered text.
///
/// The source mapping shifts with the trim: the removed white space's
/// segments and marks drop or shrink with it, so selection and copy still
/// land on the source the surviving characters were drawn from.
fn normalize_inline_boundaries(paragraph: &mut Paragraph) -> String {
    let mut at_line_start = true;
    let mut previous_white_space = false;
    let mut text = String::new();

    for node in &mut paragraph.children {
        if node.custom.is_some() || node.image.is_some() {
            // Unknown rendered width: keep the white space around it.
            at_line_start = false;
            previous_white_space = false;
            continue;
        }
        if node.text == "\n" && node.marks.is_empty() {
            // A rendered line break, from `<br>` or a hard break. White
            // space held before it is trailing and goes with the line.
            at_line_start = true;
            previous_white_space = false;
            text.push('\n');
            continue;
        }
        let leading = node.text.len() - node.text.trim_start_matches([' ', '\t']).len();
        // Inline code keeps its white space, so the fold stops where a code
        // mark begins: `` x `  y` `` keeps the span's leading spaces even
        // though the run before it ends with one (#3710 review).
        let code_start = node
            .marks
            .iter()
            .filter(|(range, mark)| mark.code && range.end > 0)
            .map(|(range, _)| range.start)
            .min();
        let leading = leading.min(code_start.unwrap_or(leading));
        if leading > 0 && (at_line_start || previous_white_space) {
            trim_leading_white_space(node, leading);
        }
        if node.text.is_empty() {
            continue;
        }
        at_line_start = false;
        previous_white_space = node.text.ends_with([' ', '\t']);
        text.push_str(&node.text);
    }

    paragraph
        .children
        .retain(|node| node.custom.is_some() || node.image.is_some() || !node.text.is_empty());
    text
}

/// Removes the first `count` bytes of white space from the node's text,
/// shifting its marks and source segments over the removed range.
fn trim_leading_white_space(node: &mut InlineNode, count: usize) {
    let old_text = std::mem::take(&mut node.text);
    node.text = old_text[count..].into();

    for (range, _) in &mut node.marks {
        let start = range.start.saturating_sub(count).min(node.text.len());
        let end = range.end.saturating_sub(count).min(node.text.len());
        *range = start..end.max(start);
    }
    node.marks.retain(|(range, _)| range.start < range.end);

    for segment in &mut node.source_segments {
        if segment.rendered.end <= count {
            // Wholly inside the dropped white space.
            segment.rendered = 0..0;
            continue;
        }
        let rendered_end = segment.rendered.end - count;
        if segment.rendered.start >= count {
            // Wholly after it: the source stays, the bytes shift.
            segment.rendered = (segment.rendered.start - count)..rendered_end;
        } else if segment.linear {
            // Straddling the cut. Linear segments map byte for byte, so the
            // dropped bytes also drop from the source; a merged run keeps
            // drawing the survivors from the bytes they came from.
            segment.source.start += count;
            segment.rendered = 0..rendered_end;
        } else {
            // A non-linear segment straddling the cut has no exact mapping
            // for its survivors; drop it rather than guess.
            segment.rendered = 0..0;
        }
    }
    node.source_segments.retain(|segment| {
        segment.rendered.start < segment.rendered.end && segment.source.start < segment.source.end
    });
}

/// Folds each run of spaces and tabs in drawn text to a single space, the
/// way `white-space: normal` renders it (#3710). Line endings have already
/// become spaces here; white space inside inline code never reaches this
/// path, and runs at a line break are dropped by
/// `normalize_inline_boundaries`.
fn collapse_white_space(text: &str) -> String {
    let mut collapsed = String::with_capacity(text.len());
    let mut in_white_space = false;
    for ch in text.chars() {
        if matches!(ch, ' ' | '\t') {
            if !in_white_space {
                collapsed.push(' ');
                in_white_space = true;
            }
        } else {
            collapsed.push(ch);
            in_white_space = false;
        }
    }
    collapsed
}

fn parse_table_row(source: &str, table: &mut Table, node: &mdast::TableRow, cx: &mut NodeContext) {
    let mut row = TableRow::default();
    node.children.iter().for_each(|c| {
        match c {
            Node::TableCell(cell) => {
                parse_table_cell(source, &mut row, cell, cx);
            }
            _ => {}
        };
    });
    // GFM fixes the column count with the header/delimiter. Leave missing
    // cells implicit; the renderer places them on the shared column grid.
    row.children.truncate(table.column_aligns.len());
    table.children.push(row);
}

fn parse_table_cell(
    source: &str,
    row: &mut node::TableRow,
    node: &mdast::TableCell,
    cx: &mut NodeContext,
) {
    let mut paragraph = Paragraph::default();
    parse_inline_children(source, &mut paragraph, &node.children, cx);
    let table_cell = node::TableCell {
        children: paragraph,
        ..Default::default()
    };
    row.children.push(table_cell);
}

/// Push a text run with its existing `marks` plus `new_mark` across the full
/// run.
///
/// If the last mark already covers the full run, merge into it. Otherwise add a
/// new full-run mark. Empty runs are skipped so callers can flush freely.
fn push_merged(
    paragraph: &mut Paragraph,
    text: String,
    marks: Vec<(Range<usize>, TextMark)>,
    source_segments: Vec<SourceSegment>,
    new_mark: TextMark,
) {
    if text.is_empty() {
        return;
    }

    let mut node = InlineNode::new(text)
        .marks(marks)
        .source_segments(source_segments);
    let len = node.text.len();
    if let Some(last) = node.marks.last_mut()
        && last.0.start == 0
        && last.0.end == len
    {
        last.1.merge(new_mark);
    } else {
        node.marks.push((0..len, new_mark));
    }
    paragraph.push(node);
}

/// Parse `children` and apply `mark` across each emitted text run.
///
/// Nested child marks are kept and shifted to match the combined text for the
/// current run, which lets nested emphasis like `**_x_**` render as both bold
/// and italic. Inline images split the run and are emitted as sibling image
/// nodes. The return value is the plain text from all children, for callers that
/// need to pass text back to their parent node.
fn merge_children_with_mark(
    source: &str,
    paragraph: &mut Paragraph,
    children: &[mdast::Node],
    mark: TextMark,
    cx: &mut NodeContext,
) -> String {
    let mut text = String::new();
    let mut merged_text = String::new();
    let mut merged_marks = Vec::new();
    let mut merged_source_segments = Vec::new();

    for group in inline_groups(children) {
        let mut child_paragraph = Paragraph::default();
        let child_text = match group {
            InlineGroup::Node(child) => parse_paragraph(source, &mut child_paragraph, child, cx),
            InlineGroup::Marked(child_mark, nodes) => {
                merge_children_with_mark(source, &mut child_paragraph, nodes, child_mark, cx)
            }
        };
        text.push_str(&child_text);

        for mut node in child_paragraph.children {
            if node.custom.is_some() {
                push_merged(
                    paragraph,
                    std::mem::take(&mut merged_text),
                    std::mem::take(&mut merged_marks),
                    std::mem::take(&mut merged_source_segments),
                    mark.clone(),
                );
                if let Some((_, existing)) = node.marks.first_mut() {
                    existing.merge(mark.clone());
                } else {
                    node.marks.push((0..node.text.len(), mark.clone()));
                }
                paragraph.push(node);
                continue;
            }
            let merged_offset = merged_text.len();
            merged_text.push_str(&node.text);
            merged_source_segments.extend(node.source_segments.drain(..).map(|mut segment| {
                segment.rendered.start += merged_offset;
                segment.rendered.end += merged_offset;
                segment
            }));

            for (range, child_mark) in node.marks {
                merged_marks.push((
                    range.start + merged_offset..range.end + merged_offset,
                    child_mark,
                ));
            }

            if let Some(mut image) = node.image {
                if let Some(link_mark) = mark.link.clone() {
                    image.link = Some(link_mark);
                }

                // GPUI InteractiveText does not support inline images, so
                // flush the accumulated text run and emit the image as its
                // own sibling InlineNode.
                push_merged(
                    paragraph,
                    std::mem::take(&mut merged_text),
                    std::mem::take(&mut merged_marks),
                    std::mem::take(&mut merged_source_segments),
                    mark.clone(),
                );
                paragraph.push(InlineNode::image(image));
            }
        }
    }

    push_merged(
        paragraph,
        merged_text,
        merged_marks,
        merged_source_segments,
        mark,
    );
    text
}

fn source_segments(
    source: &str,
    rendered: &str,
    span: Option<Span>,
    source_offset: usize,
    include_preceding_escape: bool,
) -> Vec<SourceSegment> {
    let Some(span) = span else {
        return Vec::new();
    };
    let local_start = span.start.saturating_sub(source_offset);
    let local_end = span.end.saturating_sub(source_offset);
    let Some(raw) = source.get(local_start..local_end) else {
        return Vec::new();
    };

    let mut segments = aligned_source_segments(raw, rendered, span.start, true, true);

    // The text node of an escaped character starts after its backslash. The
    // backslash and the character map as a whole; the rest of the run stays
    // linear.
    if include_preceding_escape
        && let Some(previous) = local_start.checked_sub(1)
        && source.as_bytes().get(previous) == Some(&b'\\')
        && let Some(first) = segments.first_mut()
    {
        let first_char_len = rendered[first.rendered.start..]
            .chars()
            .next()
            .map_or(0, char::len_utf8);
        if first.linear && first.rendered.len() > first_char_len {
            let rest = SourceSegment {
                rendered: first.rendered.start + first_char_len..first.rendered.end,
                source: first.source.start + first_char_len..first.source.end,
                linear: true,
            };
            first.rendered.end = rest.rendered.start;
            first.source.end = rest.source.start;
            segments.insert(1, rest);
        }
        let first = &mut segments[0];
        first.source.start -= 1;
        first.linear = false;
    }
    segments
}

fn aligned_source_segments(
    raw: &str,
    rendered: &str,
    source_offset: usize,
    decode_entities: bool,
    decode_escapes: bool,
) -> Vec<SourceSegment> {
    let mut segments = Vec::new();
    let mut raw_cursor = 0;
    let mut rendered_start = 0;
    let mut whitespace_end = 0;
    let mut source_positions = None;
    let mut entity_cursor = None;
    let mut entity = None;
    // Only the final LF may absorb source-only text before it. Find the
    // preceding line boundary once instead of recounting the suffix per LF.
    let final_line_start = raw
        .strip_suffix('\n')
        .and_then(|prefix| prefix.rfind('\n'))
        .map_or(0, |newline| newline + 1);

    while rendered_start < rendered.len() {
        if decode_entities && entity_cursor != Some(raw_cursor) {
            entity = decoded_entity(&raw[raw_cursor..]);
            entity_cursor = Some(raw_cursor);
        }
        if let Some((decoded, source_len)) = &entity
            && rendered[rendered_start..].starts_with(decoded.as_str())
        {
            let rendered_end = rendered_start + decoded.len();
            push_source_segment(
                &mut segments,
                SourceSegment {
                    rendered: rendered_start..rendered_end,
                    source: (source_offset + raw_cursor)
                        ..(source_offset + raw_cursor + *source_len),
                    // Even when the decoded characters take as many bytes as the
                    // entity, as `∾̳` and `&acE;` do, they are not one for one.
                    linear: false,
                },
            );
            rendered_start = rendered_end;
            raw_cursor += *source_len;
            continue;
        }

        let rendered_char = rendered[rendered_start..]
            .chars()
            .next()
            .expect("rendered cursor must be on a character boundary");
        let rendered_end = rendered_start + rendered_char.len_utf8();
        let remainder = &raw[raw_cursor..];
        if rendered_char == ' ' && raw_cursor >= whitespace_end {
            // Reuse this boundary while consuming a run of literal spaces.
            // Otherwise a long horizontal-whitespace run also costs O(n^2).
            whitespace_end = raw_cursor
                + remainder
                    .bytes()
                    .position(|byte| !matches!(byte, b' ' | b'\t'))
                    .unwrap_or(remainder.len());
        }
        let (relative_start, source_len) = if rendered_char == ' '
            && matches!(raw.as_bytes().get(whitespace_end), Some(b'\n' | b'\r'))
        {
            let newline_len = if raw[whitespace_end..].starts_with("\r\n") {
                2
            } else {
                1
            };
            (whitespace_end - raw_cursor, newline_len)
        } else if rendered_char == '\n'
            && remainder.ends_with('\n')
            && raw_cursor >= final_line_start
        {
            (0, remainder.len())
        } else if decode_escapes
            && let Some(escaped) = remainder.strip_prefix('\\')
            && escaped.starts_with(rendered_char)
        {
            (0, 1 + rendered_char.len_utf8())
        } else if remainder.starts_with(rendered_char) {
            (0, rendered_char.len_utf8())
        } else if let Some(source_start) =
            source_char_offset(raw, raw_cursor, rendered_char, &mut source_positions)
        {
            (source_start - raw_cursor, rendered_char.len_utf8())
        } else {
            // Decoded entities and other source-only syntax have no exact
            // rendered-byte mapping. Leave a rendered gap for this
            // character, but keep aligning later characters in the node.
            rendered_start = rendered_end;
            continue;
        };
        let source_start = raw_cursor + relative_start;
        let source_end = source_start + source_len;
        push_source_segment(
            &mut segments,
            SourceSegment {
                rendered: rendered_start..rendered_end,
                source: (source_offset + source_start)..(source_offset + source_end),
                linear: source_len == rendered_char.len_utf8(),
            },
        );
        raw_cursor = source_end;
        rendered_start = rendered_end;
    }
    segments
}

fn source_char_offset(
    raw: &str,
    raw_cursor: usize,
    character: char,
    positions: &mut Option<BTreeMap<char, Vec<usize>>>,
) -> Option<usize> {
    if let Some(positions) = positions.as_ref() {
        let offsets = positions.get(&character)?;
        return offsets
            .get(offsets.partition_point(|&offset| offset < raw_cursor))
            .copied();
    }
    if let Some(offset) = raw[raw_cursor..].find(character) {
        return Some(raw_cursor + offset);
    }

    // Successful searches advance raw_cursor, so their scans do not overlap.
    // A missing character does not advance it. Index the remaining source only
    // after the first miss so later gaps cannot repeatedly scan that suffix.
    let mut indexed = BTreeMap::<char, Vec<usize>>::new();
    for (offset, character) in raw[raw_cursor..].char_indices() {
        indexed
            .entry(character)
            .or_default()
            .push(raw_cursor + offset);
    }
    *positions = Some(indexed);
    None
}

/// Pushes `segment`, merging it into the previous one when both are linear and
/// adjacent, so that a merged run still maps byte for byte. Any other segment
/// is kept whole on its own.
fn push_source_segment(segments: &mut Vec<SourceSegment>, segment: SourceSegment) {
    if let Some(previous) = segments.last_mut()
        && previous.linear
        && segment.linear
        && previous.rendered.end == segment.rendered.start
        && previous.source.end == segment.source.start
    {
        previous.rendered.end = segment.rendered.end;
        previous.source.end = segment.source.end;
    } else {
        segments.push(segment);
    }
}

fn decoded_entity(source: &str) -> Option<(String, usize)> {
    let candidate = source.strip_prefix('&')?;
    let candidate_end = candidate
        .bytes()
        .position(|byte| byte == b';' || !(byte.is_ascii_alphanumeric() || byte == b'#'))?;
    if candidate.as_bytes()[candidate_end] != b';' {
        return None;
    }
    let semicolon = candidate_end + 1;
    let name = &source[1..=semicolon];
    let decoded = if let Some(number) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X"))
    {
        char::from_u32(u32::from_str_radix(number.strip_suffix(';')?, 16).ok()?)?.to_string()
    } else if let Some(number) = name.strip_prefix('#') {
        char::from_u32(number.strip_suffix(';')?.parse().ok()?)?.to_string()
    } else {
        let &(first, second) = html5ever::data::NAMED_ENTITIES.get(name)?;
        let mut decoded = char::from_u32(first)?.to_string();
        if second != 0 {
            decoded.push(char::from_u32(second)?);
        }
        decoded
    };
    Some((decoded, semicolon + 1))
}

fn code_source_segments(
    source: &str,
    code: &str,
    span: Option<Span>,
    source_offset: usize,
) -> Vec<SourceSegment> {
    let Some(span) = span else {
        return Vec::new();
    };
    let Some(raw) = source
        .get(span.start.saturating_sub(source_offset)..span.end.saturating_sub(source_offset))
    else {
        return Vec::new();
    };
    let trimmed = raw.trim_start();
    let fence = trimmed
        .chars()
        .next()
        .filter(|character| matches!(character, '`' | '~'))
        .map(|character| {
            let len = trimmed
                .chars()
                .take_while(|candidate| candidate == &character)
                .count();
            (character, len)
        })
        .filter(|(_, len)| *len >= 3);
    let (body_start, body_end) = if let Some((fence, fence_len)) = fence {
        let start = raw.find('\n').map_or(raw.len(), |newline| newline + 1);
        let last_line = raw.rfind('\n').map_or(start, |newline| newline + 1);
        let closing = raw[last_line..].trim();
        let is_closing = closing.chars().count() >= fence_len
            && closing.chars().all(|character| character == fence);
        (start, if is_closing { last_line } else { raw.len() })
    } else {
        (0, raw.len())
    };

    aligned_source_segments(
        &raw[body_start..body_end],
        code,
        span.start + body_start,
        false,
        false,
    )
}

fn mapped_inline(
    source: &str,
    text: impl Into<SharedString>,
    node: &mdast::Node,
    cx: &NodeContext,
) -> InlineNode {
    let text = text.into();
    let span = node.position().map(|position| Span {
        start: cx.offset + position.start.offset,
        end: cx.offset + position.end.offset,
    });
    let segments = source_segments(
        source,
        &text,
        span,
        cx.offset,
        matches!(node, Node::Text(_)),
    );
    InlineNode::new(text).source_segments(segments)
}

fn append_inline_html_blocks(paragraph: &mut Paragraph, blocks: Vec<BlockNode>) -> Option<String> {
    let mut text = String::new();

    for block in blocks {
        match block {
            BlockNode::Root { children, .. } => {
                text.push_str(&append_inline_html_blocks(paragraph, children)?);
            }
            BlockNode::Paragraph(html_paragraph) => {
                text.push_str(&html_paragraph.text());
                for child in html_paragraph.children {
                    paragraph.push(child);
                }
            }
            BlockNode::Break { .. } => {
                text.push('\n');
                paragraph.push(InlineNode::new("\n"));
            }
            _ => return None,
        }
    }

    Some(text)
}

fn parse_paragraph(
    source: &str,
    paragraph: &mut Paragraph,
    node: &mdast::Node,
    cx: &mut NodeContext,
) -> String {
    let span = node.position().map(|pos| Span {
        start: cx.offset + pos.start.offset,
        end: cx.offset + pos.end.offset,
    });
    if let Some(span) = span {
        paragraph.set_span(span);
    }

    let parse_cx = MarkdownParseContext::new(source, cx.offset);
    if let Some(mut custom) = cx.markdown_extensions.parse_inline(node, &parse_cx) {
        custom.set_span(span);
        let custom = custom.with_inline_source(parse_cx.node_source(node).unwrap_or_default());
        let text = custom.as_text().to_string();
        paragraph.push(InlineNode::custom(custom));
        return text;
    }

    let mut text = String::new();

    match node {
        Node::Paragraph(val) => {
            text.push_str(&parse_inline_children(source, paragraph, &val.children, cx));
        }
        Node::Text(val) => {
            // A CommonMark *soft* break lives inside this value as a plain
            // line ending. The renderer treats a newline in a text run as a
            // line break, so a paragraph hard-wrapped in the source would
            // render one visual line per source line instead of reflowing to
            // the available width. Collapse soft breaks to spaces; *hard*
            // breaks never reach here, they arrive as their own Node::Break.
            //
            // mdast hands the line ending over exactly as the source wrote it,
            // so a CRLF document still carries its carriage return here. Take
            // the CR with the newline: dropping only the newline would strand
            // the CR in the middle of the reflowed line.
            //
            // Runs of spaces and tabs then fold to one space, the way
            // `white-space: normal` draws them (#3710); the source mapping
            // re-aligns over the folded bytes on its own. Leading runs at a
            // line break — after `<br>` or a hard break — are dropped by
            // `normalize_inline_boundaries`, which sees the nodes whole.
            text = collapse_white_space(&val.value.replace("\r\n", " ").replace(['\n', '\r'], " "));
            paragraph.push(mapped_inline(source, text.clone(), node, cx))
        }
        Node::Emphasis(val) => {
            text = merge_children_with_mark(
                source,
                paragraph,
                &val.children,
                TextMark::default().italic(),
                cx,
            );
        }
        Node::Strong(val) => {
            text = merge_children_with_mark(
                source,
                paragraph,
                &val.children,
                TextMark::default().bold(),
                cx,
            );
        }
        Node::Delete(val) => {
            text = merge_children_with_mark(
                source,
                paragraph,
                &val.children,
                TextMark::default().strikethrough(),
                cx,
            );
        }
        Node::InlineCode(val) => {
            text = val.value.clone();
            let span = node.position().map(|position| Span {
                start: cx.offset + position.start.offset,
                end: cx.offset + position.end.offset,
            });
            paragraph.push(
                InlineNode::new(text.clone())
                    .source_segments(code_source_segments(source, &text, span, cx.offset))
                    .marks(vec![(0..text.len(), TextMark::default().code())]),
            );
        }
        Node::Link(val) => {
            let link_mark = Some(LinkMark {
                url: val.url.clone().into(),
                title: val.title.clone().map(|s| s.into()),
                ..Default::default()
            });

            text = merge_children_with_mark(
                source,
                paragraph,
                &val.children,
                TextMark {
                    link: link_mark,
                    ..Default::default()
                },
                cx,
            );
        }
        Node::Image(raw) => {
            paragraph.push_image(ImageNode {
                url: raw.url.clone().into(),
                title: raw.title.clone().map(|t| t.into()),
                alt: Some(raw.alt.clone().into()),
                span: raw.position.as_ref().map(|position| Span {
                    start: cx.offset + position.start.offset,
                    end: cx.offset + position.end.offset,
                }),
                ..Default::default()
            });
        }
        Node::ImageReference(raw) => {
            if let Some(definition) = cx.link_refs.get(raw.identifier.as_str()) {
                paragraph.push_image(ImageNode {
                    url: definition.url.clone().into(),
                    title: definition.title.clone(),
                    alt: Some(raw.alt.clone().into()),
                    span,
                    ..Default::default()
                });
            }
        }
        Node::Break(_) => {
            // Hard line break (trailing two spaces / backslash). Mirror the
            // inline-HTML <br> path: emit a newline inline node so the break
            // renders instead of silently concatenating adjacent lines.
            text.push('\n');
            paragraph.push(mapped_inline(source, "\n", node, cx));
        }
        Node::InlineMath(raw) => {
            // Math parsing is on by default, so ordinary prose that merely
            // contains dollar signs ("spent $5 and $10") arrives here as a
            // math node whenever no inline plugin claimed it. Emitting
            // `raw.value` would drop the delimiters and restyle the run as
            // code, silently rewriting what the author typed. Fall back to the
            // original source so unclaimed math stays literal.
            text = parse_cx
                .node_source(node)
                .map(str::to_string)
                .unwrap_or_else(|| raw.value.clone());
            paragraph.push(mapped_inline(source, text.clone(), node, cx));
        }
        Node::MdxTextExpression(raw) => {
            text = raw.value.clone();
            let span = node.position().map(|position| Span {
                start: cx.offset + position.start.offset,
                end: cx.offset + position.end.offset,
            });
            paragraph.push(
                InlineNode::new(text.clone())
                    .source_segments(code_source_segments(source, &text, span, cx.offset))
                    .marks(vec![(0..text.len(), TextMark::default())]),
            );
        }
        Node::Html(val) => match super::html::parse(&val.value, cx) {
            Ok(el) => {
                if let Some(inline_text) =
                    append_inline_html_blocks(paragraph, Arc::unwrap_or_clone(el.blocks))
                {
                    text = inline_text;
                } else {
                    if cfg!(debug_assertions) {
                        tracing::warn!("unsupported inline html tag: {:#?}", val.value);
                    }
                }
            }
            Err(err) => {
                if cfg!(debug_assertions) {
                    tracing::warn!("failed parsing html: {:#?}", err);
                }

                text.push_str(&val.value);
            }
        },
        Node::FootnoteReference(foot) => {
            let prefix = format!("[{}]", foot.identifier);
            paragraph.push(mapped_inline(source, prefix.clone(), node, cx).marks(vec![(
                0..prefix.len(),
                TextMark {
                    italic: true,
                    ..Default::default()
                },
            )]));
        }
        Node::LinkReference(link) => {
            let link_mark = LinkMark {
                url: "".into(),
                title: link.label.clone().map(Into::into),
                identifier: Some(link.identifier.clone().into()),
            };

            text = merge_children_with_mark(
                source,
                paragraph,
                &link.children,
                TextMark {
                    link: Some(link_mark),
                    ..Default::default()
                },
                cx,
            );
        }
        _ => {
            if cfg!(debug_assertions) {
                tracing::warn!("unsupported inline node: {:#?}", node);
            }
        }
    }

    text
}

/// Definitions may follow their references or live inside a container. The
/// parser normalizes identifiers; CommonMark uses the first definition.
fn collect_definitions(node: &Node, definitions: &mut BTreeMap<String, LinkMark>) {
    if let Node::Definition(def) = node {
        definitions
            .entry(def.identifier.clone())
            .or_insert_with(|| LinkMark {
                url: def.url.clone().into(),
                identifier: Some(def.identifier.clone().into()),
                title: def.title.clone().map(Into::into),
            });
    }
    if let Some(children) = node.children() {
        for child in children {
            collect_definitions(child, definitions);
        }
    }
}

fn ast_to_document(source: &str, root: mdast::Node, cx: &mut NodeContext) -> ParsedDocument {
    let mut definitions = BTreeMap::new();
    collect_definitions(&root, &mut definitions);
    for (identifier, definition) in definitions {
        cx.add_ref(identifier.into(), definition);
    }

    let root = match root {
        Node::Root(r) => r,
        _ => panic!("expected root node"),
    };

    let has_surfaces = cx.markdown_extensions.has_block_surfaces();
    let blocks = root
        .children
        .into_iter()
        .map(|node| match node {
            Node::Blockquote(quote) if has_surfaces => blockquote_to_node(source, quote, cx),
            node => ast_to_node(source, node, cx),
        })
        .collect();
    ParsedDocument {
        source: source.to_string().into(),
        blocks: Arc::new(blocks),
    }
}

/// A top-level blockquote, which is a block surface when it opens with the
/// marker of a registered one.
fn blockquote_to_node(
    source: &str,
    mut quote: mdast::Blockquote,
    cx: &mut NodeContext,
) -> BlockNode {
    let Some(name) = take_surface_marker(source, &mut quote, cx) else {
        return ast_to_node(source, Node::Blockquote(quote), cx);
    };
    let children = quote
        .children
        .into_iter()
        .map(|child| ast_to_node(source, child, cx))
        .collect();
    BlockNode::Surface {
        name,
        children,
        span: new_span(quote.position, cx),
    }
}

/// Removes the marker line that names a registered block surface from the
/// start of `quote`, and returns that name.
///
/// As in a GitHub alert, the marker `[!name]` is alone on the first line of
/// the first paragraph, and the lines after it are that paragraph's text. The
/// name matches ASCII-case-insensitively.
fn take_surface_marker(
    source: &str,
    quote: &mut mdast::Blockquote,
    cx: &NodeContext,
) -> Option<SharedString> {
    let Some(Node::Paragraph(paragraph)) = quote.children.first_mut() else {
        return None;
    };
    let Some(Node::Text(text)) = paragraph.children.first_mut() else {
        return None;
    };
    let (line, rest) = match text.value.split_once('\n') {
        Some((line, rest)) => (line.strip_suffix('\r').unwrap_or(line), Some(rest)),
        None => (text.value.as_str(), None),
    };
    // An escaped `\[` or a character reference reads as the same text, but
    // writes a literal, so the marker must appear as is in the source. The
    // text of an escape starts after its backslash.
    let start = text.position.as_ref()?.start.offset;
    if source.get(..start)?.ends_with('\\') || !source.get(start..)?.starts_with(line) {
        return None;
    }
    let name = line.strip_prefix("[!")?.strip_suffix(']')?;
    let name = cx.markdown_extensions.block_surface_name(name)?;
    match rest {
        // The next line goes on in this text, which starts after the line's
        // blockquote marker in the source.
        Some(rest) => {
            let rest = rest.to_string();
            let position = text.position.as_mut()?;
            let line_start =
                source[position.start.offset..].find('\n')? + position.start.offset + 1;
            let content = source[line_start..].trim_start_matches([' ', '\t']);
            let content = content.strip_prefix('>').unwrap_or(content);
            let content = content.trim_start_matches([' ', '\t']);
            let offset = source.len() - content.len();
            position.start = Point::new(position.start.line + 1, offset - line_start + 1, offset);
            text.value = rest;
            if text.value.is_empty() {
                paragraph.children.remove(0);
            }
        }
        // The marker ends the paragraph or a hard break ends its line.
        None => match paragraph.children.get(1) {
            None => {
                paragraph.children.clear();
            }
            Some(Node::Break(_)) => {
                paragraph.children.drain(..2);
            }
            Some(_) => return None,
        },
    }
    // The paragraph starts where its first remaining child does.
    match paragraph.children.first() {
        Some(first) => {
            if let Some(start) = first.position().map(|first| first.start.clone())
                && let Some(position) = paragraph.position.as_mut()
            {
                position.start = start;
            }
        }
        None => {
            quote.children.remove(0);
        }
    }
    Some(name)
}

fn new_span(pos: Option<markdown::unist::Position>, cx: &NodeContext) -> Option<Span> {
    let pos = pos?;

    Some(Span {
        start: cx.offset + pos.start.offset,
        end: cx.offset + pos.end.offset,
    })
}

/// The `[ ]` / `[x]` marker of a task list item, which mdast does not keep.
///
/// The marker opens the item's first paragraph. Before it in the item come
/// only the list marker, whitespace, line prefixes and link reference
/// definitions, so it is the first `[` after the definitions.
fn task_marker_span(source: &str, item: &mdast::ListItem, cx: &NodeContext) -> Option<Span> {
    item.checked?;
    let mut from = item.position.as_ref()?.start.offset;
    for definition in item
        .children
        .iter()
        .take_while(|child| !matches!(child, Node::Paragraph(_)))
    {
        from = definition.position()?.end.offset;
    }
    let bytes = source.as_bytes();
    let open = from + bytes.get(from..)?.iter().position(|&byte| byte == b'[')?;
    let close = open + bytes[open..].iter().position(|&byte| byte == b']')?;
    Some(Span {
        start: cx.offset + open,
        end: cx.offset + close + 1,
    })
}

fn ast_to_node(source: &str, value: mdast::Node, cx: &mut NodeContext) -> BlockNode {
    let span = new_span(value.position().cloned(), cx);
    let parse_cx = MarkdownParseContext::new(source, cx.offset);
    if let Some(mut node) = cx.markdown_extensions.parse_block(&value, &parse_cx) {
        node.set_span(span);
        return BlockNode::Custom {
            node,
            state: Default::default(),
        };
    }

    match value {
        Node::Root(_) => unreachable!("node::Root should be handled separately"),
        Node::Paragraph(val) => {
            let mut paragraph = Paragraph::default();
            parse_inline_children(source, &mut paragraph, &val.children, cx);
            paragraph.span = new_span(val.position, cx);
            BlockNode::Paragraph(paragraph)
        }
        Node::Blockquote(val) => {
            let children = val
                .children
                .into_iter()
                .map(|c| ast_to_node(source, c, cx))
                .collect();
            BlockNode::Blockquote {
                children,
                span: new_span(val.position, cx),
            }
        }
        Node::List(list) => {
            let children = list
                .children
                .into_iter()
                .map(|c| ast_to_node(source, c, cx))
                .collect();
            BlockNode::List {
                ordered: list.ordered,
                start: list.start,
                children,
                span: new_span(list.position, cx),
            }
        }
        Node::ListItem(val) => {
            let task_marker = task_marker_span(source, &val, cx);
            let children = val
                .children
                .into_iter()
                .map(|c| ast_to_node(source, c, cx))
                .collect();
            BlockNode::ListItem {
                children,
                spread: val.spread,
                checked: val.checked,
                task_marker,
                span: new_span(val.position, cx),
            }
        }
        Node::Break(val) => BlockNode::Break {
            html: false,
            span: new_span(val.position, cx),
        },
        Node::Code(raw) => {
            let span = new_span(raw.position, cx);
            let segments = code_source_segments(source, &raw.value, span, cx.offset);
            BlockNode::CodeBlock(
                CodeBlock::new(raw.value.into(), raw.lang.map(Into::into), span)
                    .with_meta(raw.meta.map(Into::into))
                    .source_segments(segments),
            )
        }
        Node::Heading(val) => {
            let mut paragraph = Paragraph::default();
            parse_inline_children(source, &mut paragraph, &val.children, cx);

            BlockNode::Heading {
                level: val.depth,
                children: paragraph,
                span: new_span(val.position, cx),
            }
        }
        Node::Math(val) => {
            let span = new_span(val.position, cx);
            let segments = code_source_segments(source, &val.value, span, cx.offset);
            BlockNode::CodeBlock(
                CodeBlock::new(val.value.into(), None, span).source_segments(segments),
            )
        }
        Node::Html(val) => match super::html::parse(&val.value, cx) {
            Ok(el) => BlockNode::Root {
                children: Arc::unwrap_or_clone(el.blocks),
                span: new_span(val.position, cx),
            },
            Err(err) => {
                if cfg!(debug_assertions) {
                    tracing::warn!("error parsing html: {:#?}", err);
                }

                BlockNode::Paragraph(Paragraph::new(val.value))
            }
        },
        Node::MdxFlowExpression(val) => {
            let span = new_span(val.position, cx);
            let segments = code_source_segments(source, &val.value, span, cx.offset);
            BlockNode::CodeBlock(
                CodeBlock::new(val.value.into(), Some("mdx".into()), span)
                    .source_segments(segments),
            )
        }
        Node::Yaml(val) => BlockNode::CodeBlock(CodeBlock::new(
            val.value.into(),
            Some("yml".into()),
            new_span(val.position, cx),
        )),
        Node::Toml(val) => BlockNode::CodeBlock(CodeBlock::new(
            val.value.into(),
            Some("toml".into()),
            new_span(val.position, cx),
        )),
        Node::MdxJsxTextElement(val) => {
            let mut paragraph = Paragraph::default();
            parse_inline_children(source, &mut paragraph, &val.children, cx);
            paragraph.span = new_span(val.position, cx);
            BlockNode::Paragraph(paragraph)
        }
        Node::MdxJsxFlowElement(val) => {
            let mut paragraph = Paragraph::default();
            parse_inline_children(source, &mut paragraph, &val.children, cx);
            paragraph.span = new_span(val.position, cx);
            BlockNode::Paragraph(paragraph)
        }
        Node::ThematicBreak(val) => BlockNode::HorizontalRule {
            span: new_span(val.position, cx),
        },
        Node::Table(val) => {
            let mut table = Table::default();
            table.column_aligns = val
                .align
                .clone()
                .into_iter()
                .map(|align| align.into())
                .collect();
            val.children.iter().for_each(|c| {
                if let Node::TableRow(row) = c {
                    parse_table_row(source, &mut table, row, cx);
                }
            });
            table.span = new_span(val.position, cx);

            BlockNode::Table(table)
        }
        Node::FootnoteDefinition(def) => {
            let mut paragraph = Paragraph::default();
            let prefix = format!("[{}]: ", def.identifier);
            paragraph.push(InlineNode::new(&prefix).marks(vec![(
                0..prefix.len(),
                TextMark {
                    italic: true,
                    ..Default::default()
                },
            )]));

            parse_inline_children(source, &mut paragraph, &def.children, cx);
            paragraph.span = new_span(def.position, cx);
            BlockNode::Paragraph(paragraph)
        }
        Node::Definition(def) => BlockNode::Definition {
            identifier: def.identifier.clone().into(),
            url: def.url.clone().into(),
            title: def.title.clone().map(|s| s.into()),
            span: new_span(def.position, cx),
        },
        _ => {
            if cfg!(debug_assertions) {
                tracing::warn!("unsupported node: {:#?}", value);
            }
            BlockNode::Unknown
        }
    }
}
