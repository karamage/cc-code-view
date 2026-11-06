//! クレート間で共有するデータ型。
use serde::{Deserialize, Serialize};

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
