//! リポジトリの走査。
use crate::model::{FileEntry, Lang};
use ignore::{WalkBuilder, WalkState};
use std::path::Path;
use std::sync::Mutex;

/// `.gitignore` を尊重して走査する。読めないファイルも一覧には含める。
pub fn walk_repo(root: &Path) -> Vec<FileEntry> {
    let found = Mutex::new(Vec::new());
    WalkBuilder::new(root)
        .hidden(false)
        .filter_entry(|e| e.file_name() != ".git")
        .build_parallel()
        .run(|| {
            let found = &found;
            Box::new(move |entry| {
                let Ok(entry) = entry else { return WalkState::Continue };
                if !entry.file_type().is_some_and(|t| t.is_file()) {
                    return WalkState::Continue;
                }
                let Ok(rel) = entry.path().strip_prefix(root) else {
                    return WalkState::Continue;
                };
                let path = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
                found.lock().unwrap().push(read_entry(entry.path(), path));
                WalkState::Continue
            })
        });
    let mut files = found.into_inner().unwrap();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    files
}

fn read_entry(abs: &Path, path: String) -> FileEntry {
    let lang = Lang::from_path(&path);
    match std::fs::read(abs) {
        Ok(bytes) => {
            let binary = bytes[..bytes.len().min(8192)].contains(&0);
            FileEntry {
                path,
                lang: if binary { Lang::Other } else { lang },
                lines: if binary { 0 } else { count_lines(&bytes) },
                bytes: bytes.len() as u64,
                binary,
                churn: 0,
            }
        }
        Err(_) => FileEntry { path, lang: Lang::Other, lines: 0, bytes: 0, binary: true, churn: 0 },
    }
}

fn count_lines(bytes: &[u8]) -> u32 {
    let newlines = bytes.iter().filter(|b| **b == b'\n').count() as u32;
    if bytes.last().is_some_and(|b| *b != b'\n') { newlines + 1 } else { newlines }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn walk_respects_gitignore_and_counts_lines() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::process::Command::new("git").arg("init").arg("-q").current_dir(root).status().unwrap();
        std::fs::write(root.join(".gitignore"), "ignored.rs\n").unwrap();
        std::fs::write(root.join("ignored.rs"), "x").unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        std::fs::write(root.join("src/a.rs"), "fn a() {}\nfn b() {}\n").unwrap();
        std::fs::write(root.join("b.bin"), [0u8, 1, 2]).unwrap();

        let files = walk_repo(root);
        let paths: Vec<_> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, [".gitignore", "b.bin", "src/a.rs"]);
        let a = files.iter().find(|f| f.path == "src/a.rs").unwrap();
        assert_eq!((a.lang, a.lines, a.binary), (Lang::Rust, 2, false));
        assert!(files.iter().find(|f| f.path == "b.bin").unwrap().binary);
    }
}
