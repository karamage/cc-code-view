//! 行ごとのシンタックスハイライト。
use crate::index::MAX_EXTRACT_BYTES;
use crate::lang::{highlights_query, language};
use crate::model::Lang;
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use tree_sitter_highlight::{HighlightConfiguration, HighlightEvent, Highlighter};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Keyword,
    Function,
    Type,
    String,
    Number,
    Comment,
    Constant,
    Plain,
}

/// `start..end` は行内のバイト位置。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub start: u32,
    pub end: u32,
    pub token: Token,
}

const NAMES: [&str; 8] = [
    "keyword",
    "function",
    "type",
    "string",
    "number",
    "comment",
    "constant",
    "constructor",
];

fn token_of(index: usize) -> Token {
    match NAMES[index] {
        "keyword" => Token::Keyword,
        "function" => Token::Function,
        "type" | "constructor" => Token::Type,
        "string" => Token::String,
        "number" => Token::Number,
        "comment" => Token::Comment,
        "constant" => Token::Constant,
        _ => Token::Plain,
    }
}

fn config(lang: Lang) -> Option<&'static HighlightConfiguration> {
    static CONFIGS: OnceLock<Mutex<HashMap<Lang, &'static HighlightConfiguration>>> =
        OnceLock::new();
    let mut map = CONFIGS.get_or_init(Default::default).lock().ok()?;
    if let Some(c) = map.get(&lang) {
        return Some(c);
    }
    let mut c =
        HighlightConfiguration::new(language(lang)?, "code", highlights_query(lang), "", "").ok()?;
    c.configure(&NAMES);
    // 言語ごとに 1 度だけ作り、終了まで保持する
    let leaked: &'static HighlightConfiguration = Box::leak(Box::new(c));
    map.insert(lang, leaked);
    Some(leaked)
}

/// 行ごとの色の区間を返す。対応外の言語や大きすぎるファイルは空を返す。
pub fn highlight(lang: Lang, source: &str) -> Vec<Vec<Span>> {
    if source.len() as u64 > MAX_EXTRACT_BYTES {
        return Vec::new();
    }
    let Some(cfg) = config(lang) else { return Vec::new() };
    let mut highlighter = Highlighter::new();
    let Ok(events) = highlighter.highlight(cfg, source.as_bytes(), None, |_| None) else {
        return Vec::new();
    };

    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(source.bytes().enumerate().filter(|(_, b)| *b == b'\n').map(|(i, _)| i + 1))
        .collect();
    let mut lines: Vec<Vec<Span>> = vec![Vec::new(); line_starts.len()];
    let mut stack: Vec<Token> = Vec::new();

    for event in events {
        let Ok(event) = event else { return lines };
        match event {
            HighlightEvent::HighlightStart(h) => stack.push(token_of(h.0)),
            HighlightEvent::HighlightEnd => {
                stack.pop();
            }
            HighlightEvent::Source { start, end } => {
                let Some(token) = stack.last().copied() else { continue };
                push_span(&mut lines, &line_starts, source, start, end, token);
            }
        }
    }
    lines
}

fn push_span(
    lines: &mut [Vec<Span>],
    starts: &[usize],
    source: &str,
    start: usize,
    end: usize,
    token: Token,
) {
    let mut line = starts.partition_point(|s| *s <= start) - 1;
    let mut pos = start;
    while pos < end && line < starts.len() {
        let line_end = starts
            .get(line + 1)
            .map_or(source.len(), |next| next - 1)
            .min(end);
        if line_end > pos {
            lines[line].push(Span {
                start: (pos - starts[line]) as u32,
                end: (line_end - starts[line]) as u32,
                token,
            });
        }
        line += 1;
        pos = starts.get(line).copied().unwrap_or(end);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn highlights_rust_by_line() {
        let lines = highlight(Lang::Rust, "fn main() {\n  // hi\n}\n");
        assert!(lines[0].iter().any(|s| s.token == Token::Keyword && (s.start, s.end) == (0, 2)));
        assert!(lines[1].iter().any(|s| s.token == Token::Comment));
    }

    #[test]
    fn multiline_tokens_split_across_lines() {
        let lines = highlight(Lang::Rust, "/* a\nb */\n");
        assert_eq!(lines[0][0].token, Token::Comment);
        assert_eq!(lines[1][0].token, Token::Comment);
        assert_eq!((lines[1][0].start, lines[1][0].end), (0, 4));
    }

    #[test]
    fn all_languages_compile_their_queries() {
        for (lang, src) in [
            (Lang::TypeScript, "const a: string = 'x';"),
            (Lang::Tsx, "const a = <div/>;"),
            (Lang::JavaScript, "const a = 'x';"),
            (Lang::Python, "def f(): return 'x'"),
            (Lang::Go, "package p\nvar a = \"x\""),
        ] {
            let spans: Vec<Span> = highlight(lang, src).into_iter().flatten().collect();
            assert!(spans.iter().any(|s| s.token != Token::Plain), "{lang:?}");
        }
    }

    #[test]
    fn unsupported_or_huge_input_is_empty() {
        assert!(highlight(Lang::Other, "x").is_empty());
        assert!(highlight(Lang::Rust, &" ".repeat(MAX_EXTRACT_BYTES as usize + 1)).is_empty());
    }
}
