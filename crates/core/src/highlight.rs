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

const NAMES: [&str; 21] = [
    "keyword",
    "function",
    "type",
    "string",
    "number",
    "comment",
    "constant",
    "constructor",
    "property",
    "string.special.key",
    "tag",
    "attribute",
    "text.title",
    "text.literal",
    "text.uri",
    "text.reference",
    "punctuation.special",
    "boolean",
    "escape",
    "label",
    "variable.builtin",
];

fn token_of(index: usize) -> Token {
    match NAMES[index] {
        // 演算子は色を付けない。記号まで色が付くと、名前が埋もれる
        "keyword" | "tag" => Token::Keyword,
        "function" | "text.title" | "label" => Token::Function,
        "type" | "constructor" | "property" | "string.special.key" | "attribute" | "text.uri"
        | "text.reference" => Token::Type,
        "string" | "text.literal" => Token::String,
        "number" | "escape" => Token::Number,
        "comment" | "punctuation.special" => Token::Comment,
        "constant" | "boolean" | "variable.builtin" => Token::Constant,
        _ => Token::Plain,
    }
}

/// 色付けに使う文法。解析の対象になる言語に加え、設定や文書の形式も読めるようにする。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Syntax {
    Code(Lang),
    Json,
    Toml,
    Css,
    Yaml,
    Shell,
    Markdown,
}

impl Syntax {
    pub fn from_path(path: &str) -> Option<Syntax> {
        let lang = Lang::from_path(path);
        if lang != Lang::Other {
            return Some(Syntax::Code(lang));
        }
        let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
        let ext = name.rsplit_once('.').map(|(_, e)| e.to_string()).unwrap_or_default();
        Some(match ext.as_str() {
            "json" | "jsonc" | "json5" => Syntax::Json,
            "toml" => Syntax::Toml,
            "css" | "scss" => Syntax::Css,
            "yml" | "yaml" => Syntax::Yaml,
            "sh" | "bash" | "zsh" => Syntax::Shell,
            "md" | "mdx" | "markdown" => Syntax::Markdown,
            _ if matches!(name.as_str(), ".zshrc" | ".bashrc" | ".profile") => Syntax::Shell,
            _ if name == "cargo.lock" => Syntax::Toml,
            _ => return None,
        })
    }

    fn grammar(self) -> Option<(tree_sitter::Language, &'static str)> {
        Some(match self {
            Syntax::Code(lang) => (language(lang)?, highlights_query(lang)),
            Syntax::Json => (tree_sitter_json::LANGUAGE.into(), tree_sitter_json::HIGHLIGHTS_QUERY),
            Syntax::Toml => (tree_sitter_toml_ng::LANGUAGE.into(), tree_sitter_toml_ng::HIGHLIGHTS_QUERY),
            Syntax::Css => (tree_sitter_css::LANGUAGE.into(), tree_sitter_css::HIGHLIGHTS_QUERY),
            Syntax::Yaml => (tree_sitter_yaml::LANGUAGE.into(), tree_sitter_yaml::HIGHLIGHTS_QUERY),
            Syntax::Shell => (tree_sitter_bash::LANGUAGE.into(), tree_sitter_bash::HIGHLIGHT_QUERY),
            // 見出しと引用、コードの区画が分かれば十分なので、ブロックの文法だけを使う
            Syntax::Markdown => (tree_sitter_md::LANGUAGE.into(), tree_sitter_md::HIGHLIGHT_QUERY_BLOCK),
        })
    }
}

fn config(syntax: Syntax) -> Option<&'static HighlightConfiguration> {
    static CONFIGS: OnceLock<Mutex<HashMap<Syntax, Option<&'static HighlightConfiguration>>>> =
        OnceLock::new();
    let mut map = CONFIGS.get_or_init(Default::default).lock().ok()?;
    if let Some(c) = map.get(&syntax) {
        return *c;
    }
    // 作れなかった文法も覚えておき、毎回作り直さない
    let built = syntax.grammar().and_then(|(language, query)| {
        let mut c = HighlightConfiguration::new(language, "code", query, "", "").ok()?;
        c.configure(&NAMES);
        // 文法ごとに 1 度だけ作り、終了まで保持する
        let leaked: &'static HighlightConfiguration = Box::leak(Box::new(c));
        Some(leaked)
    });
    map.insert(syntax, built);
    built
}

/// パスから文法を選んで色を付ける。
pub fn highlight_path(path: &str, source: &str) -> Vec<Vec<Span>> {
    match Syntax::from_path(path) {
        Some(syntax) => highlight_syntax(syntax, source),
        None => Vec::new(),
    }
}

/// 行ごとの色の区間を返す。対応外の言語や大きすぎるファイルは空を返す。
pub fn highlight(lang: Lang, source: &str) -> Vec<Vec<Span>> {
    highlight_syntax(Syntax::Code(lang), source)
}

fn highlight_syntax(syntax: Syntax, source: &str) -> Vec<Vec<Span>> {
    if source.len() as u64 > MAX_EXTRACT_BYTES {
        return Vec::new();
    }
    let Some(cfg) = config(syntax) else { return Vec::new() };
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
        assert!(highlight_path("notes.txt", "x").is_empty());
        assert!(highlight(Lang::Rust, &" ".repeat(MAX_EXTRACT_BYTES as usize + 1)).is_empty());
    }

    #[test]
    fn config_formats_get_colors() {
        for (path, src) in [
            ("a.json", "{\"k\": 1}"),
            ("Cargo.toml", "[package]\nname = \"x\""),
            ("a.css", "body { color: red; }"),
            ("a.yml", "key: value"),
            ("run.sh", "echo \"hi\""),
            ("README.md", "# Title\n\n```\ncode\n```"),
        ] {
            let lines = highlight_path(path, src);
            assert!(lines.iter().any(|l| !l.is_empty()), "{path} に色が付かない");
        }
    }
}
