//! シンボルと import の抽出。
use crate::lang::{language, tags_query};
use crate::model::{Lang, Symbol, SymbolKind};
use streaming_iterator::StreamingIterator;
use tree_sitter::{Parser, QueryCursor};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Extracted {
    pub symbols: Vec<Symbol>,
    pub imports: Vec<String>,
}

/// 構文エラーがあっても、取れた分を返す。
pub fn extract(lang: Lang, source: &str) -> Extracted {
    let (Some(ts_lang), Some(query)) = (language(lang), tags_query(lang)) else {
        return Extracted::default();
    };
    let mut parser = Parser::new();
    if parser.set_language(&ts_lang).is_err() {
        return Extracted::default();
    }
    let Some(tree) = parser.parse(source, None) else { return Extracted::default() };

    // 出現順に並べるため、いったんバイト位置つきで集める
    let mut found: Vec<(usize, Item)> = Vec::new();
    let mut cursor = QueryCursor::new();
    let names = query.capture_names();
    let mut matches = cursor.matches(query, tree.root_node(), source.as_bytes());
    while let Some(m) = matches.next() {
        for cap in m.captures {
            let text = &source[cap.node.byte_range()];
            let line = cap.node.start_position().row as u32;
            let start = cap.node.start_byte();
            let item = match names[cap.index as usize] {
                "function" => Item::symbol(text, SymbolKind::Function, line),
                "type" => Item::symbol(text, SymbolKind::Type, line),
                "constant" => Item::symbol(text, SymbolKind::Constant, line),
                "module" => {
                    let bodyless = cap.node.parent().is_some_and(|p| {
                        p.kind() == "mod_item" && p.child_by_field_name("body").is_none()
                    });
                    if bodyless {
                        found.push((start, Item::Import(format!("mod:{text}"))));
                    }
                    Item::symbol(text, SymbolKind::Module, line)
                }
                "import" => Item::Import(text.trim_matches('"').to_string()),
                _ => continue,
            };
            found.push((start, item));
        }
    }
    found.sort_by_key(|(start, _)| *start);

    let mut out = Extracted::default();
    for (_, item) in found {
        match item {
            Item::Symbol(s) => out.symbols.push(s),
            Item::Import(i) => {
                if !out.imports.contains(&i) {
                    out.imports.push(i);
                }
            }
        }
    }
    out
}

enum Item {
    Symbol(Symbol),
    Import(String),
}

impl Item {
    fn symbol(name: &str, kind: SymbolKind, line: u32) -> Item {
        Item::Symbol(Symbol { name: name.to_string(), kind, line })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(e: &Extracted) -> Vec<&str> {
        e.symbols.iter().map(|s| s.name.as_str()).collect()
    }

    #[test]
    fn rust() {
        let e = extract(Lang::Rust, "use crate::a::b;\nmod c;\npub struct S;\nfn f() {}\n");
        assert_eq!(names(&e), ["c", "S", "f"]);
        assert_eq!(e.imports, ["crate::a::b", "mod:c"]);
        assert_eq!(e.symbols[2].line, 3);
    }

    #[test]
    fn inline_rust_mod_is_not_an_import() {
        let e = extract(Lang::Rust, "mod c { fn g() {} }\n");
        assert!(e.imports.is_empty());
    }

    #[test]
    fn typescript() {
        let e = extract(
            Lang::TypeScript,
            "import x from './a';\nexport function g() {}\nclass K {}\n",
        );
        assert_eq!(names(&e), ["g", "K"]);
        assert_eq!(e.imports, ["./a"]);
    }

    #[test]
    fn tsx_and_javascript() {
        let e = extract(Lang::Tsx, "import './a';\nconst C = () => <div/>;\n");
        assert_eq!(names(&e), ["C"]);
        assert_eq!(e.imports, ["./a"]);
        let e = extract(Lang::JavaScript, "import './b';\nfunction h() {}\n");
        assert_eq!(names(&e), ["h"]);
    }

    #[test]
    fn python() {
        let e = extract(
            Lang::Python,
            "from a.b import c\nimport d\nfrom . import e\ndef f(): pass\nclass K: pass\n",
        );
        assert_eq!(names(&e), ["f", "K"]);
        assert_eq!(e.imports, ["a.b", "d", "."]);
    }

    #[test]
    fn go() {
        let e = extract(Lang::Go, "package p\nimport \"x/y\"\nfunc F() {}\ntype T struct{}\n");
        assert_eq!(names(&e), ["F", "T"]);
        assert_eq!(e.imports, ["x/y"]);
    }

    #[test]
    fn broken_source_returns_what_it_can() {
        let e = extract(Lang::Rust, "fn ok() {}\nfn broken( {");
        assert!(names(&e).contains(&"ok"));
    }
}
