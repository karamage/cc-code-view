//! ファイルツリー。索引のパスから組み立て、開いているフォルダーに応じて行に並べる。
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    /// 表示名。子が一つだけのフォルダーは `src/server` のようにまとめる
    pub name: String,
    /// リポジトリ相対のパス。フォルダーはまとめた末尾のフォルダーを指す
    pub path: String,
    /// ファイルなら索引の添字
    pub file: Option<usize>,
    pub children: Vec<usize>,
}

impl Node {
    pub fn is_dir(&self) -> bool {
        self.file.is_none()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Row {
    pub node: usize,
    pub depth: u16,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileTree {
    pub nodes: Vec<Node>,
    pub roots: Vec<usize>,
}

#[derive(Default)]
struct Draft {
    dirs: BTreeMap<String, Draft>,
    files: Vec<(String, usize)>,
}

fn sort_key(name: &str) -> (String, String) {
    (name.to_lowercase(), name.to_string())
}

impl FileTree {
    /// パスは `/` 区切りのリポジトリ相対。並びはフォルダーが先、名前は大文字小文字を区別しない。
    pub fn build<'a>(paths: impl IntoIterator<Item = &'a str>) -> FileTree {
        let mut root = Draft::default();
        for (i, path) in paths.into_iter().enumerate() {
            let mut parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
            let Some(name) = parts.pop() else { continue };
            let mut at = &mut root;
            for dir in parts {
                at = at.dirs.entry(dir.to_string()).or_default();
            }
            at.files.push((name.to_string(), i));
        }
        let mut tree = FileTree::default();
        tree.roots = tree.add_children(root, "");
        tree
    }

    fn add_children(&mut self, draft: Draft, prefix: &str) -> Vec<usize> {
        let mut dirs: Vec<(String, Draft)> = draft.dirs.into_iter().collect();
        dirs.sort_by_key(|(name, _)| sort_key(name));
        let mut files = draft.files;
        files.sort_by_key(|(name, _)| sort_key(name));
        let mut out = Vec::new();
        for (name, mut sub) in dirs {
            let mut label = name.clone();
            let mut path = join(prefix, &name);
            // ファイルを持たず、子のフォルダーが一つだけなら一行にまとめる
            while sub.files.is_empty() && sub.dirs.len() == 1 {
                let (child, next) = sub.dirs.pop_first().expect("一つある");
                label = format!("{label}/{child}");
                path = join(&path, &child);
                sub = next;
            }
            let id = self.nodes.len();
            self.nodes.push(Node { name: label, path: path.clone(), file: None, children: Vec::new() });
            let children = self.add_children(sub, &path);
            self.nodes[id].children = children;
            out.push(id);
        }
        for (name, file) in files {
            let id = self.nodes.len();
            self.nodes.push(Node {
                path: join(prefix, &name),
                name,
                file: Some(file),
                children: Vec::new(),
            });
            out.push(id);
        }
        out
    }

    /// 開いているフォルダーの中身だけを、上から順に並べる。
    pub fn rows(&self, expanded: &HashSet<String>) -> Vec<Row> {
        let mut out = Vec::new();
        let mut stack: Vec<(usize, u16)> = self.roots.iter().rev().map(|&n| (n, 0)).collect();
        while let Some((n, depth)) = stack.pop() {
            out.push(Row { node: n, depth });
            let node = &self.nodes[n];
            if node.is_dir() && expanded.contains(&node.path) {
                stack.extend(node.children.iter().rev().map(|&c| (c, depth + 1)));
            }
        }
        out
    }

    /// ファイルを見せるために開くフォルダーのパス。外側から順に返す。
    pub fn ancestors(&self, file_path: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut level = &self.roots;
        'walk: loop {
            for &n in level {
                let node = &self.nodes[n];
                if node.is_dir() && file_path.starts_with(&format!("{}/", node.path)) {
                    out.push(node.path.clone());
                    level = &node.children;
                    continue 'walk;
                }
            }
            return out;
        }
    }

    /// 最初に開いておくフォルダー。上の階層だけを開き、一覧を長くしすぎない。
    pub fn initial_expanded(&self) -> HashSet<String> {
        let mut out = HashSet::new();
        let dirs: Vec<usize> = self.roots.iter().copied().filter(|&n| self.nodes[n].is_dir()).collect();
        // 根にフォルダーが一つだけなら、その中身まで見せる
        if dirs.len() <= 2 {
            for n in dirs {
                out.insert(self.nodes[n].path.clone());
            }
        }
        out
    }
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() { name.to_string() } else { format!("{prefix}/{name}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(tree: &FileTree, rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|r| format!("{}{}", "  ".repeat(r.depth as usize), tree.nodes[r.node].name))
            .collect()
    }

    #[test]
    fn dirs_first_and_case_insensitive() {
        let t = FileTree::build(["b.rs", "A.rs", "src/x.rs", "Docs/a.md", "docs2/c.md", "docs2/d/e.md"]);
        let all: HashSet<String> = t.nodes.iter().filter(|n| n.is_dir()).map(|n| n.path.clone()).collect();
        assert_eq!(
            names(&t, &t.rows(&all)),
            ["Docs", "  a.md", "docs2", "  d", "    e.md", "  c.md", "src", "  x.rs", "A.rs", "b.rs"]
        );
    }

    #[test]
    fn single_child_chains_collapse() {
        let t = FileTree::build(["crates/app/src/main.rs", "crates/app/src/code/mod.rs", "README.md"]);
        let closed = t.rows(&HashSet::new());
        assert_eq!(names(&t, &closed), ["crates/app/src", "README.md"]);
        let n = closed[0].node;
        assert_eq!(t.nodes[n].path, "crates/app/src");
        assert_eq!(t.ancestors("crates/app/src/code/mod.rs"), ["crates/app/src", "crates/app/src/code"]);
    }

    #[test]
    fn file_indices_point_into_input() {
        let t = FileTree::build(["z/a.rs", "b.rs"]);
        let files: Vec<(String, usize)> =
            t.nodes.iter().filter_map(|n| n.file.map(|f| (n.path.clone(), f))).collect();
        assert!(files.contains(&("z/a.rs".to_string(), 0)));
        assert!(files.contains(&("b.rs".to_string(), 1)));
    }

    #[test]
    fn ancestors_of_root_file_is_empty() {
        let t = FileTree::build(["a.rs", "src/b.rs"]);
        assert!(t.ancestors("a.rs").is_empty());
        assert_eq!(t.ancestors("src/b.rs"), ["src"]);
    }
}
