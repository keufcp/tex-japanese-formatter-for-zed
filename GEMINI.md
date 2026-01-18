# コンセプト

- TeXファイル他のフォーマッタを実装
- Zed拡張機能として実装
- 日本語かつ学術文書向けに，「、」→「，」と「。」→「．」の変換を実装
  - カンマは全角カンマ，ピリオドは全角ピリオドである

# Zed拡張機能の作法に準拠すること

- https://zed.dev/docs/extensions/developing-extensions
- https://zed.dev/docs/extensions/languages
- https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocumentItem
- https://docs.rs/zed_extension_api/0.7.0/zed_extension_api/index.html

# 不明点は必ずユーザに確認を取ること

# ユーザの設定ファイルで設定できるようにすべきこと

- `enabled`
  - 型: boolean
  - デフォルト: true
  - 説明: 拡張機能を有効にするかどうか
- `formatOnSave`
  - 型: boolean
  - デフォルト: true
  - 説明: 保存時に自動フォーマットを実行するかどうか
- 対象のファイルの種類 `targetLanguages`
  - 型: array
  - デフォルト: ["latex"] ←例) Zedの記法に従うこと
  - 説明: フォーマットを適用するファイルタイプ
