//! YAML frontmatter: values are converted, keys are not. Values under
//! excluded top-level keys are left untouched.

use std::ops::Range;

use saphyr_parser::{Event, Parser, ScalarStyle};

struct Container {
    is_mapping: bool,
    /// For mappings: whether the next node is a key.
    expect_key: bool,
    /// Whether this container itself is used as a mapping key.
    is_key: bool,
    /// Whether values inside this container may be converted.
    convert: bool,
    /// For the top-level mapping: the key whose value is being read.
    current_key: Option<String>,
}

/// Returns the byte ranges of scalar values in `source`.
/// If the YAML cannot be parsed, nothing is converted.
pub fn value_ranges(source: &str, exclude_keys: &[String]) -> Vec<Range<usize>> {
    let char_to_byte: Vec<usize> = source
        .char_indices()
        .map(|(offset, _)| offset)
        .chain(std::iter::once(source.len()))
        .collect();
    let to_byte = |index: usize| char_to_byte.get(index).copied().unwrap_or(source.len());

    let mut ranges = Vec::new();
    let mut stack: Vec<Container> = Vec::new();

    for result in Parser::new_from_str(source) {
        let Ok((event, span)) = result else {
            return Vec::new();
        };

        match event {
            Event::Scalar(value, style, ..) => {
                let (is_key, convert) = node_role(&stack, exclude_keys);
                if is_key {
                    if stack.len() == 1 {
                        stack[0].current_key = Some(value.into_owned());
                    }
                } else if convert {
                    let start = to_byte(span.start.index());
                    let end = to_byte(span.end.index());
                    let start = scalar_content_start(source, start, end, style);
                    let end = quoted_scalar_end(source, start, end, style);
                    ranges.push(start..end);
                }
                finish_node(&mut stack, is_key);
            }
            Event::Alias(_) => {
                let (is_key, _) = node_role(&stack, exclude_keys);
                if is_key {
                    forget_top_level_key(&mut stack);
                }
                finish_node(&mut stack, is_key);
            }
            Event::MappingStart(..) | Event::SequenceStart(..) => {
                let (is_key, convert) = node_role(&stack, exclude_keys);
                if is_key {
                    forget_top_level_key(&mut stack);
                }
                stack.push(Container {
                    is_mapping: matches!(event, Event::MappingStart(..)),
                    expect_key: true,
                    is_key,
                    convert: convert && !is_key,
                    current_key: None,
                });
            }
            Event::MappingEnd | Event::SequenceEnd => {
                if let Some(container) = stack.pop() {
                    finish_node(&mut stack, container.is_key);
                }
            }
            _ => {}
        }
    }

    ranges
}

/// The span of a scalar may include its anchor (`&name`), its tag (`!tag`)
/// and, for block scalars, the header line (`|` or `>` with a comment).
/// Returns where the actual content begins.
fn scalar_content_start(source: &str, start: usize, end: usize, style: ScalarStyle) -> usize {
    let mut position = start;
    loop {
        let rest = &source[position..end];
        if !rest.starts_with(['&', '!']) {
            break;
        }
        let token_end = rest.find(char::is_whitespace).unwrap_or(rest.len());
        let after = &rest[token_end..];
        let content = after.trim_start_matches([' ', '\t', '\r', '\n']);
        position += token_end + (after.len() - content.len());
    }

    if matches!(style, ScalarStyle::Literal | ScalarStyle::Folded) {
        if let Some(newline) = source[position..end].find('\n') {
            position += newline + 1;
        } else {
            position = end;
        }
    }
    position
}

/// The span of a quoted scalar may extend to a comment on the same line
/// (`"x" # comment`). Returns the position just after the closing quote.
fn quoted_scalar_end(source: &str, start: usize, end: usize, style: ScalarStyle) -> usize {
    let quote = match style {
        ScalarStyle::SingleQuoted => '\'',
        ScalarStyle::DoubleQuoted => '"',
        _ => return end,
    };
    let text = &source[start..end];
    let mut chars = text.char_indices();
    if chars.next().map(|(_, c)| c) != Some(quote) {
        return end;
    }
    while let Some((index, c)) = chars.next() {
        // `\"` escapes in double quotes, `''` in single quotes.
        if quote == '"' && c == '\\' {
            chars.next();
        } else if c == quote {
            if quote == '\'' && text[index + 1..].starts_with('\'') {
                chars.next();
            } else {
                return start + index + 1;
            }
        }
    }
    end
}

/// Determines whether the next node is a mapping key and whether its
/// contents may be converted.
fn node_role(stack: &[Container], exclude_keys: &[String]) -> (bool, bool) {
    let Some(parent) = stack.last() else {
        return (false, true);
    };
    if parent.is_mapping && parent.expect_key {
        return (true, false);
    }
    if !parent.convert {
        return (false, false);
    }
    let excluded = stack.len() == 1
        && parent.is_mapping
        && parent
            .current_key
            .as_ref()
            .is_some_and(|key| exclude_keys.iter().any(|excluded| excluded == key));
    (false, !excluded)
}

/// A top-level key that is not a scalar (`? [k]`, `*alias`) has no name to
/// match against the excluded keys, so the previous key name is dropped.
fn forget_top_level_key(stack: &mut [Container]) {
    if let [top_level] = stack {
        top_level.current_key = None;
    }
}

fn finish_node(stack: &mut [Container], was_key: bool) {
    if let Some(parent) = stack.last_mut()
        && parent.is_mapping
    {
        parent.expect_key = !was_key;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn convert(source: &str, exclude_keys: &[&str]) -> String {
        let keys: Vec<String> = exclude_keys.iter().map(|key| key.to_string()).collect();
        let ranges = value_ranges(source, &keys);
        crate::format::apply(source, &ranges)
    }

    #[test]
    fn converts_values_not_keys() {
        assert_eq!(convert("題、: 値、\n", &[]), "題、: 値，\n");
    }

    #[test]
    fn quoted_and_multiline_values() {
        let source = "a: \"引用、\"\nb: '単一、'\nc: |\n  一行目、\n  二行目。\nd: >-\n  折り返し、\ne: 複数行の、\n  平文。\n";
        let expected = "a: \"引用，\"\nb: '単一，'\nc: |\n  一行目，\n  二行目．\nd: >-\n  折り返し，\ne: 複数行の，\n  平文．\n";
        assert_eq!(convert(source, &[]), expected);
    }

    #[test]
    fn sequences_and_nested_mappings() {
        let source = "tags:\n  - 一、\n  - 二。\nflow: [三、, 四。]\nseo:\n  鍵、: 値、\n  list: [{内、: 外、}]\n";
        let expected = "tags:\n  - 一，\n  - 二．\nflow: [三，, 四．]\nseo:\n  鍵、: 値，\n  list: [{内、: 外，}]\n";
        assert_eq!(convert(source, &[]), expected);
    }

    #[test]
    fn comments_are_untouched() {
        assert_eq!(
            convert("# 注、\na: 値、 # 注、\n", &[]),
            "# 注、\na: 値， # 注、\n"
        );
    }

    #[test]
    fn comments_after_quoted_values_are_untouched() {
        let source = concat!(
            "title: \"論文\" # 仮題、後で直す\n",
            "a: 'x、' # c、\n",
            "b: \"x、\\\"y、\\\"\" # \"c、\"\n",
            "c: 'it''s、' # d、\n",
            "list:\n  - \"x、\" # e、\n",
        );
        let expected = concat!(
            "title: \"論文\" # 仮題、後で直す\n",
            "a: 'x，' # c、\n",
            "b: \"x，\\\"y，\\\"\" # \"c、\"\n",
            "c: 'it''s，' # d、\n",
            "list:\n  - \"x，\" # e、\n",
        );
        assert_eq!(convert(source, &[]), expected);
    }

    #[test]
    fn block_scalar_header_comments_are_untouched() {
        let source = "a: |  # 注、\n  本、\nb: >- # 注。\n  本。\nc: |\n  本、\n";
        let expected = "a: |  # 注、\n  本，\nb: >- # 注。\n  本．\nc: |\n  本，\n";
        assert_eq!(convert(source, &[]), expected);
    }

    #[test]
    fn anchors_and_tags_are_untouched() {
        let source = "a: &錨、 値、\nb: *錨、\nc: !!str 値、\nd: &x !t 値、\ne: &y | # 注、\n  本、\nf: &z\n  値、\n";
        let expected = "a: &錨、 値，\nb: *錨、\nc: !!str 値，\nd: &x !t 値，\ne: &y | # 注、\n  本，\nf: &z\n  値，\n";
        assert_eq!(convert(source, &[]), expected);
    }

    #[test]
    fn excluded_top_level_keys() {
        let source = "slug: 題、\ntags: [一、]\nseo:\n  slug: 題、\ntitle: 題、\n";
        let expected = "slug: 題、\ntags: [一、]\nseo:\n  slug: 題，\ntitle: 題，\n";
        assert_eq!(convert(source, &["slug", "tags"]), expected);
    }

    #[test]
    fn excluded_key_with_nested_value() {
        let source = "seo:\n  title: 題、\nother: 題、\n";
        assert_eq!(
            convert(source, &["seo"]),
            "seo:\n  title: 題、\nother: 題，\n"
        );
    }

    #[test]
    fn complex_keys_are_not_converted() {
        let source = "? [鍵、]\n: 値、\n";
        assert_eq!(convert(source, &[]), "? [鍵、]\n: 値，\n");
    }

    #[test]
    fn non_scalar_key_after_excluded_key() {
        // The value after a complex or alias key does not inherit the
        // exclusion of the previous key.
        let source = "slug: a、\n? [k]\n: 値、\n";
        assert_eq!(convert(source, &["slug"]), "slug: a、\n? [k]\n: 値，\n");
        let source = "x: &k 鍵\nslug: a、\n*k : 値、\n";
        assert_eq!(
            convert(source, &["slug"]),
            "x: &k 鍵\nslug: a、\n*k : 値，\n"
        );
    }

    #[test]
    fn invalid_yaml_is_untouched() {
        assert_eq!(convert("a: [未閉じ、\n", &[]), "a: [未閉じ、\n");
    }

    #[test]
    fn empty_source() {
        assert_eq!(convert("", &[]), "");
    }
}
