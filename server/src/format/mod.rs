#![allow(clippy::single_range_in_vec_init)]

mod markdown;
mod yaml;

use std::ops::Range;

use tower_lsp::lsp_types::{Position, TextEdit};

pub struct FormatOptions<'a> {
    pub frontmatter_exclude_keys: &'a [String],
}

fn replacement(c: char) -> Option<&'static str> {
    match c {
        '、' => Some("，"),
        '。' => Some("．"),
        _ => None,
    }
}

/// Returns the byte ranges of `text` in which punctuation may be converted.
/// The ranges are sorted and do not overlap.
pub fn convertible_ranges(
    language_id: &str,
    text: &str,
    options: &FormatOptions,
) -> Result<Vec<Range<usize>>, String> {
    let mut ranges = match language_id {
        "markdown" => markdown::convertible_ranges(text, markdown::Flavor::Markdown, options)?,
        "mdx" => markdown::convertible_ranges(text, markdown::Flavor::Mdx, options)?,
        _ => vec![0..text.len()],
    };
    ranges.sort_by_key(|range| range.start);
    Ok(ranges)
}

/// Builds one edit per converted character, with UTF-16 based positions.
pub fn text_edits(text: &str, ranges: &[Range<usize>]) -> Vec<TextEdit> {
    let mut edits = Vec::new();
    let mut ranges = ranges.iter().peekable();
    let mut line: u32 = 0;
    let mut character: u32 = 0;
    let mut previous = '\0';

    for (offset, c) in text.char_indices() {
        while ranges.peek().is_some_and(|range| range.end <= offset) {
            ranges.next();
        }
        let in_range = ranges.peek().is_some_and(|range| range.start <= offset);

        if in_range && let Some(new_text) = replacement(c) {
            edits.push(TextEdit {
                range: tower_lsp::lsp_types::Range::new(
                    Position::new(line, character),
                    Position::new(line, character + 1),
                ),
                new_text: new_text.to_string(),
            });
        }

        // LSP treats `\n`, `\r\n` and a lone `\r` as line endings.
        match c {
            '\n' if previous == '\r' => {}
            '\n' | '\r' => {
                line += 1;
                character = 0;
            }
            _ => character += c.len_utf16() as u32,
        }
        previous = c;
    }

    edits
}

/// Applies the conversion to `text` and returns the result.
#[cfg(test)]
pub fn apply(text: &str, ranges: &[Range<usize>]) -> String {
    let mut output = String::with_capacity(text.len());
    let mut ranges = ranges.iter().peekable();

    for (offset, c) in text.char_indices() {
        while ranges.peek().is_some_and(|range| range.end <= offset) {
            ranges.next();
        }
        let in_range = ranges.peek().is_some_and(|range| range.start <= offset);

        match replacement(c) {
            Some(new_text) if in_range => output.push_str(new_text),
            _ => output.push(c),
        }
    }

    output
}

#[cfg(test)]
pub fn format(language_id: &str, text: &str, exclude_keys: &[&str]) -> Result<String, String> {
    let keys: Vec<String> = exclude_keys.iter().map(|key| key.to_string()).collect();
    let options = FormatOptions {
        frontmatter_exclude_keys: &keys,
    };
    let ranges = convertible_ranges(language_id, text, &options)?;
    Ok(apply(text, &ranges))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whole_text_languages_convert_everything() {
        let text = "これは、テストです。\n\\verb|、|";
        assert_eq!(
            format("latex", text, &[]).unwrap(),
            "これは，テストです．\n\\verb|，|"
        );
        assert_eq!(
            format("plaintext", text, &[]).unwrap(),
            "これは，テストです．\n\\verb|，|"
        );
    }

    #[test]
    fn edits_use_utf16_positions() {
        let text = "😀、\nあ。";
        let edits = text_edits(text, &[0..text.len()]);
        assert_eq!(edits.len(), 2);
        assert_eq!(edits[0].range.start, Position::new(0, 2));
        assert_eq!(edits[0].range.end, Position::new(0, 3));
        assert_eq!(edits[0].new_text, "，");
        assert_eq!(edits[1].range.start, Position::new(1, 1));
        assert_eq!(edits[1].new_text, "．");
    }

    #[test]
    fn edits_respect_ranges() {
        let text = "、、、";
        // Only the middle character (bytes 3..6).
        let edits = text_edits(text, &[3..6]);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].range.start, Position::new(0, 1));
        assert_eq!(apply(text, &[3..6]), "、，、");
    }

    #[test]
    fn edits_handle_all_line_endings() {
        let text = "、\r、\r\n、\n、";
        let starts: Vec<Position> = text_edits(text, &[0..text.len()])
            .into_iter()
            .map(|edit| edit.range.start)
            .collect();
        assert_eq!(
            starts,
            vec![
                Position::new(0, 0),
                Position::new(1, 0),
                Position::new(2, 0),
                Position::new(3, 0),
            ]
        );
    }

    #[test]
    fn no_edits_for_already_formatted_text() {
        let text = "これは，テストです．";
        assert!(text_edits(text, &[0..text.len()]).is_empty());
    }
}
