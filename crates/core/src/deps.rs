//! import 文字列をリポジトリ内のファイルに解決する。
use crate::model::Lang;
use std::collections::HashSet;

/// 解決できないもの（外部パッケージなど）は無視する。重複は除き、見つかった順に返す。
pub fn resolve_imports(
    from: &str,
    lang: Lang,
    imports: &[String],
    all: &HashSet<String>,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut push = |found: Vec<String>| {
        for f in found {
            if f != from && !out.contains(&f) {
                out.push(f);
            }
        }
    };
    for import in imports {
        push(match lang {
            Lang::TypeScript | Lang::Tsx | Lang::JavaScript => resolve_js(from, import, all),
            Lang::Rust => resolve_rust(from, import, all),
            Lang::Python => resolve_python(from, import, all),
            Lang::Go => resolve_go(import, all),
            Lang::Other => vec![],
        });
    }
    out
}

fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(d, _)| d)
}

/// `.` と `..` を畳む。リポジトリの外へ出るなら None。
fn normalize(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

fn first_existing<I: IntoIterator<Item = String>>(cands: I, all: &HashSet<String>) -> Vec<String> {
    cands.into_iter().find(|c| all.contains(c)).into_iter().collect()
}

fn resolve_js(from: &str, import: &str, all: &HashSet<String>) -> Vec<String> {
    if !import.starts_with('.') {
        return vec![];
    }
    let Some(base) = normalize(&format!("{}/{}", dir_of(from), import)) else { return vec![] };
    // TS では './a.js' と書いて a.ts を指すことがある
    let stem = base.strip_suffix(".js").unwrap_or(&base).to_string();
    const SUFFIXES: [&str; 9] = [
        "",
        ".ts",
        ".tsx",
        ".js",
        ".jsx",
        ".mjs",
        "/index.ts",
        "/index.tsx",
        "/index.js",
    ];
    let cands = SUFFIXES
        .iter()
        .map(|s| format!("{base}{s}"))
        .chain(SUFFIXES.iter().map(|s| format!("{stem}{s}")));
    first_existing(cands, all)
}

fn src_root(from: &str) -> String {
    match from.rfind("src/") {
        Some(i) => from[..i + 3].to_string(),
        None => dir_of(from).to_string(),
    }
}

fn join(a: &str, b: &str) -> String {
    if a.is_empty() { b.to_string() } else { format!("{a}/{b}") }
}

fn resolve_rust(from: &str, import: &str, all: &HashSet<String>) -> Vec<String> {
    if let Some(name) = import.strip_prefix("mod:") {
        let dir = dir_of(from);
        return first_existing(
            [join(dir, &format!("{name}.rs")), join(dir, &format!("{name}/mod.rs"))],
            all,
        );
    }
    expand_use(import)
        .iter()
        .flat_map(|path| {
            let segs: Vec<&str> = path.split("::").collect();
            let (base, rest): (String, &[&str]) = match segs.first().copied() {
                Some("crate") => (src_root(from), &segs[1..]),
                Some("self") => (dir_of(from).to_string(), &segs[1..]),
                Some("super") => {
                    let d = dir_of(from);
                    (dir_of(d).to_string(), &segs[1..])
                }
                _ => return vec![],
            };
            // 長い順に試す。末尾はファイル内の項目かもしれない
            (1..=rest.len())
                .rev()
                .find_map(|n| {
                    let joined = rest[..n].join("/");
                    let p = join(&base, &joined);
                    let hit =
                        first_existing([format!("{p}.rs"), format!("{p}/mod.rs")], all);
                    (!hit.is_empty()).then_some(hit)
                })
                .unwrap_or_default()
        })
        .collect()
}

/// `crate::a::{b, c::d}` を `crate::a::b` と `crate::a::c::d` に展開する。
fn expand_use(path: &str) -> Vec<String> {
    let path: String = path.chars().filter(|c| !c.is_whitespace()).collect();
    let Some(open) = path.find('{') else {
        return vec![path.split("as").next().unwrap_or(&path).to_string()];
    };
    let Some(close) = path.rfind('}') else { return vec![] };
    let prefix = &path[..open];
    let mut items = Vec::new();
    let (mut depth, mut start) = (0, open + 1);
    for (i, c) in path.char_indices().take(close).skip(open + 1) {
        match c {
            '{' => depth += 1,
            '}' => depth -= 1,
            ',' if depth == 0 => {
                items.push(&path[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    items.push(&path[start..close]);
    items
        .into_iter()
        .filter(|i| !i.is_empty())
        .flat_map(|i| {
            let full = if i == "self" {
                prefix.trim_end_matches("::").to_string()
            } else {
                format!("{prefix}{i}")
            };
            expand_use(&full)
        })
        .collect()
}

fn resolve_python(from: &str, import: &str, all: &HashSet<String>) -> Vec<String> {
    let dots = import.chars().take_while(|c| *c == '.').count();
    let rest = import[dots..].replace('.', "/");
    let base = if dots == 0 {
        rest
    } else {
        let mut dir = dir_of(from).to_string();
        for _ in 1..dots {
            dir = dir_of(&dir).to_string();
        }
        join(&dir, &rest)
    };
    if base.is_empty() {
        return vec![];
    }
    let found = first_existing([format!("{base}.py"), format!("{base}/__init__.py")], all);
    if !found.is_empty() || dots > 0 {
        return found;
    }
    // src/ 配下のパッケージも拾う
    let suffix_file = format!("/{base}.py");
    let suffix_init = format!("/{base}/__init__.py");
    let mut hits: Vec<String> =
        all.iter().filter(|p| p.ends_with(&suffix_file) || p.ends_with(&suffix_init)).cloned().collect();
    hits.sort();
    hits.truncate(1);
    hits
}

fn resolve_go(import: &str, all: &HashSet<String>) -> Vec<String> {
    let segs: Vec<&str> = import.split('/').collect();
    for skip in 0..segs.len() {
        let dir = segs[skip..].join("/");
        let mut hits: Vec<String> = all
            .iter()
            .filter(|p| p.ends_with(".go") && !p.ends_with("_test.go") && dir_of(p) == dir)
            .cloned()
            .collect();
        if !hits.is_empty() {
            hits.sort();
            return hits;
        }
    }
    vec![]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn resolves_relative_ts() {
        let all = set(&["src/a.ts", "src/b/index.ts", "src/c.tsx", "src/d.ts"]);
        let got = resolve_imports(
            "src/x.ts",
            Lang::TypeScript,
            &["./a".into(), "./b".into(), "./c".into(), "react".into(), "./d.js".into()],
            &all,
        );
        assert_eq!(got, ["src/a.ts", "src/b/index.ts", "src/c.tsx", "src/d.ts"]);
    }

    #[test]
    fn js_import_cannot_escape_the_repo() {
        let all = set(&["a.ts"]);
        assert!(resolve_imports("x.ts", Lang::TypeScript, &["../../a".into()], &all).is_empty());
    }

    #[test]
    fn resolves_rust_crate_paths_and_mods() {
        let all = set(&["src/a/b.rs", "src/c.rs", "src/d/mod.rs"]);
        let got = resolve_imports(
            "src/lib.rs",
            Lang::Rust,
            &["crate::a::b::Thing".into(), "mod:c".into(), "mod:d".into(), "std::fs".into()],
            &all,
        );
        assert_eq!(got, ["src/a/b.rs", "src/c.rs", "src/d/mod.rs"]);
    }

    #[test]
    fn resolves_rust_use_lists_and_super() {
        let all = set(&["src/a.rs", "src/b/c.rs", "src/b/d.rs"]);
        let got = resolve_imports(
            "src/b/e.rs",
            Lang::Rust,
            &["crate::{a, b::{c, d::X}}".into(), "super::a::Y".into()],
            &all,
        );
        assert_eq!(got, ["src/a.rs", "src/b/c.rs", "src/b/d.rs"]);
    }

    #[test]
    fn resolves_python_and_go_by_suffix() {
        let all = set(&["pkg/a/b.py", "x/y/z.go", "x/y/z_test.go", "pkg/a/c.py"]);
        assert_eq!(
            resolve_imports("m.py", Lang::Python, &["pkg.a.b".into()], &all),
            ["pkg/a/b.py"]
        );
        assert_eq!(
            resolve_imports("pkg/a/m.py", Lang::Python, &[".c".into()], &all),
            ["pkg/a/c.py"]
        );
        assert_eq!(
            resolve_imports("m.go", Lang::Go, &["example.com/x/y".into()], &all),
            ["x/y/z.go"]
        );
    }
}
