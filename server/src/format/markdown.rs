//! Markdown and MDX: only prose (mdast `Text` nodes) and frontmatter values
//! are converted. Code, math, HTML, ESM, expressions, JSX attributes and URLs
//! are left untouched.

use std::ops::Range;

use markdown::mdast::{Node, ReferenceKind};
use markdown::{Constructs, ParseOptions};

use super::{FormatOptions, yaml};

#[derive(Debug, Clone, Copy)]
pub enum Flavor {
    Markdown,
    Mdx,
}

fn parse_options(flavor: Flavor) -> ParseOptions {
    let base = match flavor {
        Flavor::Markdown => Constructs::gfm(),
        Flavor::Mdx => Constructs {
            mdx_esm: true,
            mdx_expression_flow: true,
            mdx_expression_text: true,
            mdx_jsx_flow: true,
            mdx_jsx_text: true,
            ..Constructs::gfm()
        },
    };
    let constructs = match flavor {
        Flavor::Markdown => base,
        // MDX does not support raw HTML, indented code or `<url>` autolinks.
        Flavor::Mdx => Constructs {
            autolink: false,
            code_indented: false,
            html_flow: false,
            html_text: false,
            ..base
        },
    };

    ParseOptions {
        constructs: Constructs {
            frontmatter: true,
            math_flow: true,
            math_text: true,
            ..constructs
        },
        // ESM is only recognised when a parse function is given.
        mdx_esm_parse: matches!(flavor, Flavor::Mdx).then(|| Box::new(esm_signal) as Box<_>),
        ..ParseOptions::gfm()
    }
}

/// A lightweight stand-in for a JavaScript parser: an ESM block is complete
/// once brackets, strings, template literals and comments are closed.
/// Otherwise markdown-rs is asked to feed more lines (e.g. past blank lines
/// inside an exported function).
fn esm_signal(source: &str) -> markdown::MdxSignal {
    if esm_is_complete(source) {
        markdown::MdxSignal::Ok
    } else {
        markdown::MdxSignal::Eof(
            "unexpected end of ESM".to_string(),
            Box::new("tex-japanese-formatter".to_string()),
            Box::new("esm".to_string()),
        )
    }
}

fn esm_is_complete(source: &str) -> bool {
    // `{` opened inside a template literal (`${`) is marked with `$`.
    let mut stack: Vec<char> = Vec::new();
    let mut chars = source.chars().peekable();
    // The last non-whitespace character, used to tell regexes from division.
    let mut previous_significant = '\0';

    while let Some(c) = chars.next() {
        if stack.last() == Some(&'`') {
            match c {
                '\\' => {
                    chars.next();
                }
                '`' => {
                    stack.pop();
                }
                '$' if chars.peek() == Some(&'{') => {
                    chars.next();
                    stack.push('$');
                }
                _ => {}
            }
            continue;
        }

        let previous = previous_significant;
        if !c.is_whitespace() {
            previous_significant = c;
        }

        match c {
            // Strings cannot span lines. A quote that is not closed on its
            // line is not a string (e.g. `it's` in JSX text).
            '\'' | '"' => {
                if let Some(rest) = skip_literal(chars.clone(), c) {
                    chars = rest;
                }
            }
            // A regular expression literal, e.g. `/'/` or `/[{]/`. `<` is not
            // in the list because `</` is a JSX closing tag.
            '/' if chars.peek() != Some(&'/')
                && chars.peek() != Some(&'*')
                && (previous == '\0' || "(,=:[!&|?{};+-*%>~^".contains(previous)) =>
            {
                if let Some(rest) = skip_literal(chars.clone(), '/') {
                    chars = rest;
                }
            }
            '/' if chars.peek() == Some(&'/') => {
                for next in chars.by_ref() {
                    if next == '\n' {
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut closed = false;
                let mut previous = '\0';
                for next in chars.by_ref() {
                    if previous == '*' && next == '/' {
                        closed = true;
                        break;
                    }
                    previous = next;
                }
                if !closed {
                    return false;
                }
            }
            '`' | '(' | '[' | '{' => stack.push(c),
            ')' | ']' | '}' => {
                let expected = match c {
                    ')' => '(',
                    ']' => '[',
                    _ => '{',
                };
                match stack.last() {
                    Some(&open) if open == expected || (c == '}' && open == '$') => {
                        stack.pop();
                    }
                    // Stray closing bracket: let the block end here.
                    _ => return true,
                }
            }
            _ => {}
        }
    }

    stack.is_empty()
}

/// Skips a string or regular expression literal whose opening `delimiter`
/// has been consumed. Returns `None` if it is not closed on the same line.
fn skip_literal(
    mut chars: std::iter::Peekable<std::str::Chars<'_>>,
    delimiter: char,
) -> Option<std::iter::Peekable<std::str::Chars<'_>>> {
    let mut in_class = false;
    while let Some(next) = chars.next() {
        match next {
            '\\' => {
                chars.next();
            }
            '\n' => return None,
            '[' if delimiter == '/' => in_class = true,
            ']' if delimiter == '/' => in_class = false,
            _ if next == delimiter && !in_class => return Some(chars),
            _ => {}
        }
    }
    None
}

pub fn convertible_ranges(
    text: &str,
    flavor: Flavor,
    options: &FormatOptions,
) -> Result<Vec<Range<usize>>, String> {
    let tree = markdown::to_mdast(text, &parse_options(flavor))
        .map_err(|message| format!("failed to parse document: {message}"))?;

    let mut ranges = Vec::new();
    collect(&tree, text, options, &mut ranges);
    Ok(ranges)
}

fn byte_range(node: &Node) -> Option<Range<usize>> {
    node.position()
        .map(|position| position.start.offset..position.end.offset)
}

/// `<https://...>` and GFM literal autolinks show the URL itself as text.
/// Unlike `[text](url)`, their source does not start with `[`.
fn is_autolink(node: &Node, text: &str) -> bool {
    byte_range(node).is_some_and(|range| !text[range].starts_with('['))
}

fn collect(node: &Node, text: &str, options: &FormatOptions, ranges: &mut Vec<Range<usize>>) {
    match node {
        Node::Text(_) => ranges.extend(byte_range(node)),
        Node::Yaml(_) => {
            if let Some(range) = byte_range(node) {
                ranges.extend(frontmatter_ranges(text, range, options));
            }
        }
        Node::Link(_) if is_autolink(node, text) => {}
        // In `[label]` and `[label][]` the text is also the label that must
        // match the definition, so converting it would break the link.
        Node::LinkReference(reference)
            if matches!(
                reference.reference_kind,
                ReferenceKind::Shortcut | ReferenceKind::Collapsed
            ) => {}
        // Leaves that are not prose.
        Node::Code(_)
        | Node::InlineCode(_)
        | Node::Math(_)
        | Node::InlineMath(_)
        | Node::Html(_)
        | Node::MdxjsEsm(_)
        | Node::MdxFlowExpression(_)
        | Node::MdxTextExpression(_)
        | Node::Toml(_)
        | Node::Image(_)
        | Node::ImageReference(_)
        | Node::Definition(_)
        | Node::FootnoteReference(_)
        | Node::Break(_)
        | Node::ThematicBreak(_) => {}
        _ => {
            if let Some(children) = node.children() {
                for child in children {
                    collect(child, text, options, ranges);
                }
            }
        }
    }
}

/// `range` covers the whole frontmatter including both `---` fences.
fn frontmatter_ranges(
    text: &str,
    range: Range<usize>,
    options: &FormatOptions,
) -> Vec<Range<usize>> {
    let block = &text[range.clone()];
    let Some(first_newline) = block.find('\n') else {
        return Vec::new();
    };
    let body_start = range.start + first_newline + 1;
    let body_end = text[..range.end]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(range.end)
        .max(body_start);

    yaml::value_ranges(
        &text[body_start..body_end],
        options.frontmatter_exclude_keys,
    )
    .into_iter()
    .map(|value| value.start + body_start..value.end + body_start)
    .collect()
}

#[cfg(test)]
mod tests {
    use crate::format::format;

    fn md(text: &str) -> String {
        format("markdown", text, &[]).unwrap()
    }

    fn mdx(text: &str) -> String {
        format("mdx", text, &[]).unwrap()
    }

    #[test]
    fn converts_prose() {
        let input = "# 見出し、です。\n\n本文、*強調、*と**太字。**\n\n- 項目、1\n> 引用、です。\n";
        let expected =
            "# 見出し，です．\n\n本文，*強調，*と**太字．**\n\n- 項目，1\n> 引用，です．\n";
        assert_eq!(md(input), expected);
        assert_eq!(mdx(input), expected);
    }

    #[test]
    fn keeps_code() {
        let input = "本文、`コード、`です。\n\n```js\nconst s = \"、。\";\n```\n\n~~~\n、\n~~~\n";
        let expected =
            "本文，`コード、`です．\n\n```js\nconst s = \"、。\";\n```\n\n~~~\n、\n~~~\n";
        assert_eq!(md(input), expected);
        assert_eq!(mdx(input), expected);
    }

    #[test]
    fn keeps_indented_code_in_markdown() {
        let input = "本文、です。\n\n    コード、です。\n";
        assert_eq!(md(input), "本文，です．\n\n    コード、です。\n");
    }

    #[test]
    fn keeps_math() {
        let input = "式$a、b$と、\n\n$$\nf(x)、g(x)\n$$\n";
        let expected = "式$a、b$と，\n\n$$\nf(x)、g(x)\n$$\n";
        assert_eq!(md(input), expected);
        assert_eq!(mdx(input), expected);
    }

    #[test]
    fn links_convert_text_but_not_urls() {
        let input = "[リンク、です](https://例え.jp/a、b \"題、\")と<https://example.com/、>と https://example.com/、x。\n";
        // A GFM literal autolink runs up to whitespace, so `、x。` is part of the URL.
        let expected = "[リンク，です](https://例え.jp/a、b \"題、\")と<https://example.com/、>と https://example.com/、x。\n";
        assert_eq!(md(input), expected);
    }

    #[test]
    fn keeps_image_alt_and_definitions() {
        let input = "![図、1](a.png)\n\n[参照、]: https://example.com/、\n\n本文、[参照、]です。\n";
        let expected =
            "![図、1](a.png)\n\n[参照、]: https://example.com/、\n\n本文，[参照、]です．\n";
        assert_eq!(md(input), expected);
    }

    #[test]
    fn reference_labels_are_untouched() {
        // Shortcut and collapsed references use their text as the label.
        let input = "[参照、]と[参照、][]と[文字、][参照、]。\n\n[参照、]: https://e.com\n";
        // Only the text of a full reference is converted.
        let expected = "[参照、]と[参照、][]と[文字，][参照、]．\n\n[参照、]: https://e.com\n";
        assert_eq!(md(input), expected);
        assert_eq!(mdx(input), expected);
    }

    #[test]
    fn keeps_html_in_markdown() {
        let input = "<div title=\"、\">\n、\n</div>\n\n本文<span>、</span>。\n";
        let expected = "<div title=\"、\">\n、\n</div>\n\n本文<span>，</span>．\n";
        assert_eq!(md(input), expected);
    }

    #[test]
    fn tables_and_footnotes() {
        let input = "| 見出し、 | b |\n| --- | --- |\n| セル、 | `、` |\n\n注釈[^1]。\n\n[^1]: 脚注、です。\n";
        let expected = "| 見出し， | b |\n| --- | --- |\n| セル， | `、` |\n\n注釈[^1]．\n\n[^1]: 脚注，です．\n";
        assert_eq!(md(input), expected);
    }

    #[test]
    fn mdx_keeps_esm_expressions_and_attributes() {
        let input = concat!(
            "import { Callout } from './callout、.js'\n",
            "export const meta = { title: '題、' }\n",
            "\n",
            "本文、{'式、'}です。\n",
            "\n",
            "{/* コメント、 */}\n",
            "\n",
            "<Callout title=\"概要、\" note={'、'}>\n",
            "  中身、です。\n",
            "</Callout>\n",
            "\n",
            "文中<Tag value=\"、\">タグ、</Tag>です。\n",
        );
        let expected = concat!(
            "import { Callout } from './callout、.js'\n",
            "export const meta = { title: '題、' }\n",
            "\n",
            "本文，{'式、'}です．\n",
            "\n",
            "{/* コメント、 */}\n",
            "\n",
            "<Callout title=\"概要、\" note={'、'}>\n",
            "  中身，です．\n",
            "</Callout>\n",
            "\n",
            "文中<Tag value=\"、\">タグ，</Tag>です．\n",
        );
        assert_eq!(mdx(input), expected);
    }

    #[test]
    fn mdx_esm_spanning_blank_lines() {
        let input = concat!(
            "export function f() {\n",
            "  const a = '、'\n",
            "\n",
            "  return `${a}。`\n",
            "}\n",
            "\n",
            "本文、です。\n",
            "\n",
            "import x from \"、\" // 注、\n",
            "\n",
            "後半、\n",
        );
        let expected = concat!(
            "export function f() {\n",
            "  const a = '、'\n",
            "\n",
            "  return `${a}。`\n",
            "}\n",
            "\n",
            "本文，です．\n",
            "\n",
            "import x from \"、\" // 注、\n",
            "\n",
            "後半，\n",
        );
        assert_eq!(mdx(input), expected);
    }

    #[test]
    fn markdown_treats_import_lines_as_prose() {
        assert_eq!(md("import は、です。\n"), "import は，です．\n");
    }

    #[test]
    fn esm_completeness() {
        use super::esm_is_complete;
        assert!(esm_is_complete("import a from 'b'"));
        assert!(esm_is_complete("export const a = { b: [1, (2)] }"));
        assert!(esm_is_complete("export const s = `x${ {a: 1}.a }y`"));
        assert!(esm_is_complete("export const s = '}' // {\n"));
        assert!(esm_is_complete("export const s = /* { */ 1"));
        assert!(!esm_is_complete("export function f() {\n"));
        assert!(!esm_is_complete("export const s = `abc"));
        assert!(!esm_is_complete("export const s = /* abc"));
        // A quote that is not closed on its line is not a string.
        assert!(esm_is_complete("export const s = 'abc"));
        assert!(esm_is_complete("export const a = <div>it's</div>"));
        // Regular expression literals.
        assert!(esm_is_complete("export const re = /'/"));
        assert!(esm_is_complete("export const re = /`/g"));
        assert!(esm_is_complete("export const re = /[{(]/"));
        assert!(esm_is_complete("export const re = /\\//"));
        assert!(!esm_is_complete(
            "export const f = (a) => a.match(/[}]/) && {\n"
        ));
        // Division is not a regular expression.
        assert!(esm_is_complete("export const a = (4) / 2 / (1)"));
        assert!(esm_is_complete("export const a = b / 2, c = '/'"));
    }

    #[test]
    fn mdx_esm_with_regex_and_jsx_text() {
        let input = "export const re = /'/\n\nexport const a = <div>it's</div>\n\n本文、です。\n";
        let expected =
            "export const re = /'/\n\nexport const a = <div>it's</div>\n\n本文，です．\n";
        assert_eq!(mdx(input), expected);
    }

    #[test]
    fn mdx_esm_with_jsx_closing_tags() {
        let esm = concat!(
            "export const List = ({items}) => (\n",
            "  <ul>{items.map((item) => <li key={item}>{item}</li>)}</ul>\n",
            ")\n",
            "\n",
            "export const Nums = () => <div>{[1, 2].map(n => <span>{n}</span>)}</div>\n",
            "\n",
        );
        let input = format!("{esm}本文、です。\n");
        let expected = format!("{esm}本文，です．\n");
        assert_eq!(mdx(&input), expected);
    }

    #[test]
    fn link_text_equal_to_url_is_converted() {
        assert_eq!(md("[a、b](a、b)\n"), "[a，b](a、b)\n");
    }

    #[test]
    fn mdx_does_not_treat_indentation_as_code() {
        assert_eq!(
            mdx("本文、\n\n    字下げ、です。\n"),
            "本文，\n\n    字下げ，です．\n"
        );
    }

    #[test]
    fn mdx_parse_error_is_reported() {
        assert!(format("mdx", "<Callout>\n本文、\n", &[]).is_err());
    }

    #[test]
    fn frontmatter_values_are_converted() {
        let input = "---\ntitle: 研究、概要。\ntags: [機械学習、統計]\n---\n\n本文、です。\n";
        let expected = "---\ntitle: 研究，概要．\ntags: [機械学習，統計]\n---\n\n本文，です．\n";
        assert_eq!(md(input), expected);
        assert_eq!(mdx(input), expected);
    }

    #[test]
    fn frontmatter_excluded_keys() {
        let input = "---\ntitle: 題、\nslug: 題、\n---\n本文、\n";
        let output = format("mdx", input, &["slug"]).unwrap();
        assert_eq!(output, "---\ntitle: 題，\nslug: 題、\n---\n本文，\n");
    }

    #[test]
    fn crlf_line_endings() {
        let input = "---\r\ntitle: 題、\r\n---\r\n\r\n本文、`、`です。\r\n";
        let expected = "---\r\ntitle: 題，\r\n---\r\n\r\n本文，`、`です．\r\n";
        assert_eq!(md(input), expected);
        assert_eq!(mdx(input), expected);
    }

    #[test]
    fn toml_frontmatter_is_left_alone() {
        let input = "+++\ntitle = \"題、\"\n+++\n\n本文、\n";
        assert_eq!(md(input), "+++\ntitle = \"題、\"\n+++\n\n本文，\n");
    }
}
