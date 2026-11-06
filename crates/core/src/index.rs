//! リポジトリ全体の集計。
use crate::churn::{churn_counts, head_sha};
use crate::deps::resolve_imports;
use crate::model::{Lang, RepoIndex, Symbol};
use crate::symbols::extract;
use crate::walk::walk_repo;
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// これより大きいファイルからはシンボルを取らない。
pub const MAX_EXTRACT_BYTES: u64 = 2 * 1024 * 1024;

pub fn build_index(root: &Path) -> RepoIndex {
    let mut files = walk_repo(root);
    let churn = churn_counts(root);
    for f in &mut files {
        f.churn = churn.get(&f.path).copied().unwrap_or(0);
    }
    let all: HashSet<String> = files.iter().map(|f| f.path.clone()).collect();
    let position: HashMap<&str, u32> =
        files.iter().enumerate().map(|(i, f)| (f.path.as_str(), i as u32)).collect();

    let extracted: Vec<(String, Vec<Symbol>, Vec<String>)> = files
        .par_iter()
        .filter(|f| f.lang != Lang::Other && !f.binary && f.bytes <= MAX_EXTRACT_BYTES)
        .filter_map(|f| {
            let source = std::fs::read_to_string(root.join(&f.path)).ok()?;
            let e = extract(f.lang, &source);
            let deps = resolve_imports(&f.path, f.lang, &e.imports, &all);
            Some((f.path.clone(), e.symbols, deps))
        })
        .collect();

    let mut symbols = HashMap::new();
    let mut edges = Vec::new();
    for (path, syms, deps) in extracted {
        let from = position[path.as_str()];
        edges.extend(deps.iter().map(|d| (from, position[d.as_str()])));
        if !syms.is_empty() {
            symbols.insert(path, syms);
        }
    }
    edges.sort_unstable();
    RepoIndex { head: head_sha(root), files, symbols, edges }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_index_end_to_end() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let git = |args: &[&str]| {
            std::process::Command::new("git").args(args).current_dir(root).status().unwrap();
        };
        git(&["init", "-q"]);
        std::fs::write(root.join("a.ts"), "import './b';\nexport function a() {}\n").unwrap();
        std::fs::write(root.join("b.ts"), "export function b() {}\n").unwrap();
        git(&["add", "."]);
        git(&["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-qm", "x"]);

        let idx = build_index(root);
        assert_eq!(idx.head.as_ref().map(|s| s.len()), Some(40));
        assert_eq!(idx.files.iter().map(|f| f.churn).max(), Some(1));
        assert_eq!(idx.edges, [(0, 1)]);
        assert_eq!(idx.symbols["a.ts"][0].name, "a");
    }
}
