//! クレート間で共有するデータ型。
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Lang {
    Rust,
    TypeScript,
    Tsx,
    JavaScript,
    Python,
    Go,
    Other,
}

impl Lang {
    pub fn from_path(path: &str) -> Lang {
        match path.rsplit_once('.').map(|(_, ext)| ext) {
            Some("rs") => Lang::Rust,
            Some("ts" | "mts" | "cts") => Lang::TypeScript,
            Some("tsx") => Lang::Tsx,
            Some("js" | "jsx" | "mjs" | "cjs") => Lang::JavaScript,
            Some("py") => Lang::Python,
            Some("go") => Lang::Go,
            _ => Lang::Other,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FileEntry {
    /// リポジトリ相対で `/` 区切り
    pub path: String,
    pub lang: Lang,
    pub lines: u32,
    pub bytes: u64,
    pub binary: bool,
    pub churn: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolKind {
    Function,
    Type,
    Module,
    Constant,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    /// 0 始まり
    pub line: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct RepoIndex {
    pub head: Option<String>,
    pub files: Vec<FileEntry>,
    pub symbols: HashMap<String, Vec<Symbol>>,
    /// `files` の添字の組。依存元 → 依存先
    pub edges: Vec<(u32, u32)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Module {
    pub id: String,
    pub name: String,
    pub summary: String,
    /// ファイルかディレクトリの接頭辞
    pub paths: Vec<String>,
}

/// AI 処理に使うモデル。別名を渡すので、各系列の最新が使われる。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelChoice {
    #[default]
    Default,
    Fable,
    Opus,
    Sonnet,
    Haiku,
}

impl ModelChoice {
    pub const ALL: [ModelChoice; 5] = [
        ModelChoice::Default,
        ModelChoice::Fable,
        ModelChoice::Opus,
        ModelChoice::Sonnet,
        ModelChoice::Haiku,
    ];

    /// `None` なら `--model` を付けず、Claude の設定に任せる。
    pub fn cli_alias(self) -> Option<&'static str> {
        match self {
            ModelChoice::Default => None,
            ModelChoice::Fable => Some("fable"),
            ModelChoice::Opus => Some("opus"),
            ModelChoice::Sonnet => Some("sonnet"),
            ModelChoice::Haiku => Some("haiku"),
        }
    }

    /// 選ぶときの目安。
    pub fn hint(self) -> &'static str {
        match self {
            ModelChoice::Default => "Claude Code の設定に従います",
            ModelChoice::Fable | ModelChoice::Opus => "じっくり読みます。利用枠を多く使います",
            ModelChoice::Sonnet => "速さと質のバランスを取ります",
            ModelChoice::Haiku => "速く読みます。利用枠をあまり使いません",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ModelChoice::Default => "Claude の既定",
            ModelChoice::Fable => "Fable",
            ModelChoice::Opus => "Opus",
            ModelChoice::Sonnet => "Sonnet",
            ModelChoice::Haiku => "Haiku",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TourStop {
    pub title: String,
    pub body: String,
    pub path: String,
    /// 1 始まり
    pub start_line: u32,
    pub end_line: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Analysis {
    pub head: Option<String>,
    pub model: ModelChoice,
    pub overview: String,
    pub modules: Vec<Module>,
    pub tour: Vec<TourStop>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeyFile {
    pub path: String,
    pub why: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DeepDive {
    pub module_id: String,
    pub head: Option<String>,
    pub model: ModelChoice,
    pub explanation: String,
    pub key_files: Vec<KeyFile>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RepoEntry {
    pub path: PathBuf,
    pub name: String,
    #[serde(default)]
    pub tour_position: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub repos: Vec<RepoEntry>,
    #[serde(default)]
    pub last_model: ModelChoice,
}

/// ホーム画面に出す、リポジトリの小さな星図。座標は 0.0〜1.0 に正規化する。
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SkyPreview {
    /// x, y, 半径, 明るさ
    pub stars: Vec<(f32, f32, f32, f32)>,
    pub lines: Vec<(u32, u32)>,
}

/// 設計図の章。コードを読むときの観点ごとに一つ。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChapterKind {
    Architecture,
    DataStructures,
    DataFlow,
    Domain,
    UseCases,
    Modules,
}

impl ChapterKind {
    pub const ALL: [ChapterKind; 6] = [
        ChapterKind::Architecture,
        ChapterKind::DataStructures,
        ChapterKind::DataFlow,
        ChapterKind::Domain,
        ChapterKind::UseCases,
        ChapterKind::Modules,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            ChapterKind::Architecture => "architecture",
            ChapterKind::DataStructures => "data-structures",
            ChapterKind::DataFlow => "data-flow",
            ChapterKind::Domain => "domain",
            ChapterKind::UseCases => "use-cases",
            ChapterKind::Modules => "modules",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            ChapterKind::Architecture => "全体アーキテクチャ",
            ChapterKind::DataStructures => "データ構造",
            ChapterKind::DataFlow => "データフロー",
            ChapterKind::Domain => "ドメイン",
            ChapterKind::UseCases => "ユースケース",
            ChapterKind::Modules => "画面とモジュール構成",
        }
    }

    /// 章が答える問い。目次と空の状態に出す
    pub fn question(self) -> &'static str {
        match self {
            ChapterKind::Architecture => "どんな部品が、どの向きに依存しているか",
            ChapterKind::DataStructures => "中心になる型と、その持ち物と関係",
            ChapterKind::DataFlow => "入力が出力になるまでの変換の道筋",
            ChapterKind::Domain => "扱っている問題の言葉と決まりごと",
            ChapterKind::UseCases => "使う人から見て、何ができるか",
            ChapterKind::Modules => "画面とモジュールの分け方と行き来",
        }
    }

    pub fn number(self) -> usize {
        ChapterKind::ALL.iter().position(|k| *k == self).unwrap_or(0) + 1
    }
}

/// 解説が根拠にするコードの範囲。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CodeRef {
    pub path: String,
    /// 1 始まり
    pub start_line: u32,
    pub end_line: u32,
}

/// 図の要素とファイルの対応。図から直接コードを開ける。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NodeLink {
    pub id: String,
    pub path: String,
    /// 1 始まり。0 ならファイルの先頭
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DiagramSpec {
    pub title: String,
    pub caption: String,
    /// 検証済みの Mermaid
    pub mermaid: String,
    pub links: Vec<NodeLink>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChapterPoint {
    pub heading: String,
    pub body: String,
    pub refs: Vec<CodeRef>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Term {
    pub term: String,
    pub meaning: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Chapter {
    pub kind: ChapterKind,
    pub head: Option<String>,
    pub model: ModelChoice,
    /// UNIX 時刻の秒
    #[serde(default)]
    pub generated_at: u64,
    pub headline: String,
    pub summary: String,
    pub diagrams: Vec<DiagramSpec>,
    pub points: Vec<ChapterPoint>,
    #[serde(default)]
    pub glossary: Vec<Term>,
}
