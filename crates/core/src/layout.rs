//! 星の決定的な配置。乱数は使わない。
use crate::model::FileEntry;

const GOLDEN: f32 = 2.399_963;
const MODULE_GAP: f32 = 30.0;
/// ウィンドウは横長なので、星座の並びも横へ広げる
const WIDE: f32 = 1.5;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Star {
    pub x: f32,
    pub y: f32,
    pub radius: f32,
    pub brightness: f32,
    pub module: usize,
}

fn phyllotaxis(i: usize, spacing: f32) -> (f32, f32) {
    let r = spacing * (i as f32 + 0.5).sqrt();
    let t = i as f32 * GOLDEN;
    (r * t.cos(), r * t.sin())
}

/// 大半の星は小さく、大きなファイルだけが際立つように、対数を累乗して差を広げる。
fn star_radius(lines: u32) -> f32 {
    0.7 + 0.32 * ((lines + 1) as f32).log10().powf(2.2)
}

/// パスから 0.0〜1.0 の値を 2 つ作る。乱数を使わず、同じファイルは常に同じ揺らぎになる。
fn jitter(path: &str) -> (f32, f32) {
    let mut h: u32 = 0x811c_9dc5;
    for b in path.bytes() {
        h = (h ^ b as u32).wrapping_mul(0x0100_0193);
    }
    let h2 = (h ^ (h >> 15)).wrapping_mul(0x2c1b_3c6d);
    ((h % 10_000) as f32 / 10_000.0, ((h2 >> 8) % 10_000) as f32 / 10_000.0)
}

/// `assignment` はファイルごとのモジュール添字。`module_count` は「その他」も含めた数の上限で、
/// 範囲外の添字は `module_count` 番目の星座にまとめる。
pub fn layout(files: &[FileEntry], assignment: &[usize], module_count: usize) -> Vec<Star> {
    let max_churn = files.iter().map(|f| f.churn).max().unwrap_or(0);
    let brightness = |churn: u32| {
        if max_churn == 0 {
            0.35
        } else {
            0.35 + 0.65 * ((churn + 1) as f32).log2() / ((max_churn + 1) as f32).log2()
        }
    };

    // モジュールごとに、行数の降順（同数ならパスの昇順）で並べる
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); module_count + 1];
    for (i, m) in assignment.iter().enumerate() {
        members[(*m).min(module_count)].push(i);
    }
    for list in &mut members {
        list.sort_by(|a, b| {
            files[*b].lines.cmp(&files[*a].lines).then_with(|| files[*a].path.cmp(&files[*b].path))
        });
    }

    // 各星座の内部配置と外接半径
    let mut stars = vec![Star { x: 0.0, y: 0.0, radius: 0.0, brightness: 0.0, module: 0 }; files.len()];
    let mut circles: Vec<(usize, f32)> = Vec::new();
    for (mi, list) in members.iter().enumerate() {
        if list.is_empty() {
            continue;
        }
        let mean: f32 =
            list.iter().map(|i| star_radius(files[*i].lines)).sum::<f32>() / list.len() as f32;
        let spacing = mean.max(1.4) * 4.4;
        let mut extent = 0.0f32;
        for (k, i) in list.iter().enumerate() {
            let (mut x, mut y) = phyllotaxis(k, spacing);
            // 螺旋の規則性を崩して、自然な星の散らばりにする
            let (ja, jb) = jitter(&files[*i].path);
            let angle = ja * std::f32::consts::TAU;
            x += angle.cos() * jb * spacing * 0.8;
            y += angle.sin() * jb * spacing * 0.8;
            // 星座ごとに少し潰して、円盤に見えないようにする
            let (sa, sb) = jitter(&mi.to_string());
            let stretch = 0.72 + sa * 0.3;
            let tilt = sb * std::f32::consts::PI;
            let (c, si) = (tilt.cos(), tilt.sin());
            let (u, v) = (x * c + y * si, (y * c - x * si) * stretch);
            (x, y) = (u * c - v * si, u * si + v * c);
            stars[*i] = Star {
                x,
                y,
                radius: star_radius(files[*i].lines),
                brightness: brightness(files[*i].churn),
                module: mi,
            };
            extent = extent.max((x * x + y * y).sqrt());
        }
        circles.push((mi, extent + 24.0));
    }

    // 大きい星座から順に、螺旋の内側の空いた場所へ置く
    circles.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let mut placed: Vec<(f32, f32, f32)> = Vec::new();
    let mut centers = vec![(0.0f32, 0.0f32); module_count + 1];
    for (mi, radius) in circles {
        let mut k = 0;
        let center = loop {
            let c = phyllotaxis(k, 30.0);
            let free = placed.iter().all(|(x, y, r)| {
                let d = ((c.0 - x).powi(2) + (c.1 - y).powi(2)).sqrt();
                d >= r + radius + MODULE_GAP
            });
            if free {
                break c;
            }
            k += 1;
        };
        placed.push((center.0, center.1, radius));
        centers[mi] = center;
    }
    for s in &mut stars {
        s.x += centers[s.module].0 * WIDE;
        s.y += centers[s.module].1;
    }
    stars
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Lang;

    fn file(path: &str, lines: u32, churn: u32) -> FileEntry {
        FileEntry { path: path.into(), lang: Lang::Other, lines, bytes: 0, binary: false, churn }
    }

    #[test]
    fn layout_is_deterministic_and_stars_do_not_overlap_much() {
        let files: Vec<_> =
            (0..500).map(|i| file(&format!("d{}/f{i}.rs", i % 7), i + 1, i % 9)).collect();
        let assignment: Vec<_> = (0..500).map(|i| i % 7).collect();
        let a = layout(&files, &assignment, 7);
        assert_eq!(a, layout(&files, &assignment, 7));
        let mut close = 0;
        for i in 0..a.len() {
            for j in i + 1..a.len() {
                let d = ((a[i].x - a[j].x).powi(2) + (a[i].y - a[j].y).powi(2)).sqrt();
                if d < (a[i].radius + a[j].radius) * 0.8 {
                    close += 1;
                }
            }
        }
        assert!(close < 5, "overlaps: {close}");
    }

    #[test]
    fn bigger_files_are_larger_and_active_files_are_brighter() {
        let files = [file("a", 10, 0), file("b", 10_000, 50)];
        let s = layout(&files, &[0, 0], 1);
        assert!(s[1].radius > s[0].radius);
        assert!(s[1].brightness > s[0].brightness);
    }

    #[test]
    fn unassigned_files_gather_in_the_last_slot_and_empty_input_is_fine() {
        let files = [file("a", 1, 0), file("b", 1, 0)];
        let s = layout(&files, &[0, 1], 1);
        assert_eq!((s[0].module, s[1].module), (0, 1));
        assert!(layout(&[], &[], 0).is_empty());
    }

    #[test]
    fn handles_large_repositories_quickly() {
        let files: Vec<_> =
            (0..50_000).map(|i| file(&format!("d{}/f{i}", i % 200), i % 500, 0)).collect();
        let assignment: Vec<_> = (0..50_000).map(|i| i % 200).collect();
        let start = std::time::Instant::now();
        let s = layout(&files, &assignment, 200);
        assert_eq!(s.len(), 50_000);
        assert!(start.elapsed().as_secs() < 2, "{:?}", start.elapsed());
    }
}
