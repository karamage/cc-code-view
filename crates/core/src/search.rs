//! ファイル名のあいまい検索と全文検索。
use crate::model::FileEntry;
use grep::regex::RegexMatcherBuilder;
use grep::searcher::sinks::UTF8;
use grep::searcher::{BinaryDetection, SearcherBuilder};
use ignore::WalkBuilder;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// 得点順に、`files` の添字と一致した文字位置を返す。
pub fn fuzzy_files(query: &str, files: &[FileEntry], limit: usize) -> Vec<(usize, Vec<u32>)> {
    if query.trim().is_empty() {
        return (0..files.len().min(limit)).map(|i| (i, Vec::new())).collect();
    }
    let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
    let pattern = Pattern::parse(query, CaseMatching::Smart, Normalization::Smart);
    let mut buf = Vec::new();
    let mut scored: Vec<(u32, u8, usize, Vec<u32>)> = Vec::new();
    for (i, f) in files.iter().enumerate() {
        let hay = Utf32Str::new(&f.path, &mut buf);
        let mut indices = Vec::new();
        let Some(score) = pattern.indices(hay, &mut matcher, &mut indices) else { continue };
        let base = f.path.rsplit('/').next().unwrap_or(&f.path);
        let in_base = u8::from(base.to_lowercase().contains(&query.to_lowercase()));
        indices.sort_unstable();
        indices.dedup();
        scored.push((score, in_base, i, indices));
    }
    scored.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then(b.0.cmp(&a.0))
            .then(files[a.2].path.len().cmp(&files[b.2].path.len()))
            .then(a.2.cmp(&b.2))
    });
    scored.truncate(limit);
    scored.into_iter().map(|(_, _, i, idx)| (i, idx)).collect()
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hit {
    pub path: String,
    /// 0 始まり
    pub line: u32,
    pub text: String,
}

/// 大文字小文字を無視する固定文字列検索。`cancel` が立つと打ち切る。
pub fn search_content(root: &Path, query: &str, limit: usize, cancel: &AtomicBool) -> Vec<Hit> {
    if query.is_empty() {
        return Vec::new();
    }
    let Ok(matcher) =
        RegexMatcherBuilder::new().case_insensitive(true).fixed_strings(true).build(query)
    else {
        return Vec::new();
    };
    let mut searcher =
        SearcherBuilder::new().binary_detection(BinaryDetection::quit(0)).line_number(true).build();
    let mut hits = Vec::new();
    let mut paths: Vec<_> = WalkBuilder::new(root)
        .hidden(false)
        .filter_entry(|e| e.file_name() != ".git")
        .build()
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .map(|e| e.into_path())
        .collect();
    paths.sort();
    for path in paths {
        if cancel.load(Ordering::Relaxed) || hits.len() >= limit {
            break;
        }
        let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
        let _ = searcher.search_path(
            &matcher,
            &path,
            UTF8(|line, text| {
                hits.push(Hit {
                    path: rel.clone(),
                    line: line.saturating_sub(1) as u32,
                    text: text.trim_end().to_string(),
                });
                Ok(hits.len() < limit && !cancel.load(Ordering::Relaxed))
            }),
        );
    }
    if cancel.load(Ordering::Relaxed) {
        hits.clear();
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Lang;

    fn f(p: &str) -> FileEntry {
        FileEntry { path: p.into(), lang: Lang::Other, lines: 0, bytes: 0, binary: false, churn: 0 }
    }

    #[test]
    fn fuzzy_ranks_basename_match_first() {
        let files = [f("main/zzz/other.rs"), f("zzz/main_helper_long.rs"), f("src/main.rs")];
        let hits = fuzzy_files("main", &files, 10);
        assert_eq!(hits[0].0, 2);
        assert_eq!(hits[0].1.len(), 4);
    }

    #[test]
    fn empty_query_lists_files_up_to_limit() {
        let files = [f("a"), f("b"), f("c")];
        assert_eq!(fuzzy_files("", &files, 2).len(), 2);
    }

    #[test]
    fn content_search_respects_limit_and_cancel() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "Needle\nnone\nneedle\n").unwrap();
        let cancel = AtomicBool::new(false);
        let hits = search_content(dir.path(), "needle", 10, &cancel);
        assert_eq!(hits.iter().map(|h| h.line).collect::<Vec<_>>(), [0, 2]);
        assert_eq!(hits[0].path, "a.txt");
        assert_eq!(search_content(dir.path(), "needle", 1, &cancel).len(), 1);
        cancel.store(true, Ordering::Relaxed);
        assert!(search_content(dir.path(), "needle", 10, &cancel).is_empty());
    }

    #[test]
    fn regex_characters_are_literal() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "a.b\naxb\n").unwrap();
        let hits = search_content(dir.path(), "a.b", 10, &AtomicBool::new(false));
        assert_eq!(hits.len(), 1);
    }
}
