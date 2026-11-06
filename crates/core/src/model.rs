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
