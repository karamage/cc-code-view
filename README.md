# CC Code View

コードベースの星図を、AI の案内で航海する。大きなリポジトリを読むための macOS アプリです。

左のファイルツリー、タブ、コードという普通のビューアーの形で読めます。
右には現在地の星図があり、いま読んでいるファイルがリポジトリ全体のどこにあるかを示します。
手元の Claude Code（`claude -p`）が星座に名前を付け、読む順番をガイドツアーにまとめます。
Rust と [gpui](https://github.com/zed-industries/zed/tree/main/crates/gpui) で作っています。

![コードを読む画面。左にファイルツリー、中央にタブとコード、右に現在地の星図とアウトライン](docs/images/code.png)

![全景。AI が名付けた星座、読む順番の航路、星座の一覧](docs/images/overview.png)

![ガイドツアー。星図、コード、解説が同期する](docs/images/tour.png)

> Anthropic とは関係のない個人のプロジェクトです。Claude は Anthropic, PBC の商標です。

## できること

- **ファイルツリー**: 読んでいるファイルまで自動で開いて示します。点の色は所属する星座の色で、ツアーの停留所には番号が付きます。矢印キーで移動でき、幅はドラッグで変えられます。
- **タブとパンくず**: 開いたファイルはタブに残り、戻ると読んでいた位置から再開します。パンくずの末尾には、いま読んでいる関数の名前が出ます。
- **現在地の星図とアウトライン**: 右のパネルに、読んでいるファイルの星と星座、その星座の説明、ファイル内の関数と型の一覧を出します。
- **星図**: 大きさがコード量、明るさが変更頻度、線が依存関係を表します。AI を使わず数秒で描きます。
- **ガイドツアー**: `←` `→` で進むと、星図のカメラ、コード、解説が同時に動きます。
- **コード表示**: シンタックスハイライト（Rust、TypeScript、JavaScript、Python、Go に加え JSON、TOML、YAML、CSS、シェル、Markdown）、ファイル内検索、行への移動、簡易な定義ジャンプ、移動履歴、行のコピー。
- **検索**: ファイル名のあいまい検索と全文検索。
- **深掘り**: 星座を開いて「解説を生成」を押すと、そのモジュールの解説を作って保存します。
- **モデル選択**: AI を実行するすべての場所で、Claude の既定、Fable、Opus、Sonnet、Haiku から選べます。

## 動作要件

- macOS 13 以降（Apple Silicon で確認）
- AI 機能には Claude Code が要ります。`--restricted` に対応した版が必要です（2.1.319 で確認）。Claude Code が無くても星図とコード表示は使えます。

## 作り方

```bash
scripts/bundle.sh   # target/CC Code View.app を作る（アドホック署名、公証なし）
```

Rust stable を使います。gpui の `runtime_shaders` 機能を使うので、Metal Toolchain は不要です。

## キー操作

| キー | 動作 |
|---|---|
| `⌘P` / `⌘⇧F` | ファイル検索 / 全文検索 |
| `⌘F`、`⏎` `⇧⏎`、`esc` | ファイル内検索、次と前の一致、閉じる |
| `⌘L` / `⌃G` | 行へ移動 |
| `⌘⇧E`、`↑` `↓` `←` `→` | ファイルツリーへ移り、矢印で移動と開閉 |
| `⌘W`、`⌃⇥` `⌃⇧⇥` | タブを閉じる、タブを切り替える |
| `⌘1` | 星図のタブへ |
| `⌘B` / `⌘⇧L` | ファイルツリー / 右のパネルの表示切り替え |
| `Enter` | AI 解析を開始（星図のタブで） |
| `T`、`←` `→`、`esc` | ツアーの開始、移動、終了 |
| `⌘` + スクロール、`⌘=` `⌘-` `⌘0` | 星図のズーム、全景に戻す |
| `⌘` + クリック | 定義へ移動 |
| `⌘[` `⌘]`、`‹` `›` ボタン | 移動履歴を戻る、進む |
| ドラッグ、`⌘C` | 行を選択、コピー |
| `⌘⇧O` | リポジトリ一覧へ戻る |

## セキュリティ

クローンしたリポジトリは信頼できない入力として扱います。

- Claude は空の一時ディレクトリで動かし、リポジトリを読み取り専用で渡します。ツールは `Read`、`Grep`、`Glob` だけで、書き込みとコマンド実行はできません。
- `--restricted` と `--strict-mcp-config` により、リポジトリ内の `.claude/` のフックや `.mcp.json` は読み込まれません。
- `.env`、秘密鍵、認証情報のファイルは読ませません。読まれたファイルは Anthropic の API に送られるためです。一覧は `crates/ai/src/command.rs` にあります。ファイル名で判定するので、別の名前で置かれた秘密情報は防げません。
- AI の出力は検証し、リポジトリに無いファイルを指す項目は捨てます。文章は書式なしで表示します。
- 登録したリポジトリには何も書き込みません。データは `~/Library/Application Support/cc-code-view/` に置きます。

隔離は実機のテストで確認しています。

```bash
cargo test -p ccv-ai --test security -- --ignored   # 実際の claude を使い、利用枠を少し消費する
```

## 開発

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
scripts/capture.sh <出力先> <リポジトリ>   # 全状態を撮影（BIN=target/debug/cc-code-view で開発版を使う）
scripts/drive-viewer.sh <出力先> <リポジトリ>  # ツリー、検索、行移動を実際の入力でたどる
scripts/measure.sh [リポジトリ]            # 性能を実測
```

クレートは `core`（集計と保存）、`ai`（claude の実行と検証）、`app`（gpui の画面）の 3 つです。

## ライセンス

MIT。同梱の JetBrains Mono は SIL OFL（`crates/app/assets/fonts/OFL.txt`）、入力部品は gpui の例（Apache-2.0、`LICENSES/`）を元にしています。ホーム画面の星雲（`crates/app/assets/images/nebula.jpg`）は画像生成で作りました。
