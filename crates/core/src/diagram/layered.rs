//! 階層型の配置。フローチャート、クラス図、ER 図、状態遷移図に使う。
//!
//! 1. 閉路を切る 2. 階層を決める 3. 長い線に中継点を入れる
//! 4. 交差が減るよう並べ替える（囲みは連続させる） 5. 横位置を二次の最適化で決める
//! 6. 囲み同士の重なりを押し出す 7. 線を 3 次ベジェで結ぶ
//!
//! 配置は常に上から下の向きで計算し、最後に向きを変える。
// 層と添字を行き来する計算なので、添字のループのほうが読みやすい
#![allow(clippy::needless_range_loop)]
use super::ast::{Dir, Graph, Line, Shape};
use super::scene::bezier_point;
use super::scene::*;

const MARGIN: f32 = 18.0;
const NODE_GAP: f32 = 34.0;
const DUMMY_GAP: f32 = 12.0;
const CLUSTER_PAD: f32 = 16.0;
const CLUSTER_TITLE: f32 = 22.0;
const LABEL_WRAP: f32 = 150.0;
const TABLE_PAD: f32 = 12.0;
const COLUMN_GAP: f32 = 14.0;

struct Vertex {
    /// 上から下の向きでの幅と高さ
    w: f32,
    h: f32,
    real: Option<usize>,
    rank: usize,
    x: f32,
}

struct Chain {
    edge: usize,
    vertices: Vec<usize>,
    /// ラベルを置く区間（何番目の中継点の後ろか）
    label_segment: Option<usize>,
}

struct Sized {
    w: f32,
    h: f32,
    title: Vec<String>,
    sections: Vec<Vec<Vec<String>>>,
    columns: Vec<f32>,
}

fn size_node(g: &Graph, i: usize, m: &dyn Measure) -> Sized {
    let n = &g.nodes[i];
    let lh = line_height(NODE_TEXT);
    match n.shape {
        Shape::Start => {
            return Sized { w: 16.0, h: 16.0, title: vec![], sections: vec![], columns: vec![] };
        }
        Shape::End => {
            return Sized { w: 20.0, h: 20.0, title: vec![], sections: vec![], columns: vec![] };
        }
        Shape::Bar => {
            let (w, h) = if g.dir.horizontal() { (8.0, 64.0) } else { (64.0, 8.0) };
            return Sized { w, h, title: vec![], sections: vec![], columns: vec![] };
        }
        _ => {}
    }
    if n.shape == Shape::Table {
        let title = m.wrap(&n.label, NODE_TEXT, 260.0, true);
        let mut width =
            title.iter().map(|l| m.width(l, NODE_TEXT, false, true)).fold(0.0, f32::max);
        if let Some(s) = &n.stereotype {
            width = width.max(m.width(&format!("«{s}»"), SMALL_TEXT, false, false));
        }
        let sections: Vec<Vec<Vec<String>>> = n
            .sections
            .iter()
            .map(|s| s.iter().map(|row| row.split('\t').map(str::to_string).collect()).collect())
            .collect();
        let cols = sections.iter().flatten().map(Vec::len).max().unwrap_or(0);
        let mut col_w = vec![0.0f32; cols];
        for row in sections.iter().flatten() {
            for (c, cell) in row.iter().enumerate() {
                col_w[c] = col_w[c].max(m.width(cell, MONO_TEXT, true, false));
            }
        }
        let mut columns = Vec::with_capacity(cols);
        let mut x = TABLE_PAD;
        for w in &col_w {
            columns.push(x);
            x += w + COLUMN_GAP;
        }
        let rows_w = (x - COLUMN_GAP - TABLE_PAD).max(0.0);
        width = width.max(rows_w).max(116.0) + TABLE_PAD * 2.0;
        let row_h = line_height(MONO_TEXT) + 3.0;
        let mut h = 10.0 + title.len() as f32 * lh + 10.0;
        if n.stereotype.is_some() {
            h += line_height(SMALL_TEXT);
        }
        for s in sections.iter().filter(|s| !s.is_empty()) {
            h += 7.0 + s.len() as f32 * row_h + 7.0;
        }
        return Sized { w: width.ceil(), h: h.ceil(), title, sections, columns };
    }
    let max = if g.dir.horizontal() { 176.0 } else { 196.0 };
    let title = m.wrap(&n.label, NODE_TEXT, max, false);
    let tw = title.iter().map(|l| m.width(l, NODE_TEXT, false, false)).fold(0.0, f32::max);
    let th = title.len() as f32 * lh;
    let (mut w, mut h) = (tw + 32.0, th + 20.0);
    match n.shape {
        Shape::Diamond => {
            w = tw * 1.4 + 40.0;
            h = th * 1.4 + 34.0;
            w = w.max(h * 1.25);
        }
        Shape::Circle => {
            let d = (tw + 28.0).max(th + 28.0);
            w = d;
            h = d;
        }
        Shape::Hexagon => w += h * 0.5,
        Shape::Cylinder => h += 16.0,
        Shape::Stadium => w += h * 0.4,
        Shape::Subroutine => w += 16.0,
        Shape::Parallelogram | Shape::Flag => w += 22.0,
        _ => {}
    }
    Sized { w: w.max(56.0).ceil(), h: h.max(38.0).ceil(), title, sections: vec![], columns: vec![] }
}

/// 線の終わりを、箱に入る手前で切る。
fn clip_end(path: &[(f32, f32)], b: &Rect) -> Vec<(f32, f32)> {
    let segments = (path.len() - 1) / 3;
    if segments == 0 {
        return path.to_vec();
    }
    const STEPS: usize = 240;
    for k in 0..=STEPS {
        let t = k as f32 / STEPS as f32;
        let whole = ((t * segments as f32).floor() as usize).min(segments - 1);
        let local = t * segments as f32 - whole as f32;
        let i = whole * 3;
        let p = bezier_point(path[i], path[i + 1], path[i + 2], path[i + 3], local);
        if b.contains(p.0, p.1) {
            return if k == 0 { path.to_vec() } else { trim_path(path, t) };
        }
    }
    path.to_vec()
}

/// 囲みの祖先を根から並べる。
fn cluster_path(g: &Graph, c: Option<usize>) -> Vec<usize> {
    let mut path = Vec::new();
    let mut cur = c;
    while let Some(i) = cur {
        if path.contains(&i) {
            break;
        }
        path.push(i);
        cur = g.clusters[i].parent;
    }
    path.reverse();
    path
}

fn common_cluster(a: &[usize], b: &[usize]) -> Option<usize> {
    a.iter().zip(b).take_while(|(x, y)| x == y).last().map(|(x, _)| *x)
}

/// 重み付きの単調回帰。並び順を保ったまま、目標の位置へできるだけ近づける。
fn pava(target: &[f32], weight: &[f32]) -> Vec<f32> {
    let mut blocks: Vec<(f32, f32, usize)> = Vec::new();
    for (t, w) in target.iter().zip(weight) {
        blocks.push((t * w, *w, 1));
        while blocks.len() >= 2 {
            let (a, b) = (blocks[blocks.len() - 2], blocks[blocks.len() - 1]);
            if a.0 / a.1 > b.0 / b.1 {
                blocks.pop();
                let last = blocks.last_mut().expect("2 つ以上ある");
                *last = (a.0 + b.0, a.1 + b.1, a.2 + b.2);
            } else {
                break;
            }
        }
    }
    blocks.into_iter().flat_map(|(s, w, n)| std::iter::repeat_n(s / w, n)).collect()
}

pub fn layout(g: &Graph, m: &dyn Measure) -> Scene {
    let horizontal = g.dir.horizontal();
    let sized: Vec<Sized> = (0..g.nodes.len()).map(|i| size_node(g, i, m)).collect();
    let paths: Vec<Vec<usize>> = g.nodes.iter().map(|n| cluster_path(g, n.cluster)).collect();
    let cluster_paths: Vec<Vec<usize>> =
        (0..g.clusters.len()).map(|c| cluster_path(g, Some(c))).collect();

    // 線のラベルの大きさ
    let labels: Vec<Option<(Vec<String>, f32, f32)>> = g
        .edges
        .iter()
        .map(|e| {
            if e.label.is_empty() || e.line == Line::Invisible {
                return None;
            }
            let lines = m.wrap(&e.label, SMALL_TEXT, LABEL_WRAP, false);
            let w = lines.iter().map(|l| m.width(l, SMALL_TEXT, false, false)).fold(0.0, f32::max)
                + 14.0;
            let h = lines.len() as f32 * line_height(SMALL_TEXT) + 6.0;
            Some((lines, w.ceil(), h.ceil()))
        })
        .collect();

    // 1. 閉路を切る。宣言順に深さ優先でたどり、戻る線を逆向きにする
    let n = g.nodes.len();
    // 下から上、右から左の図は、線の向きを逆にして同じ計算をする
    let flip = matches!(g.dir, Dir::BottomUp | Dir::RightLeft);
    let oriented: Vec<(usize, usize)> = g
        .edges
        .iter()
        .map(|e| if e.upward != flip { (e.to, e.from) } else { (e.from, e.to) })
        .collect();
    let mut out_edges: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (k, (a, b)) in oriented.iter().enumerate() {
        if a != b {
            out_edges[*a].push(k);
        }
    }
    let mut reversed = vec![false; g.edges.len()];
    let mut state = vec![0u8; n];
    for root in 0..n {
        if state[root] != 0 {
            continue;
        }
        let mut stack: Vec<(usize, usize)> = vec![(root, 0)];
        state[root] = 1;
        while let Some(&mut (v, ref mut next)) = stack.last_mut() {
            if *next < out_edges[v].len() {
                let k = out_edges[v][*next];
                *next += 1;
                let w = oriented[k].1;
                match state[w] {
                    0 => {
                        state[w] = 1;
                        stack.push((w, 0));
                    }
                    1 => reversed[k] = true,
                    _ => {}
                }
            } else {
                state[v] = 2;
                stack.pop();
            }
        }
    }
    let dag: Vec<(usize, usize, usize, usize)> = oriented
        .iter()
        .enumerate()
        .filter(|(_, (a, b))| a != b)
        .map(|(k, &(a, b))| {
            let (a, b) = if reversed[k] { (b, a) } else { (a, b) };
            (a, b, 1, k)
        })
        .collect();

    // 2. 階層。最長路で決め、入口だけの要素は行き先の近くまで下げる
    let mut indeg = vec![0usize; n];
    let mut succ: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
    let mut pred: Vec<Vec<(usize, usize)>> = vec![Vec::new(); n];
    for &(a, b, len, _) in &dag {
        indeg[b] += 1;
        succ[a].push((b, len));
        pred[b].push((a, len));
    }
    let mut topo = Vec::with_capacity(n);
    let mut queue: std::collections::VecDeque<usize> = (0..n).filter(|v| indeg[*v] == 0).collect();
    while let Some(v) = queue.pop_front() {
        topo.push(v);
        for &(w, _) in &succ[v] {
            indeg[w] -= 1;
            if indeg[w] == 0 {
                queue.push_back(w);
            }
        }
    }
    let mut rank = vec![0usize; n];
    for &v in &topo {
        for &(w, len) in &succ[v] {
            rank[w] = rank[w].max(rank[v] + len);
        }
    }
    for &v in topo.iter().rev() {
        if pred[v].is_empty() && !succ[v].is_empty() {
            rank[v] = succ[v].iter().map(|(w, len)| rank[*w] - len).min().unwrap_or(rank[v]);
        }
    }

    // 3. 中継点を入れる
    let mut vs: Vec<Vertex> = (0..n)
        .map(|i| {
            let (w, h) =
                if horizontal { (sized[i].h, sized[i].w) } else { (sized[i].w, sized[i].h) };
            Vertex { w, h, real: Some(i), rank: rank[i], x: 0.0 }
        })
        .collect();
    let mut vpaths: Vec<Vec<usize>> = paths.clone();
    let mut chains: Vec<Chain> = Vec::new();
    for &(a, b, _, k) in &dag {
        let common = common_cluster(&paths[a], &paths[b]);
        let mut vertices = vec![a];
        for r in rank[a] + 1..rank[b] {
            vs.push(Vertex { w: 0.0, h: 0.0, real: None, rank: r, x: 0.0 });
            vpaths.push(cluster_path(g, common));
            vertices.push(vs.len() - 1);
        }
        vertices.push(b);
        // ラベルは階層の間に置く。長い線なら中ほどの区間
        let label_segment = labels[k].as_ref().map(|_| (vertices.len() - 2) / 2);
        chains.push(Chain { edge: k, vertices, label_segment });
    }
    let ranks = vs.iter().map(|v| v.rank).max().map_or(0, |r| r + 1);
    let mut layers: Vec<Vec<usize>> = vec![Vec::new(); ranks];
    for (i, v) in vs.iter().enumerate() {
        layers[v.rank].push(i);
    }
    let mut up: Vec<Vec<(usize, f32)>> = vec![Vec::new(); vs.len()];
    let mut down: Vec<Vec<(usize, f32)>> = vec![Vec::new(); vs.len()];
    for c in &chains {
        for pair in c.vertices.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let weight = match (vs[a].real.is_none(), vs[b].real.is_none()) {
                (true, true) => 8.0,
                (true, false) | (false, true) => 2.0,
                _ => 1.0,
            };
            down[a].push((b, weight));
            up[b].push((a, weight));
        }
    }

    // 4. 並べ替え
    fn arrange(
        items: &[usize],
        depth: usize,
        key: &dyn Fn(usize) -> f32,
        group_key: &dyn Fn(usize) -> Option<f32>,
        vpaths: &[Vec<usize>],
    ) -> Vec<usize> {
        let mut units: Vec<(f32, Vec<usize>, bool)> = Vec::new();
        let mut group_at: std::collections::HashMap<usize, usize> = Default::default();
        let mut cluster_of: Vec<Option<usize>> = Vec::new();
        for &v in items {
            match vpaths[v].get(depth) {
                Some(&c) => match group_at.get(&c) {
                    Some(&u) => units[u].1.push(v),
                    None => {
                        group_at.insert(c, units.len());
                        units.push((0.0, vec![v], true));
                        cluster_of.push(Some(c));
                    }
                },
                None => {
                    units.push((0.0, vec![v], false));
                    cluster_of.push(None);
                }
            }
        }
        for (u, c) in units.iter_mut().zip(&cluster_of) {
            u.0 = c
                .and_then(group_key)
                .unwrap_or_else(|| u.1.iter().map(|v| key(*v)).sum::<f32>() / u.1.len() as f32);
        }
        units.sort_by(|a, b| a.0.total_cmp(&b.0));
        units
            .into_iter()
            .flat_map(|(_, members, group)| {
                if group { arrange(&members, depth + 1, key, group_key, vpaths) } else { members }
            })
            .collect()
    }
    let mut pos = vec![0.0f32; vs.len()];
    let set_positions = |layers: &Vec<Vec<usize>>, pos: &mut Vec<f32>| {
        for layer in layers {
            for (i, v) in layer.iter().enumerate() {
                pos[*v] = i as f32;
            }
        }
    };
    // 最初の並びは、囲みごとにまとめた宣言順
    for layer in &mut layers {
        let snapshot: Vec<usize> = layer.clone();
        *layer = arrange(&snapshot, 0, &|v| v as f32, &|_| None, &vpaths);
    }
    set_positions(&layers, &mut pos);
    let crossings = |layers: &Vec<Vec<usize>>, pos: &Vec<f32>| -> usize {
        let mut total = 0;
        for layer in layers.iter().take(layers.len().saturating_sub(1)) {
            let mut segs: Vec<(f32, f32)> = Vec::new();
            for &v in layer {
                for &(w, _) in &down[v] {
                    segs.push((pos[v], pos[w]));
                }
            }
            for i in 0..segs.len() {
                for j in i + 1..segs.len() {
                    let (a, b) = (segs[i], segs[j]);
                    if (a.0 - b.0) * (a.1 - b.1) < 0.0 {
                        total += 1;
                    }
                }
            }
        }
        total
    };
    let mut best = layers.clone();
    let mut best_cross = crossings(&layers, &pos);
    for iter in 0..28 {
        if best_cross == 0 {
            break;
        }
        let downward = iter % 2 == 0;
        let order: Vec<usize> = if downward {
            (1..ranks).collect()
        } else {
            (0..ranks.saturating_sub(1)).rev().collect()
        };
        for r in order {
            let neighbors = if downward { &up } else { &down };
            let snapshot = layers[r].clone();
            let current = pos.clone();
            let key = |v: usize| -> f32 {
                let ns = &neighbors[v];
                if ns.is_empty() {
                    current[v]
                } else {
                    ns.iter().map(|(w, _)| current[*w]).sum::<f32>() / ns.len() as f32
                }
            };
            layers[r] = arrange(&snapshot, 0, &key, &|_| None, &vpaths);
            for (i, v) in layers[r].iter().enumerate() {
                pos[*v] = i as f32;
            }
        }
        let c = crossings(&layers, &pos);
        if c < best_cross {
            best_cross = c;
            best = layers.clone();
        }
    }
    layers = best;
    set_positions(&layers, &mut pos);
    // 兄弟の囲みの並びを、すべての階層で同じにする。階層ごとに入れ替わると囲みが重なる
    if !g.clusters.is_empty() {
        let norm: Vec<f32> = (0..vs.len())
            .map(|v| {
                let len = layers[vs[v].rank].len().max(1) as f32;
                (pos[v] + 0.5) / len
            })
            .collect();
        let cluster_key: Vec<Option<f32>> = (0..g.clusters.len())
            .map(|c| {
                let ms: Vec<f32> =
                    (0..vs.len()).filter(|v| vpaths[*v].contains(&c)).map(|v| norm[v]).collect();
                (!ms.is_empty()).then(|| ms.iter().sum::<f32>() / ms.len() as f32)
            })
            .collect();
        for r in 0..ranks {
            let snapshot = layers[r].clone();
            layers[r] = arrange(&snapshot, 0, &|v| norm[v], &|c| cluster_key[c], &vpaths);
        }
        set_positions(&layers, &mut pos);
    }
    // 隣どうしの入れ替えで、さらに交差を減らす
    if vs.len() <= 220 {
        for _ in 0..4 {
            let mut improved = false;
            for r in 0..ranks {
                for i in 0..layers[r].len().saturating_sub(1) {
                    let (a, b) = (layers[r][i], layers[r][i + 1]);
                    if vpaths[a] != vpaths[b] {
                        continue;
                    }
                    let before = crossings(&layers, &pos);
                    layers[r].swap(i, i + 1);
                    pos[a] = (i + 1) as f32;
                    pos[b] = i as f32;
                    if crossings(&layers, &pos) < before {
                        improved = true;
                    } else {
                        layers[r].swap(i, i + 1);
                        pos[a] = i as f32;
                        pos[b] = (i + 1) as f32;
                    }
                }
            }
            if !improved {
                break;
            }
        }
    }

    // 5. 横位置
    fn gap(vs: &[Vertex], vpaths: &[Vec<usize>], horizontal: bool, a: usize, b: usize) -> f32 {
        let (va, vb) = (&vs[a], &vs[b]);
        let base = if va.real.is_none() || vb.real.is_none() { DUMMY_GAP } else { NODE_GAP };
        let common = vpaths[a].iter().zip(&vpaths[b]).take_while(|(x, y)| x == y).count();
        let exits = vpaths[a].len() - common;
        let enters = vpaths[b].len() - common;
        let title = if horizontal { CLUSTER_TITLE } else { 0.0 };
        (va.w + vb.w) / 2.0 + base + (exits + enters) as f32 * CLUSTER_PAD + enters as f32 * title
    }
    let mut widest = 0.0f32;
    for layer in &layers {
        let mut x = 0.0;
        for (i, &v) in layer.iter().enumerate() {
            if i > 0 {
                x += gap(&vs, &vpaths, horizontal, layer[i - 1], v);
            }
            vs[v].x = x;
        }
        widest = widest.max(x);
    }
    for layer in &layers {
        let span = layer.last().map_or(0.0, |v| vs[*v].x);
        let shift = (widest - span) / 2.0;
        for &v in layer {
            vs[v].x += shift;
        }
    }
    let place = |vs: &mut Vec<Vertex>, layer: &[usize], use_up: bool, use_down: bool| {
        if layer.is_empty() {
            return;
        }
        let mut target = Vec::with_capacity(layer.len());
        let mut weight = Vec::with_capacity(layer.len());
        for &v in layer {
            let mut sum = 0.0;
            let mut wsum = 0.0;
            if use_up {
                for &(w, k) in &up[v] {
                    sum += vs[w].x * k;
                    wsum += k;
                }
            }
            if use_down {
                for &(w, k) in &down[v] {
                    sum += vs[w].x * k;
                    wsum += k;
                }
            }
            if wsum == 0.0 {
                target.push(vs[v].x);
                weight.push(0.3);
            } else {
                target.push(sum / wsum);
                weight.push(wsum);
            }
        }
        let mut offsets = vec![0.0; layer.len()];
        for i in 1..layer.len() {
            offsets[i] = offsets[i - 1] + gap(vs, &vpaths, horizontal, layer[i - 1], layer[i]);
        }
        let shifted: Vec<f32> = target.iter().zip(&offsets).map(|(t, o)| t - o).collect();
        let solved = pava(&shifted, &weight);
        for (i, &v) in layer.iter().enumerate() {
            vs[v].x = solved[i] + offsets[i];
        }
    };
    for iter in 0..14 {
        if iter % 2 == 0 {
            for r in 1..ranks {
                place(&mut vs, &layers[r], true, false);
            }
        } else {
            for r in (0..ranks.saturating_sub(1)).rev() {
                place(&mut vs, &layers[r], false, true);
            }
        }
    }
    for _ in 0..4 {
        for layer in &layers {
            place(&mut vs, layer, true, true);
        }
    }

    // 6. 囲みの重なりを右へ押し出す
    let members: Vec<Vec<usize>> = (0..g.clusters.len())
        .map(|c| (0..vs.len()).filter(|v| vpaths[*v].contains(&c)).collect())
        .collect();
    let depth_below = |c: usize| -> usize {
        cluster_paths
            .iter()
            .filter_map(|p| p.iter().position(|x| *x == c).map(|at| p.len() - 1 - at))
            .max()
            .unwrap_or(0)
    };
    let title_w: Vec<f32> =
        g.clusters.iter().map(|c| m.width(&c.label, SMALL_TEXT, false, true) + 28.0).collect();
    let cluster_x = |vs: &Vec<Vertex>, c: usize| -> Option<(f32, f32, usize, usize)> {
        let ms = &members[c];
        if ms.is_empty() {
            return None;
        }
        let levels = (depth_below(c) + 1) as f32;
        let pad = CLUSTER_PAD * levels;
        let title = if horizontal { CLUSTER_TITLE * levels } else { 0.0 };
        let left =
            ms.iter().map(|v| vs[*v].x - vs[*v].w / 2.0).fold(f32::MAX, f32::min) - pad - title;
        let mut right = ms.iter().map(|v| vs[*v].x + vs[*v].w / 2.0).fold(f32::MIN, f32::max) + pad;
        if !horizontal {
            right = right.max(left + title_w[c]);
        }
        let lo = ms.iter().map(|v| vs[*v].rank).min().unwrap_or(0);
        let hi = ms.iter().map(|v| vs[*v].rank).max().unwrap_or(0);
        Some((left, right, lo, hi))
    };
    // 整える、押し出す、を繰り返し、囲みを詰めながら重ならないようにする
    let rounds = if g.clusters.is_empty() { 1 } else { 3 };
    for round in 0..rounds {
        if round > 0 {
            for iter in 0..6 {
                if iter % 2 == 0 {
                    for r in 1..ranks {
                        place(&mut vs, &layers[r], true, false);
                    }
                } else {
                    for r in (0..ranks.saturating_sub(1)).rev() {
                        place(&mut vs, &layers[r], false, true);
                    }
                }
            }
            for _ in 0..4 {
                for layer in &layers {
                    place(&mut vs, layer, true, true);
                }
            }
        }
        for _ in 0..600 {
            let mut moved = false;
            'clusters: for c in 0..g.clusters.len() {
                let Some((left, right, lo, hi)) = cluster_x(&vs, c) else { continue };
                for r in lo..=hi {
                    let layer = layers[r].clone();
                    let inside: Vec<usize> =
                        (0..layer.len()).filter(|i| vpaths[layer[*i]].contains(&c)).collect();
                    for (i, &v) in layer.iter().enumerate() {
                        // 祖先の囲みに直接属する要素も、外側の要素として扱う
                        if vpaths[v].contains(&c) {
                            continue;
                        }
                        let (l, rr) = (vs[v].x - vs[v].w / 2.0, vs[v].x + vs[v].w / 2.0);
                        let need = if vs[v].real.is_none() { DUMMY_GAP } else { NODE_GAP / 2.0 };
                        if rr + need <= left || l - need >= right {
                            continue;
                        }
                        let right_side = match (inside.first(), inside.last()) {
                            (Some(first), _) if i < *first => false,
                            (_, Some(last)) if i > *last => true,
                            (None, None) => vs[v].x >= (left + right) / 2.0,
                            _ => continue,
                        };
                        if right_side {
                            let delta = right + need - l + 0.5;
                            for &w in &layer[i..] {
                                vs[w].x += delta;
                            }
                        } else {
                            // 囲みのほうを右へ動かす。囲みが通るすべての階層で、囲みから右をずらす
                            let delta = rr + need - left + 0.5;
                            for rr2 in lo..=hi {
                                let l2 = &layers[rr2];
                                if let Some(first) = l2.iter().position(|w| vpaths[*w].contains(&c))
                                {
                                    for &w in &l2[first..] {
                                        vs[w].x += delta;
                                    }
                                }
                            }
                        }
                        moved = true;
                        continue 'clusters;
                    }
                }
            }
            // 兄弟の囲みどうしの重なり。右にある囲みを、その囲みが通る階層ごとずらす
            if !moved {
                'pairs: for c in 0..g.clusters.len() {
                    for d in 0..g.clusters.len() {
                        if c == d || cluster_paths[c].contains(&d) || cluster_paths[d].contains(&c)
                        {
                            continue;
                        }
                        let (Some(a), Some(b)) = (cluster_x(&vs, c), cluster_x(&vs, d)) else {
                            continue;
                        };
                        let ranks_overlap = a.2 <= b.3 && b.2 <= a.3;
                        let x_overlap =
                            a.0 < b.1 + CLUSTER_PAD / 2.0 && b.0 < a.1 + CLUSTER_PAD / 2.0;
                        if !ranks_overlap || !x_overlap || (a.0 + a.1) > (b.0 + b.1) {
                            continue;
                        }
                        // c が左、d が右。d を右へ送る
                        let delta = a.1 + CLUSTER_PAD - b.0 + 0.5;
                        if delta <= 0.0 {
                            continue;
                        }
                        for r in b.2..=b.3 {
                            let layer = &layers[r];
                            if let Some(first) = layer.iter().position(|w| vpaths[*w].contains(&d))
                            {
                                for &w in &layer[first..] {
                                    vs[w].x += delta;
                                }
                            }
                        }
                        moved = true;
                        break 'pairs;
                    }
                }
            }
            if !moved {
                break;
            }
        }
    }
    // 縦位置
    let starts_at = |r: usize| {
        (0..g.clusters.len()).filter(|c| cluster_x(&vs, *c).is_some_and(|x| x.2 == r)).count()
    };
    let ends_at = |r: usize| {
        (0..g.clusters.len()).filter(|c| cluster_x(&vs, *c).is_some_and(|x| x.3 == r)).count()
    };
    let rank_sep = if horizontal { 46.0 } else { 40.0 };
    // ラベルの入る階層の間は、ラベルの大きさだけ広げる
    let mut gap_extra = vec![0.0f32; ranks];
    for c in &chains {
        if let (Some(seg), Some((_, lw, lh))) = (c.label_segment, &labels[c.edge]) {
            let r = vs[c.vertices[seg]].rank;
            let need = if horizontal { *lw } else { *lh } + 10.0;
            gap_extra[r] = gap_extra[r].max(need);
        }
    }
    let mut rank_y = vec![0.0f32; ranks];
    let mut rank_h = vec![0.0f32; ranks];
    let mut y = 0.0;
    for r in 0..ranks {
        rank_h[r] = layers[r].iter().map(|v| vs[*v].h).fold(0.0, f32::max);
        let top_extra =
            starts_at(r) as f32 * (CLUSTER_PAD + if horizontal { 0.0 } else { CLUSTER_TITLE });
        if r > 0 {
            y += rank_h[r - 1] + rank_sep + gap_extra[r - 1] + ends_at(r - 1) as f32 * CLUSTER_PAD;
        }
        y += top_extra;
        rank_y[r] = y;
    }
    let center_y = |v: usize| rank_y[vs[v].rank] + rank_h[vs[v].rank] / 2.0;

    // 画面の座標へ
    let to_screen = |x: f32, y: f32| -> (f32, f32) { if horizontal { (y, x) } else { (x, y) } };

    let mut scene = Scene {
        kind: g.kind,
        width: 0.0,
        height: 0.0,
        nodes: Vec::new(),
        edges: Vec::new(),
        groups: Vec::new(),
        notes: Vec::new(),
        lifelines: Vec::new(),
        ranks,
    };
    let top_hue: Vec<Option<usize>> = paths.iter().map(|p| p.first().copied()).collect();
    let top_level: Vec<usize> =
        (0..g.clusters.len()).filter(|c| g.clusters[*c].parent.is_none()).collect();
    let hue_of = |c: usize| top_level.iter().position(|t| *t == c).unwrap_or(c);
    for (i, node) in g.nodes.iter().enumerate() {
        let v = &vs[i];
        let (cx, cy) = to_screen(v.x, center_y(i));
        let s = &sized[i];
        scene.nodes.push(SceneNode {
            id: node.id.clone(),
            rect: Rect::centered(cx, cy, s.w, s.h),
            shape: node.shape,
            title: s.title.clone(),
            stereotype: node.stereotype.clone(),
            sections: s.sections.clone(),
            columns: s.columns.clone(),
            hue: top_hue[i].map(hue_of),
            rank: v.rank,
            mirror_of: None,
        });
    }

    // 囲みの箱。子から先に作り、親は子を含める
    let mut order: Vec<usize> = (0..g.clusters.len()).collect();
    order.sort_by_key(|c| std::cmp::Reverse(cluster_paths[*c].len()));
    let mut boxes: Vec<Option<Rect>> = vec![None; g.clusters.len()];
    for &c in &order {
        let mut r: Option<Rect> = None;
        for (i, p) in paths.iter().enumerate() {
            if p.last() == Some(&c) {
                let nr = scene.nodes[i].rect;
                r = Some(r.map_or(nr, |b| b.union(&nr)));
            }
        }
        for (d, p) in cluster_paths.iter().enumerate() {
            if d != c
                && p.len() >= 2
                && p[p.len() - 2] == c
                && let Some(b) = boxes[d]
            {
                r = Some(r.map_or(b, |x| x.union(&b)));
            }
        }
        if let Some(mut b) = r {
            b.x -= CLUSTER_PAD;
            b.w += CLUSTER_PAD * 2.0;
            b.y -= CLUSTER_PAD + CLUSTER_TITLE;
            b.h += CLUSTER_PAD * 2.0 + CLUSTER_TITLE;
            b.w = b.w.max(title_w[c]);
            boxes[c] = Some(b);
        }
    }
    let mut group_order: Vec<usize> = (0..g.clusters.len()).collect();
    group_order.sort_by_key(|c| cluster_paths[*c].len());
    for c in group_order {
        if let Some(rect) = boxes[c] {
            scene.groups.push(SceneGroup {
                rect,
                label: g.clusters[c].label.clone(),
                keyword: None,
                depth: cluster_paths[c].len() - 1,
                hue: hue_of(cluster_paths[c][0]),
                dividers: Vec::new(),
            });
        }
    }

    // 7. 線。出入り口を箱の辺に散らし、中継点を通る滑らかな曲線にする
    let mut out_ports: Vec<Vec<(usize, f32)>> = vec![Vec::new(); n];
    let mut in_ports: Vec<Vec<(usize, f32)>> = vec![Vec::new(); n];
    for (ci, c) in chains.iter().enumerate() {
        let first = c.vertices[0];
        let second = c.vertices[1];
        let last = *c.vertices.last().expect("2 点以上");
        let before = c.vertices[c.vertices.len() - 2];
        out_ports[first].push((ci, vs[second].x));
        in_ports[last].push((ci, vs[before].x));
    }
    let port_x = |list: &mut Vec<(usize, f32)>, v: usize, chain: usize| -> f32 {
        list.sort_by(|a, b| a.1.total_cmp(&b.1));
        let count = list.len();
        let i = list.iter().position(|(c, _)| *c == chain).unwrap_or(0);
        if count <= 1 {
            return vs[v].x;
        }
        let step = (vs[v].w * 0.6 / (count - 1) as f32).min(16.0);
        vs[v].x + (i as f32 - (count - 1) as f32 / 2.0) * step
    };
    // 形に沿った上下の端
    let boundary = |v: usize, x: f32, bottom: bool| -> f32 {
        let vert = &vs[v];
        let cy = center_y(v);
        let half_h = vert.h / 2.0;
        let dx = (x - vert.x).abs();
        let inset = match vert.real.map(|i| g.nodes[i].shape) {
            Some(Shape::Diamond) => half_h * (dx / (vert.w / 2.0)).min(1.0),
            Some(Shape::Circle | Shape::Start | Shape::End) => {
                let r = vert.w.min(vert.h) / 2.0;
                r - (r * r - dx.min(r).powi(2)).sqrt()
            }
            _ => 0.0,
        };
        if bottom { cy + half_h - inset } else { cy - half_h + inset }
    };
    for (ci, c) in chains.iter().enumerate() {
        let e = &g.edges[c.edge];
        let first = c.vertices[0];
        let last = *c.vertices.last().expect("2 点以上");
        let sx = port_x(&mut out_ports[first].clone(), first, ci);
        let ex = port_x(&mut in_ports[last].clone(), last, ci);
        let mut pts: Vec<(f32, f32)> = vec![(sx, boundary(first, sx, true))];
        for &v in &c.vertices[1..c.vertices.len() - 1] {
            pts.push((vs[v].x, center_y(v)));
        }
        pts.push((ex, boundary(last, ex, false)));
        let mut path = vec![pts[0]];
        for pair in pts.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let dy = b.1 - a.1;
            path.push((a.0, a.1 + dy * 0.48));
            path.push((b.0, b.1 - dy * 0.48));
            path.push(b);
        }
        let label = c.label_segment.and_then(|seg| {
            let (lines, w, h) = labels[c.edge].clone()?;
            let k = seg * 3;
            let mid = bezier_point(path[k], path[k + 1], path[k + 2], path[k + 3], 0.5);
            let (cx, cy) = to_screen(mid.0, mid.1);
            Some(SceneLabel { rect: Rect::centered(cx, cy, w, h), lines })
        });
        let mut path: Vec<(f32, f32)> = path.into_iter().map(|(x, y)| to_screen(x, y)).collect();
        if vs[first].real != Some(e.from) {
            path.reverse();
        }
        // 囲みを端にした線は、囲みの枠で止める
        if let Some(b) = e.to_cluster.and_then(|c| boxes[c]) {
            path = clip_end(&path, &b);
        }
        if let Some(b) = e.from_cluster.and_then(|c| boxes[c]) {
            path.reverse();
            path = clip_end(&path, &b);
            path.reverse();
        }
        scene.edges.push(SceneEdge {
            from: e.from,
            to: e.to,
            path,
            line: e.line,
            start: e.start,
            end: e.end,
            label,
            start_label: None,
            end_label: None,
            number: None,
            rank: vs[first].rank.min(vs[last].rank),
        });
    }
    // 同じ間に並んだラベルが重ならないよう、横（流れと直交する向き）にずらす
    {
        let mut idx: Vec<usize> =
            (0..scene.edges.len()).filter(|i| scene.edges[*i].label.is_some()).collect();
        let along = |r: &Rect| if horizontal { r.cx() } else { r.cy() };
        idx.sort_by(|a, b| {
            let (ra, rb) = (
                scene.edges[*a].label.as_ref().unwrap().rect,
                scene.edges[*b].label.as_ref().unwrap().rect,
            );
            along(&ra).round().total_cmp(&along(&rb).round()).then(if horizontal {
                ra.cy().total_cmp(&rb.cy())
            } else {
                ra.cx().total_cmp(&rb.cx())
            })
        });
        for i in 1..idx.len() {
            let prev = scene.edges[idx[i - 1]].label.as_ref().unwrap().rect;
            let cur = &mut scene.edges[idx[i]].label.as_mut().unwrap().rect;
            let same_gap = (along(&prev) - along(cur)).abs() < 30.0;
            if !same_gap {
                continue;
            }
            if horizontal {
                if cur.y < prev.bottom() + 4.0 && cur.right() > prev.x && cur.x < prev.right() {
                    cur.y = prev.bottom() + 4.0;
                }
            } else if cur.x < prev.right() + 6.0 && cur.bottom() > prev.y && cur.y < prev.bottom() {
                cur.x = prev.right() + 6.0;
            }
        }
    }
    // 自分へ戻る線
    for (k, e) in g.edges.iter().enumerate() {
        if e.from != e.to {
            continue;
        }
        let r = scene.nodes[e.from].rect;
        let (a, b) = ((r.right(), r.cy() - 8.0), (r.right(), r.cy() + 8.0));
        let path = vec![a, (r.right() + 40.0, r.cy() - 30.0), (r.right() + 40.0, r.cy() + 30.0), b];
        let label = labels[k].clone().map(|(lines, w, h)| SceneLabel {
            rect: Rect { x: r.right() + 36.0, y: r.cy() - h / 2.0, w, h },
            lines,
        });
        scene.edges.push(SceneEdge {
            from: e.from,
            to: e.to,
            path,
            line: e.line,
            start: e.start,
            end: e.end,
            label,
            start_label: None,
            end_label: None,
            number: None,
            rank: rank[e.from],
        });
    }
    // 多重度は線の両端の脇に置く
    let edge_of: Vec<usize> = chains
        .iter()
        .map(|c| c.edge)
        .chain(g.edges.iter().enumerate().filter(|(_, e)| e.from == e.to).map(|(k, _)| k))
        .collect();
    for (si, &k) in edge_of.iter().enumerate() {
        let e = &g.edges[k];
        let path = scene.edges[si].path.clone();
        let place = |text: &str, at_end: bool| -> Option<SceneLabel> {
            if text.is_empty() || path.len() < 4 {
                return None;
            }
            let (p, q) = if at_end {
                (path[path.len() - 1], path[path.len() - 2])
            } else {
                (path[0], path[1])
            };
            let len = ((q.0 - p.0).powi(2) + (q.1 - p.1).powi(2)).sqrt().max(0.001);
            let (ux, uy) = ((q.0 - p.0) / len, (q.1 - p.1) / len);
            let w = m.width(text, SMALL_TEXT, false, false) + 8.0;
            let h = line_height(SMALL_TEXT) + 2.0;
            let (cx, cy) =
                (p.0 + ux * 18.0 - uy * (w / 2.0 + 4.0), p.1 + uy * 18.0 + ux * (h / 2.0 + 2.0));
            Some(SceneLabel { rect: Rect::centered(cx, cy, w, h), lines: vec![text.to_string()] })
        };
        scene.edges[si].start_label = place(&e.start_label, false);
        scene.edges[si].end_label = place(&e.end_label, true);
    }
    // 宣言の順に描くと、描画の重なりが読みやすい
    let mut paired: Vec<(usize, SceneEdge)> =
        edge_of.into_iter().zip(scene.edges.drain(..)).collect();
    paired.sort_by_key(|(k, _)| *k);
    scene.edges = paired
        .into_iter()
        .filter(|(k, _)| g.edges[*k].line != Line::Invisible)
        .map(|(_, e)| e)
        .collect();

    scene.normalize(MARGIN);
    scene
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagram::ast::Diagram;
    use crate::diagram::parse::parse;

    fn scene(src: &str) -> Scene {
        let Diagram::Graph(g) = parse(src).unwrap() else { panic!() };
        layout(&g, &FixedMeasure)
    }

    fn overlaps(a: &Rect, b: &Rect) -> bool {
        a.x < b.right() - 0.5
            && b.x < a.right() - 0.5
            && a.y < b.bottom() - 0.5
            && b.y < a.bottom() - 0.5
    }

    #[test]
    fn nodes_never_overlap_and_edges_flow_downward() {
        let s = scene(
            "flowchart TD\nA --> B & C & D\nB --> E\nC --> E\nD --> F\nE --> F\nA --> F\nF --> A",
        );
        for i in 0..s.nodes.len() {
            for j in i + 1..s.nodes.len() {
                assert!(
                    !overlaps(&s.nodes[i].rect, &s.nodes[j].rect),
                    "{} {}",
                    s.nodes[i].id,
                    s.nodes[j].id
                );
            }
        }
        let a = &s.nodes[0].rect;
        let b = &s.nodes[1].rect;
        assert!(b.y > a.bottom(), "A above B");
        assert_eq!(s.edges.len(), 9);
        for e in &s.edges {
            assert_eq!((e.path.len() - 1) % 3, 0);
            // 線は始点の要素から出る
            let from = s.nodes[e.from].rect;
            let p = e.path[0];
            assert!(p.0 >= from.x - 1.0 && p.0 <= from.right() + 1.0, "{p:?} {from:?}");
        }
        assert!(s.width > 0.0 && s.height > 0.0);
    }

    #[test]
    fn left_right_lays_out_horizontally() {
        let s = scene("flowchart LR\nA[入口] --> B[処理] --> C[出口]");
        assert!(s.nodes[1].rect.x > s.nodes[0].rect.right());
        assert!(s.nodes[2].rect.x > s.nodes[1].rect.right());
        assert!((s.nodes[0].rect.cy() - s.nodes[2].rect.cy()).abs() < 1.0);
    }

    #[test]
    fn clusters_contain_their_nodes_and_do_not_overlap_others() {
        let s = scene(
            "flowchart TD\nsubgraph ui[画面]\n a[ホーム] --> b[ワークスペース]\nend\nsubgraph core[コア]\n c[索引] --> d[保存]\nend\nsubgraph ai[AI]\n e[実行]\nend\nb --> c\nb --> e\ne --> d\nx[外] --> a",
        );
        assert_eq!(s.groups.len(), 3);
        for (gi, group) in s.groups.iter().enumerate() {
            let ids: &[&str] = match gi {
                0 => &["a", "b"],
                1 => &["c", "d"],
                _ => &["e"],
            };
            for n in &s.nodes {
                let inside = group.rect.contains(n.rect.cx(), n.rect.cy());
                assert_eq!(inside, ids.contains(&n.id.as_str()), "{} in {}", n.id, group.label);
            }
        }
        for i in 0..s.groups.len() {
            for j in i + 1..s.groups.len() {
                assert!(!overlaps(&s.groups[i].rect, &s.groups[j].rect), "{i} {j}");
            }
        }
    }

    #[test]
    fn real_module_diagram_has_no_overlapping_clusters() {
        struct M(f32);
        impl Measure for M {
            fn advance(&self, c: char, size: f32, mono: bool, _: bool) -> f32 {
                if c.is_ascii() { size * if mono { 0.6 } else { self.0 } } else { size }
            }
        }
        let Diagram::Graph(g) = parse(include_str!("testdata_modules.mmd")).unwrap() else {
            panic!()
        };
        for k in 40..70 {
            let s = layout(&g, &M(k as f32 / 100.0));
            for i in 0..s.groups.len() {
                for j in i + 1..s.groups.len() {
                    assert!(
                        !overlaps(&s.groups[i].rect, &s.groups[j].rect),
                        "k={k} {} {:?} / {} {:?}",
                        s.groups[i].label,
                        s.groups[i].rect,
                        s.groups[j].label,
                        s.groups[j].rect
                    );
                }
            }
        }
    }

    #[test]
    fn sibling_clusters_with_different_spans_do_not_overlap() {
        let s = scene(
            "flowchart TD\nsubgraph shared[共有]\n t[types]\n r[risk]\nend\nsubgraph server[サーバ]\n i[index] --> m[main] --> a[app] --> rt[routes] --> rv[reviewer]\nend\nsubgraph client[画面]\n c1[main.tsx] --> c2[state] --> c3[api]\nend\nc3 --> a\nc1 --> t\ni --> t\nrv --> r\nm --> r",
        );
        for i in 0..s.groups.len() {
            for j in i + 1..s.groups.len() {
                assert!(
                    !overlaps(&s.groups[i].rect, &s.groups[j].rect),
                    "{} {}",
                    s.groups[i].label,
                    s.groups[j].label
                );
            }
        }
    }

    #[test]
    fn labels_get_room_and_tables_get_columns() {
        let s = scene("flowchart TD\nA -->|とても長いラベルの文字| B");
        let label = s.edges[0].label.as_ref().unwrap();
        assert!(!overlaps(&label.rect, &s.nodes[0].rect));
        assert!(!overlaps(&label.rect, &s.nodes[1].rect));
        let s = scene("erDiagram\nA {\n string name PK\n int id\n}\nA ||--o{ B : has");
        assert_eq!(s.nodes[0].columns.len(), 3);
        assert!(s.nodes[0].rect.h > 60.0);
    }

    #[test]
    fn inheritance_puts_parent_on_top_and_self_loops_render() {
        let s = scene("classDiagram\nChild --|> Parent\nParent --> Parent : self");
        let child = s.nodes.iter().find(|n| n.id == "Child").unwrap();
        let parent = s.nodes.iter().find(|n| n.id == "Parent").unwrap();
        assert!(parent.rect.bottom() < child.rect.y);
        assert_eq!(s.edges.len(), 2);
        // 継承の三角は親の側の端にある
        let e = &s.edges[0];
        let end = *e.path.last().unwrap();
        assert!((end.1 - parent.rect.bottom()).abs() < 2.0, "{end:?} {:?}", parent.rect);
    }

    #[test]
    fn handles_bottom_up_and_state_diagrams() {
        let s = scene("flowchart BT\nA --> B");
        assert!(s.nodes[1].rect.bottom() < s.nodes[0].rect.y);
        let s =
            scene("stateDiagram-v2\n[*] --> A\nA --> B : go\nB --> [*]\nstate B {\n[*] --> C\n}");
        assert!(s.nodes.iter().any(|n| n.shape == Shape::Start));
        assert_eq!(s.groups.len(), 1);
    }

    #[test]
    fn pava_keeps_order() {
        let got = pava(&[3.0, 1.0, 2.0], &[1.0, 1.0, 1.0]);
        assert!(got[0] <= got[1] && got[1] <= got[2]);
        assert!((got[0] - 2.0).abs() < 1e-5);
    }
}
