//! End-to-end tests that drive the language server binary over stdio.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{Value, json};

struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Client {
    fn start(initialization_options: Value) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_tex-japanese-formatter-lsp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to start server");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut client = Self {
            child,
            stdin,
            stdout,
            next_id: 1,
        };

        client.request(
            "initialize",
            json!({
                "processId": null,
                "rootUri": null,
                "capabilities": {},
                "initializationOptions": initialization_options,
            }),
        );
        client.notify("initialized", json!({}));
        client
    }

    fn send(&mut self, message: Value) {
        let body = message.to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{}", body.len(), body).unwrap();
        self.stdin.flush().unwrap();
    }

    fn receive(&mut self) -> Value {
        let mut length = 0;
        loop {
            let mut line = String::new();
            self.stdout.read_line(&mut line).unwrap();
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some(value) = line.strip_prefix("Content-Length: ") {
                length = value.parse().unwrap();
            }
        }
        let mut body = vec![0; length];
        self.stdout.read_exact(&mut body).unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({ "jsonrpc": "2.0", "method": method, "params": params }));
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        loop {
            let message = self.receive();
            if message.get("id") == Some(&json!(id)) {
                assert!(message.get("error").is_none(), "error: {message}");
                return message["result"].clone();
            }
        }
    }

    fn open(&mut self, uri: &str, language_id: &str, text: &str) {
        self.notify(
            "textDocument/didOpen",
            json!({
                "textDocument": { "uri": uri, "languageId": language_id, "version": 1, "text": text }
            }),
        );
    }

    fn change(&mut self, uri: &str, version: i32, text: &str) {
        self.notify(
            "textDocument/didChange",
            json!({
                "textDocument": { "uri": uri, "version": version },
                "contentChanges": [{ "text": text }]
            }),
        );
    }

    fn format(&mut self, uri: &str) -> Vec<Value> {
        let result = self.request(
            "textDocument/formatting",
            json!({
                "textDocument": { "uri": uri },
                "options": { "tabSize": 4, "insertSpaces": true }
            }),
        );
        result.as_array().cloned().unwrap_or_default()
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

/// Applies LSP edits (UTF-16 positions, lines separated by `\n`).
/// Runs in linear time so that it also works for very long documents.
fn apply(text: &str, edits: &[Value]) -> String {
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(index, _)| index + 1))
        .collect();
    let offset = |position: &Value| -> usize {
        let line = position["line"].as_u64().unwrap() as usize;
        let character = position["character"].as_u64().unwrap() as usize;
        let start = line_starts[line];
        let mut units = 0;
        for (index, c) in text[start..].char_indices() {
            if units == character || c == '\n' {
                return start + index;
            }
            units += c.len_utf16();
        }
        text.len()
    };

    let mut spans: Vec<(usize, usize, &str)> = edits
        .iter()
        .map(|edit| {
            let range = &edit["range"];
            (
                offset(&range["start"]),
                offset(&range["end"]),
                edit["newText"].as_str().unwrap(),
            )
        })
        .collect();
    spans.sort_by_key(|span| span.0);

    let mut output = String::with_capacity(text.len());
    let mut cursor = 0;
    for (start, end, new_text) in spans {
        assert!(start >= cursor, "edits overlap");
        output.push_str(&text[cursor..start]);
        output.push_str(new_text);
        cursor = end;
    }
    output.push_str(&text[cursor..]);
    output
}

const MDX: &str = "---\ntitle: 研究、概要。\nslug: 研究、概要\n---\n\nimport { Note } from './note、.js'\n\n# 見出し、です。\n\n<Note title=\"注意、\">\n  中身、`コード、`です。\n</Note>\n\n```js\nconst s = '、。'\n```\n";
const MDX_FORMATTED: &str = "---\ntitle: 研究，概要．\nslug: 研究、概要\n---\n\nimport { Note } from './note、.js'\n\n# 見出し，です．\n\n<Note title=\"注意、\">\n  中身，`コード、`です．\n</Note>\n\n```js\nconst s = '、。'\n```\n";

#[test]
fn formats_mdx_document() {
    let mut client = Client::start(json!({
        "targetLanguages": ["mdx"],
        "frontmatterExcludeKeys": ["slug"]
    }));
    client.open("file:///doc.mdx", "mdx", MDX);
    let edits = client.format("file:///doc.mdx");
    assert_eq!(apply(MDX, &edits), MDX_FORMATTED);

    // Formatting the result again is a no-op.
    client.change("file:///doc.mdx", 2, MDX_FORMATTED);
    assert!(client.format("file:///doc.mdx").is_empty());
}

#[test]
fn default_targets_only_latex() {
    let mut client = Client::start(json!({}));
    client.open("file:///doc.mdx", "mdx", "本文、です。\n");
    assert!(client.format("file:///doc.mdx").is_empty());

    let tex = "\\section{節}\n本文、です。\n";
    client.open("file:///doc.tex", "latex", tex);
    let edits = client.format("file:///doc.tex");
    assert_eq!(apply(tex, &edits), "\\section{節}\n本文，です．\n");
}

#[test]
fn configuration_changes_are_applied() {
    let mut client = Client::start(json!({ "targetLanguages": ["markdown"] }));
    let text = "本文、です。\n";
    client.open("file:///doc.md", "markdown", text);
    assert_eq!(client.format("file:///doc.md").len(), 2);

    client.notify(
        "workspace/didChangeConfiguration",
        json!({ "settings": { "enabled": false, "targetLanguages": ["markdown"] } }),
    );
    assert!(client.format("file:///doc.md").is_empty());

    client.notify(
        "workspace/didChangeConfiguration",
        json!({ "settings": { "targetLanguages": ["markdown"] } }),
    );
    assert_eq!(client.format("file:///doc.md").len(), 2);
}

#[test]
fn invalid_mdx_is_left_untouched() {
    let mut client = Client::start(json!({ "targetLanguages": ["mdx"] }));
    client.open("file:///broken.mdx", "mdx", "<Note>\n本文、\n");
    assert!(client.format("file:///broken.mdx").is_empty());
}

#[test]
fn formats_markdown_document() {
    let text = concat!(
        "---\ntitle: 研究、概要。\n---\n\n",
        "# 見出し、です。\n\n",
        "本文、`コード、`と[リンク、](https://example.com/、)です。\n\n",
        "<div>\n、\n</div>\n\n",
        "```\nコード、。\n```\n",
    );
    let expected = concat!(
        "---\ntitle: 研究，概要．\n---\n\n",
        "# 見出し，です．\n\n",
        "本文，`コード、`と[リンク，](https://example.com/、)です．\n\n",
        "<div>\n、\n</div>\n\n",
        "```\nコード、。\n```\n",
    );
    let mut client = Client::start(json!({ "targetLanguages": ["markdown"] }));
    client.open("file:///doc.md", "markdown", text);
    let edits = client.format("file:///doc.md");
    assert_eq!(apply(text, &edits), expected);
}

#[test]
fn formats_plaintext_document() {
    // Plain text has no syntax, so everything is converted.
    let text = "本文、です。\n```\nコード、。\n```\n😀、𠮷。\n";
    let expected = "本文，です．\n```\nコード，．\n```\n😀，𠮷．\n";
    let mut client = Client::start(json!({ "targetLanguages": ["plaintext"] }));
    client.open("file:///doc.txt", "plaintext", text);
    let edits = client.format("file:///doc.txt");
    assert_eq!(apply(text, &edits), expected);
}

/// Formats `text`, checks the result against `expected`, then checks that
/// formatting the result again changes nothing.
fn assert_formats_long_document(language_id: &str, uri: &str, text: &str, expected: &str) {
    let mut client = Client::start(json!({ "targetLanguages": [language_id] }));
    client.open(uri, language_id, text);

    let started = std::time::Instant::now();
    let edits = client.format(uri);
    eprintln!(
        "{language_id}: {} bytes, {} edits, {:?}",
        text.len(),
        edits.len(),
        started.elapsed()
    );
    let result = apply(text, &edits);
    assert!(result == expected, "long {language_id} document mismatch");

    client.change(uri, 2, &result);
    assert!(client.format(uri).is_empty());
}

#[test]
fn formats_long_latex_document() {
    let chunk =
        "\\section{節、}\n本文、です。😀𠮷、\n\\begin{verbatim}\n、。\n\\end{verbatim}\n% 注、\n";
    let text = chunk.repeat(30_000); // About 3 MB.
    let expected = text.replace('、', "，").replace('。', "．");
    assert_formats_long_document("latex", "file:///long.tex", &text, &expected);
}

#[test]
fn formats_long_mdx_document() {
    let chunk = concat!(
        "## 節、です。\n\n",
        "本文、`コード、`と$a、b$です。😀\n\n",
        "<Note title=\"注、\">\n  中身、です。\n</Note>\n\n",
        "```js\nconst s = '、。'\n```\n\n",
    );
    let formatted_chunk = concat!(
        "## 節，です．\n\n",
        "本文，`コード、`と$a、b$です．😀\n\n",
        "<Note title=\"注、\">\n  中身，です．\n</Note>\n\n",
        "```js\nconst s = '、。'\n```\n\n",
    );
    let repeat = 5_000; // About 0.8 MB.
    let text = format!("---\ntitle: 題、\n---\n\n{}", chunk.repeat(repeat));
    let expected = format!(
        "---\ntitle: 題，\n---\n\n{}",
        formatted_chunk.repeat(repeat)
    );
    assert_formats_long_document("mdx", "file:///long.mdx", &text, &expected);
}
