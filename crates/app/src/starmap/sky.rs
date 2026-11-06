//! インデックスとモジュール分割から、描画用のデータを作る。
use ccv_core::grouping::assign_files;
use ccv_core::layout::{Star, layout};
use ccv_core::model::{Module, RepoIndex};
use gpui::SharedString;
use std::collections::HashMap;

/// 星座の形を作る星の数。全部をつなぐと毛玉になる。
const FIGURE_STARS: usize = 10;

/// 全景で名前を出す価値のないファイル。
fn is_noise(path: &str, binary: bool) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    binary
        || name.ends_with(".lock")
        || matches!(name, "package-lock.json" | "pnpm-lock.yaml" | "go.sum" | ".gitignore" | ".DS_Store")
        || name.starts_with("LICENSE")
        || [".png", ".jpg", ".jpeg", ".gif", ".svg", ".ico", ".icns", ".webp", ".snap", ".map"]
            .iter()
            .any(|ext| name.ends_with(ext))
}

/// AI が割り当てなかったファイルを、同じディレクトリか最上位ディレクトリの多数派に寄せる。
/// 受け皿の星座が最大になると、解析の結果が失敗に見えるため。
pub fn absorb_unassigned(paths: &[&str], assignment: &mut [usize], modules: usize) {
    let parent = |p: &str, depth: Option<usize>| -> String {
        let segs: Vec<&str> = p.split('/').collect();
        let keep = depth.unwrap_or(segs.len().saturating_sub(1)).min(segs.len().saturating_sub(1));
        segs[..keep].join("/")
    };
    for depth in [None, Some(1)] {
        let mut votes: HashMap<String, HashMap<usize, u32>> = HashMap::new();
        for (p, m) in paths.iter().zip(assignment.iter()) {
            if *m < modules {
                *votes.entry(parent(p, depth)).or_default().entry(*m).or_default() += 1;
            }
        }
        for (p, m) in paths.iter().zip(assignment.iter_mut()) {
            let dir = parent(p, depth);
            if *m >= modules && !dir.is_empty()
                && let Some(counts) = votes.get(&dir)
                && let Some((best, _)) = counts.iter().max_by_key(|(id, n)| (**n, std::cmp::Reverse(**id)))
            {
                *m = *best;
            }
        }
    }
}

pub struct Constellation {
    pub name: SharedString,
    pub center: (f32, f32),
    pub extent: f32,
    pub file_count: usize,
    pub lines: u64,
    /// `None` は AI が割り当てなかった「その他」
    pub module_index: Option<usize>,
}

pub struct Sky {
    pub stars: Vec<Star>,
    pub constellations: Vec<Constellation>,
    /// 星の添字の組。星座の形を示す淡い線
    pub figure_lines: Vec<(u32, u32)>,
    /// 星座の添字の組と依存の本数
    pub module_links: Vec<(u32, u32, u32)>,
    /// ファイル単位の依存
    pub edges: Vec<(u32, u32)>,
    pub labels: Vec<SharedString>,
    /// 星座の中の順位。ラベルの間引きに使う
    pub rank: Vec<u32>,
    /// 星座の形を作る星。ほかは背景の塵として淡く描く
    pub figure: Vec<bool>,
    /// 名前を出す価値のある星
    pub labeled: Vec<bool>,
    pub bounds: (f32, f32, f32, f32),
}

pub fn build_sky(index: &RepoIndex, modules: &[Module]) -> Sky {
    let mut assignment = assign_files(&index.files, modules);
    let paths: Vec<&str> = index.files.iter().map(|f| f.path.as_str()).collect();
    absorb_unassigned(&paths, &mut assignment, modules.len());
    let stars = layout(&index.files, &assignment, modules.len());

    let slots = modules.len() + 1;
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); slots];
    for (i, s) in stars.iter().enumerate() {
        members[s.module].push(i);
    }

    let mut constellations = Vec::new();
    let mut slot_to_constellation = vec![u32::MAX; slots];
    let mut figure_lines = Vec::new();
    let mut rank = vec![u32::MAX; stars.len()];
    let mut figure_flags = vec![false; stars.len()];
    let labeled: Vec<bool> =
        index.files.iter().map(|f| !is_noise(&f.path, f.binary)).collect();
    let mut bounds = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);

    for (slot, list) in members.iter_mut().enumerate() {
        if list.is_empty() {
            continue;
        }
        // 名前を出す価値のあるファイルから順に、形と順位を決める
        list.sort_by(|a, b| {
            labeled[*b]
                .cmp(&labeled[*a])
                .then(stars[*b].radius.total_cmp(&stars[*a].radius))
                .then_with(|| index.files[*a].path.cmp(&index.files[*b].path))
        });
        let n = list.len() as f32;
        let cx = list.iter().map(|i| stars[*i].x).sum::<f32>() / n;
        let cy = list.iter().map(|i| stars[*i].y).sum::<f32>() / n;
        let mut extent = 0.0f32;
        for (r, i) in list.iter().enumerate() {
            let s = stars[*i];
            extent = extent.max(((s.x - cx).powi(2) + (s.y - cy).powi(2)).sqrt());
            rank[*i] = r as u32;
            bounds = (bounds.0.min(s.x), bounds.1.min(s.y), bounds.2.max(s.x), bounds.3.max(s.y));
        }
        // 各星を、自分より大きく最も近い星へつなぐ
        let figure = &list[..list.len().min(FIGURE_STARS)];
        for i in figure {
            figure_flags[*i] = true;
        }
        for k in 1..figure.len() {
            let a = stars[figure[k]];
            let nearest = figure[..k]
                .iter()
                .min_by(|p, q| {
                    let d = |i: &&usize| {
                        (stars[**i].x - a.x).powi(2) + (stars[**i].y - a.y).powi(2)
                    };
                    d(p).total_cmp(&d(q))
                })
                .copied();
            if let Some(b) = nearest {
                figure_lines.push((figure[k] as u32, b as u32));
            }
        }
        slot_to_constellation[slot] = constellations.len() as u32;
        constellations.push(Constellation {
            name: modules.get(slot).map_or("未分類".into(), |m| m.name.clone().into()),
            center: (cx, cy),
            extent,
            file_count: list.len(),
            lines: list.iter().map(|i| index.files[*i].lines as u64).sum(),
            module_index: (slot < modules.len()).then_some(slot),
        });
    }
    if stars.is_empty() {
        bounds = (-100.0, -100.0, 100.0, 100.0);
    }

    let mut link_counts: HashMap<(u32, u32), u32> = HashMap::new();
    for (from, to) in &index.edges {
        let a = slot_to_constellation[stars[*from as usize].module];
        let b = slot_to_constellation[stars[*to as usize].module];
        if a != b {
            *link_counts.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    let mut module_links: Vec<(u32, u32, u32)> =
        link_counts.into_iter().map(|((a, b), n)| (a, b, n)).collect();
    // 全部を描くと毛玉になる。太い順に、星座の数の 3 倍までに絞る
    module_links.sort_by_key(|l| (std::cmp::Reverse(l.2), l.0, l.1));
    module_links.truncate(constellations.len() * 3);

    // 同じ名前が複数あれば、親のディレクトリを添えて見分けられるようにする
    let base = |p: &str| p.rsplit('/').next().unwrap_or(p).to_string();
    let mut name_count: HashMap<String, u32> = HashMap::new();
    for f in &index.files {
        *name_count.entry(base(&f.path)).or_default() += 1;
    }
    let labels = index
        .files
        .iter()
        .map(|f| {
            let name = base(&f.path);
            if name_count[&name] > 1 {
                let mut segs = f.path.rsplit('/');
                let (file, dir) = (segs.next().unwrap_or(""), segs.next());
                SharedString::from(dir.map_or(name.clone(), |d| format!("{d}/{file}")))
            } else {
                SharedString::from(name)
            }
        })
        .collect();

    Sky { stars, constellations, figure_lines, module_links, edges: index.edges.clone(), labels, rank, figure: figure_flags, labeled, bounds }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ccv_core::model::{FileEntry, Lang};

    fn file(p: &str, lines: u32) -> FileEntry {
        FileEntry { path: p.into(), lang: Lang::Rust, lines, bytes: 0, binary: false, churn: 0 }
    }

    fn module(p: &str) -> Module {
        Module { id: p.into(), name: p.into(), summary: String::new(), paths: vec![p.into()] }
    }

    #[test]
    fn links_modules_and_collects_unassigned_files() {
        let index = RepoIndex {
            head: None,
            files: vec![file("a/x.rs", 10), file("a/y.rs", 20), file("b/z.rs", 30), file("c/w.rs", 40)],
            symbols: Default::default(),
            edges: vec![(0, 2), (1, 2), (0, 1)],
        };
        let sky = build_sky(&index, &[module("a"), module("b")]);
        assert_eq!(sky.constellations.len(), 3);
        assert_eq!(sky.constellations[2].name, "未分類");
        assert_eq!(sky.constellations[2].module_index, None);
        // 同じ星座の中の依存は星座間のリンクにならない
        assert_eq!(sky.module_links, [(0, 1, 2)]);
        assert_eq!(sky.figure_lines.len(), 1);
        assert_eq!(sky.rank[1], 0);
    }

    #[test]
    fn unassigned_files_join_the_majority_of_their_directory() {
        let paths = ["a/b/x.rs", "a/b/y.rs", "a/b/z.rs", "a/c/w.rs", "q/r.rs", "top.txt"];
        let mut assignment = [1, 1, 9, 9, 9, 9];
        absorb_unassigned(&paths, &mut assignment, 2);
        assert_eq!(assignment, [1, 1, 1, 1, 9, 9]);
    }

    #[test]
    fn noise_is_not_labeled_and_duplicate_names_show_their_parent() {
        let files = vec![file("a/index.ts", 10), file("b/index.ts", 10), file("a/bun.lock", 9000)];
        let index = RepoIndex { files, ..Default::default() };
        let sky = build_sky(&index, &[module("a"), module("b")]);
        assert_eq!(sky.labels[0], "a/index.ts");
        assert_eq!(sky.labels[2], "bun.lock");
        assert_eq!(sky.labeled, [true, true, false]);
        // 大きくても、ロックファイルは星座の主役にしない
        assert_eq!(sky.rank[2], 1);
    }

    #[test]
    fn only_the_strongest_links_are_kept() {
        let files: Vec<_> = (0..8).map(|i| file(&format!("m{i}/f.rs"), 1)).collect();
        let mut edges = Vec::new();
        for a in 0..8u32 {
            for b in 0..8u32 {
                if a != b {
                    edges.push((a, b));
                }
            }
        }
        edges.extend([(0, 1); 5]);
        let modules: Vec<_> = (0..8).map(|i| module(&format!("m{i}"))).collect();
        let index = RepoIndex { files, edges, ..Default::default() };
        let sky = build_sky(&index, &modules);
        assert_eq!(sky.module_links.len(), 24);
        assert_eq!(sky.module_links[0], (0, 1, 7));
    }

    #[test]
    fn figure_lines_are_limited_and_empty_repo_is_safe() {
        let files: Vec<_> = (0..100).map(|i| file(&format!("a/{i}.rs"), i + 1)).collect();
        let index = RepoIndex { files, ..Default::default() };
        let sky = build_sky(&index, &[module("a")]);
        assert_eq!(sky.figure_lines.len(), FIGURE_STARS - 1);
        let empty = build_sky(&RepoIndex::default(), &[]);
        assert!(empty.stars.is_empty() && empty.bounds.2 > empty.bounds.0);
    }
}
