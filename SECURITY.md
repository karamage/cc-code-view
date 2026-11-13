# Security Policy

## Reporting a vulnerability

Please report vulnerabilities privately through [GitHub's vulnerability reporting](https://github.com/karamage/cc-code-view/security/advisories/new). Do not open a public issue.

## Scope

CC Code View treats every registered repository as untrusted input. Reports are especially welcome if a repository can make Claude write files, run commands, read files outside the repository, read secret files that should be denied, or load the repository's own `.claude/` hooks or `.mcp.json`. See the Security section of the [README](README.md#-security) for the intended isolation.

## Supported versions

Only the latest commit on `main` receives security fixes.

---

## 脆弱性の報告

脆弱性は [GitHub の非公開の報告機能](https://github.com/karamage/cc-code-view/security/advisories/new)から報告してください。公開の Issue には書かないでください。

## 対象

CC Code View は、登録したリポジトリをすべて信頼できない入力として扱います。リポジトリの中身によって Claude にファイルを書かせる、コマンドを実行させる、リポジトリの外のファイルや読ませないはずの秘密ファイルを読ませる、リポジトリ内の `.claude/` のフックや `.mcp.json` を読み込ませる、といったことができる場合は特に報告をお願いします。想定している隔離は [README](README.ja.md#-セキュリティ) のセキュリティの節にあります。

## サポートするバージョン

セキュリティの修正を行うのは `main` の最新のコミットだけです。
