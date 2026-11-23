# tex-japanese-formatter-for-zed

日本語の学術文書向けに，読点「、」を全角カンマ「，」に，句点「。」を全角ピリオド「．」に変換する Zed 拡張機能である．

## 対象とする言語

既定の対象は LaTeX である．他は `targetLanguages` に追加すると対象になる．

| Zed の言語名 | `targetLanguages` の値 | 変換する範囲 |
|---|---|---|
| LaTeX | `latex` | 文書全体 |
| Plain Text | `plaintext` | 文書全体 |
| Markdown | `markdown` | 本文と frontmatter の値 |
| MDX | `mdx` | 本文と frontmatter の値 |

### Markdown／MDX で変換しないもの

- コードブロック，インラインコード
- 数式（`$...$`，`$$...$$`）
- URL（リンク先，自動リンク）
- 画像の代替テキスト，リンクの title
- HTML（Markdown のみ）
- `import` / `export` 文，`{式}`，JSX の属性値（MDX のみ）
- frontmatter のキーとコメント，TOML の frontmatter（`+++`）

MDX として解析できない文書（JSX のタグが閉じていないなど）は変換しない．

## 設定

`settings.json` に記述する．以下は Markdown と MDX を対象に加える例である．

```json
{
  "lsp": {
    "tex-japanese-formatter": {
      "settings": {
        "enabled": true,
        "formatOnSave": true,
        "targetLanguages": ["latex", "markdown", "mdx"],
        "frontmatterExcludeKeys": ["slug", "category"]
      }
    }
  }
}
```

| 項目 | 型 | 既定値 | 説明 |
|---|---|---|---|
| `enabled` | boolean | `true` | 拡張機能を有効にするか |
| `formatOnSave` | boolean | `true` | フォーマットを実行するか |
| `targetLanguages` | array | `["latex"]` | 変換する言語 |
| `frontmatterExcludeKeys` | array | `[]` | 値を変換しない frontmatter のトップレベルのキー |

### 他のフォーマッタと併用する

他の言語サーバや Prettier などを使う言語では，このフォーマッタを明示的に指定する．

```json
{
  "languages": {
    "MDX": {
      "format_on_save": "on",
      "formatter": [
        { "external": { "command": "prettier", "arguments": ["--stdin-filepath", "{buffer_path}"] } },
        { "language_server": { "name": "tex-japanese-formatter" } }
      ]
    }
  }
}
```
