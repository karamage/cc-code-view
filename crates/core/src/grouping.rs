//! AI 解析の前に使う、ディレクトリからの仮モジュール分割。
use crate::model::{FileEntry, Module};
use std::collections::BTreeMap;

const SPLIT_ABOVE: usize = 60;
const ROOT_NAME: &str = "(root)";

pub fn provisional_modules(files: &[FileEntry]) -> Vec<Module> {
    let mut out = Vec::new();
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    split("", &paths, &mut out);
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn split(prefix: &str, paths: &[&str], out: &mut Vec<Module>) {
    let mut children: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    let mut direct: Vec<&str> = Vec::new();
    for p in paths {
        let rest = &p[prefix.len()..];
        match rest.split_once('/') {
            Some((dir, _)) => children.entry(dir).or_default().push(p),
            None => direct.push(p),
        }
    }
    if !direct.is_empty() {
        let name = if prefix.is_empty() { ROOT_NAME.to_string() } else { format!("{}/(files)", prefix.trim_end_matches('/')) };
        out.push(module(&name, direct.iter().map(|s| s.to_string()).collect()));
    }
    for (dir, items) in children {
        let child = format!("{prefix}{dir}");
        if items.len() > SPLIT_ABOVE && items.iter().any(|p| p[child.len() + 1..].contains('/')) {
            split(&format!("{child}/"), &items, out);
        } else {
            out.push(module(&child, vec![child.clone()]));
        }
    }
}

fn module(name: &str, paths: Vec<String>) -> Module {
    Module { id: name.to_string(), name: name.to_string(), summary: String::new(), paths }
}

/// ファイルごとのモジュール添字を返す。最長の接頭辞に一致したものを選ぶ。
/// どれにも属さないファイルは `modules.len()`（「その他」）になる。
pub fn assign_files(files: &[FileEntry], modules: &[Module]) -> Vec<usize> {
    files
        .iter()
        .map(|f| {
            let mut best: Option<(usize, usize)> = None;
            for (mi, m) in modules.iter().enumerate() {
                for p in &m.paths {
                    let matches = f.path == *p
                        || (f.path.starts_with(p.as_str()) && f.path.as_bytes().get(p.len()) == Some(&b'/'));
                    if matches && best.is_none_or(|(len, _)| p.len() > len) {
                        best = Some((p.len(), mi));
                    }
                }
            }
            best.map_or(modules.len(), |(_, mi)| mi)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Lang;

    fn file(path: &str) -> FileEntry {
        FileEntry { path: path.into(), lang: Lang::Other, lines: 1, bytes: 0, binary: false, churn: 0 }
    }

    #[test]
    fn provisional_splits_large_directories() {
        let mut files = vec![file("README.md")];
        for i in 0..300 {
            files.push(file(&format!("big/sub{}/f{i}.rs", i % 3)));
        }
        files.push(file("small/a.rs"));
        let names: Vec<_> = provisional_modules(&files).into_iter().map(|m| m.name).collect();
        assert_eq!(names, ["(root)", "big/sub0", "big/sub1", "big/sub2", "small"]);
    }

    #[test]
    fn every_file_is_assigned_provisionally() {
        let mut files = vec![file("README.md"), file("big/top.rs")];
        for i in 0..300 {
            files.push(file(&format!("big/sub/f{i}.rs")));
        }
        let modules = provisional_modules(&files);
        assert!(assign_files(&files, &modules).iter().all(|m| *m < modules.len()));
    }

    fn m(p: &str) -> Module {
        Module { id: p.into(), name: p.into(), summary: String::new(), paths: vec![p.into()] }
    }

    #[test]
    fn assign_uses_longest_prefix_and_other() {
        let files = [file("a/b/c.rs"), file("a/d.rs"), file("z.rs")];
        assert_eq!(assign_files(&files, &[m("a"), m("a/b")]), [1, 0, 2]);
    }

    #[test]
    fn prefix_matches_only_whole_path_segments() {
        assert_eq!(assign_files(&[file("ab/c.rs")], &[m("a")]), [1]);
    }
}
