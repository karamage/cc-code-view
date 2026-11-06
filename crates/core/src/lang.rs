//! 言語ごとの tree-sitter 文法とクエリ。
use crate::model::Lang;
use std::sync::OnceLock;
use tree_sitter::{Language, Query};

pub fn language(lang: Lang) -> Option<Language> {
    Some(match lang {
        Lang::Rust => tree_sitter_rust::LANGUAGE.into(),
        Lang::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        Lang::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Lang::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
        Lang::Python => tree_sitter_python::LANGUAGE.into(),
        Lang::Go => tree_sitter_go::LANGUAGE.into(),
        Lang::Other => return None,
    })
}

pub fn highlights_query(lang: Lang) -> &'static str {
    match lang {
        Lang::Rust => tree_sitter_rust::HIGHLIGHTS_QUERY,
        Lang::TypeScript | Lang::Tsx => TS_HIGHLIGHTS.get_or_init(|| {
            format!(
                "{}\n{}",
                tree_sitter_typescript::HIGHLIGHTS_QUERY,
                tree_sitter_javascript::HIGHLIGHT_QUERY
            )
        }),
        Lang::JavaScript => tree_sitter_javascript::HIGHLIGHT_QUERY,
        Lang::Python => tree_sitter_python::HIGHLIGHTS_QUERY,
        Lang::Go => tree_sitter_go::HIGHLIGHTS_QUERY,
        Lang::Other => "",
    }
}

static TS_HIGHLIGHTS: OnceLock<String> = OnceLock::new();

const RUST_QUERY: &str = r#"
(function_item name: (identifier) @function)
(function_signature_item name: (identifier) @function)
(struct_item name: (type_identifier) @type)
(enum_item name: (type_identifier) @type)
(union_item name: (type_identifier) @type)
(trait_item name: (type_identifier) @type)
(type_item name: (type_identifier) @type)
(const_item name: (identifier) @constant)
(static_item name: (identifier) @constant)
(mod_item name: (identifier) @module)
(use_declaration argument: (_) @import)
"#;

const TS_QUERY: &str = r#"
(function_declaration name: (identifier) @function)
(class_declaration name: (_) @type)
(interface_declaration name: (_) @type)
(type_alias_declaration name: (_) @type)
(enum_declaration name: (_) @type)
(lexical_declaration (variable_declarator name: (identifier) @function value: (arrow_function)))
(import_statement source: (string (string_fragment) @import))
(export_statement source: (string (string_fragment) @import))
"#;

const JS_QUERY: &str = r#"
(function_declaration name: (identifier) @function)
(class_declaration name: (_) @type)
(lexical_declaration (variable_declarator name: (identifier) @function value: (arrow_function)))
(import_statement source: (string (string_fragment) @import))
(export_statement source: (string (string_fragment) @import))
"#;

const PY_QUERY: &str = r#"
(function_definition name: (identifier) @function)
(class_definition name: (identifier) @type)
(import_statement name: (dotted_name) @import)
(import_statement name: (aliased_import name: (dotted_name) @import))
(import_from_statement module_name: (dotted_name) @import)
(import_from_statement module_name: (relative_import) @import)
"#;

const GO_QUERY: &str = r#"
(function_declaration name: (identifier) @function)
(method_declaration name: (field_identifier) @function)
(type_spec name: (type_identifier) @type)
(import_spec path: (interpreted_string_literal) @import)
"#;

/// シンボル抽出用のクエリ。コンパイルが重いので言語ごとに 1 度だけ作る。
pub fn tags_query(lang: Lang) -> Option<&'static Query> {
    static RUST: OnceLock<Query> = OnceLock::new();
    static TS: OnceLock<Query> = OnceLock::new();
    static TSX: OnceLock<Query> = OnceLock::new();
    static JS: OnceLock<Query> = OnceLock::new();
    static PY: OnceLock<Query> = OnceLock::new();
    static GO: OnceLock<Query> = OnceLock::new();
    let (cell, source) = match lang {
        Lang::Rust => (&RUST, RUST_QUERY),
        Lang::TypeScript => (&TS, TS_QUERY),
        Lang::Tsx => (&TSX, TS_QUERY),
        Lang::JavaScript => (&JS, JS_QUERY),
        Lang::Python => (&PY, PY_QUERY),
        Lang::Go => (&GO, GO_QUERY),
        Lang::Other => return None,
    };
    let language = language(lang)?;
    Some(cell.get_or_init(|| Query::new(&language, source).expect("組み込みクエリが不正です")))
}
