//! シーケンス図の配置。参加者を列に、メッセージを行に並べる。
use super::ast::{Kind, Line, Marker, NoteSide, SeqItem, Sequence, Shape};
use super::scene::*;

const MARGIN: f32 = 18.0;
const COLUMN_GAP: f32 = 44.0;
const LABEL_WRAP: f32 = 230.0;
const SELF_LOOP: f32 = 40.0;

struct Frame {
    keyword: String,
    label: String,
    top: f32,
    dividers: Vec<(f32, String)>,
    /// 中身の左右の端
    min_x: f32,
    max_x: f32,
    /// 入れ子の深さの最大。外側の枠ほど余白を広げる
    height: usize,
}

pub fn layout(s: &Sequence, m: &dyn Measure) -> Scene {
    let lh = line_height(NODE_TEXT);
    let slh = line_height(SMALL_TEXT);
    let count = s.participants.len();

    // 参加者の箱
    let heads: Vec<(Vec<String>, f32, f32)> = s
        .participants
        .iter()
        .map(|p| {
            let lines = m.wrap(&p.label, NODE_TEXT, 160.0, true);
            let tw = lines.iter().map(|l| m.width(l, NODE_TEXT, false, true)).fold(0.0, f32::max);
            let h = lines.len() as f32 * lh + if p.actor { 40.0 } else { 18.0 };
            (lines, (tw + 30.0).max(96.0).ceil(), h.ceil())
        })
        .collect();
    let head_h = heads.iter().map(|h| h.2).fold(0.0, f32::max);
    let mut cx = vec![0.0f32; count];
    for i in 0..count {
        cx[i] = if i == 0 {
            heads[0].1 / 2.0
        } else {
            cx[i - 1] + (heads[i - 1].1 + heads[i].1) / 2.0 + COLUMN_GAP
        };
    }
    let push_from = |cx: &mut Vec<f32>, at: usize, delta: f32| {
        if delta > 0.0 {
            for x in cx.iter_mut().skip(at) {
                *x += delta;
            }
        }
    };

    // 文字の幅に合わせて列の間を広げる
    let texts: Vec<Vec<String>> = s
        .items
        .iter()
        .map(|item| match item {
            SeqItem::Message { text, .. } | SeqItem::Note { text, .. } if !text.is_empty() => {
                m.wrap(text, SMALL_TEXT, LABEL_WRAP, false)
            }
            _ => Vec::new(),
        })
        .collect();
    let width_of = |lines: &[String]| {
        lines.iter().map(|l| m.width(l, SMALL_TEXT, false, false)).fold(0.0, f32::max)
    };
    for (k, item) in s.items.iter().enumerate() {
        match item {
            SeqItem::Message { from, to, .. } if from != to => {
                let (lo, hi) = (*from.min(to), *from.max(to));
                let need = width_of(&texts[k]) + 40.0;
                let have = cx[hi] - cx[lo];
                push_from(&mut cx, hi, need - have);
            }
            SeqItem::Message { from, .. } => {
                if from + 1 < count {
                    let need = SELF_LOOP + 16.0 + width_of(&texts[k]) + heads[from + 1].1 / 2.0;
                    let have = cx[from + 1] - cx[*from];
                    push_from(&mut cx, from + 1, need - have);
                }
            }
            SeqItem::Note { side, first, last, .. } => {
                let w = width_of(&texts[k]) + 22.0;
                match side {
                    NoteSide::Right if first + 1 < count => {
                        let need = w + 18.0 + heads[first + 1].1 / 2.0;
                        let have = cx[first + 1] - cx[*first];
                        push_from(&mut cx, first + 1, need - have);
                    }
                    NoteSide::Left if *first > 0 => {
                        let need = w + 18.0 + heads[first - 1].1 / 2.0;
                        let have = cx[*first] - cx[first - 1];
                        push_from(&mut cx, *first, need - have);
                    }
                    NoteSide::Over if first != last => {
                        let have = cx[*last] - cx[*first] + 60.0;
                        push_from(&mut cx, *last, w - have);
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    let mut scene = Scene {
        kind: Kind::Sequence,
        width: 0.0,
        height: 0.0,
        nodes: Vec::new(),
        edges: Vec::new(),
        groups: Vec::new(),
        notes: Vec::new(),
        lifelines: Vec::new(),
        ranks: 0,
    };
    for (i, p) in s.participants.iter().enumerate() {
        let (lines, w, h) = &heads[i];
        scene.nodes.push(SceneNode {
            id: p.id.clone(),
            rect: Rect { x: cx[i] - w / 2.0, y: head_h - h, w: *w, h: *h },
            shape: if p.actor { Shape::Actor } else { Shape::Participant },
            title: lines.clone(),
            stereotype: None,
            sections: Vec::new(),
            columns: Vec::new(),
            hue: None,
            rank: 0,
            mirror_of: None,
        });
    }

    let mut y = head_h + 26.0;
    let mut frames: Vec<Frame> = Vec::new();
    let mut number = 0;
    let mut row = 0;
    let widen = |frames: &mut Vec<Frame>, lo: f32, hi: f32| {
        for f in frames.iter_mut() {
            f.min_x = f.min_x.min(lo);
            f.max_x = f.max_x.max(hi);
        }
    };
    for (k, item) in s.items.iter().enumerate() {
        match item {
            SeqItem::Message { from, to, line, end, .. } => {
                row += 1;
                let lines = texts[k].clone();
                let label_h = lines.len() as f32 * slh;
                let w = width_of(&lines) + 12.0;
                number += 1;
                let numbered = s.autonumber.then_some(number);
                if from == to {
                    let x = cx[*from];
                    let top = y + 4.0;
                    let path = vec![
                        (x, top),
                        (x + SELF_LOOP * 1.4, top - 2.0),
                        (x + SELF_LOOP * 1.4, top + 26.0),
                        (x, top + 24.0),
                    ];
                    let label = (!lines.is_empty()).then(|| SceneLabel {
                        rect: Rect {
                            x: x + SELF_LOOP + 12.0,
                            y: top + 12.0 - label_h / 2.0 - 3.0,
                            w,
                            h: label_h + 6.0,
                        },
                        lines,
                    });
                    widen(&mut frames, x - 10.0, x + SELF_LOOP + 16.0 + w);
                    scene.edges.push(SceneEdge {
                        from: *from,
                        to: *to,
                        path,
                        line: *line,
                        start: Marker::None,
                        end: *end,
                        label,
                        start_label: None,
                        end_label: None,
                        number: numbered,
                        rank: row,
                    });
                    y = top + 24.0 + 18.0;
                } else {
                    let arrow_y = y + label_h + 8.0;
                    let (a, b) = (cx[*from], cx[*to]);
                    let mid = (a + b) / 2.0;
                    let label = (!lines.is_empty()).then(|| SceneLabel {
                        rect: Rect {
                            x: mid - w / 2.0,
                            y: arrow_y - label_h - 7.0,
                            w,
                            h: label_h + 4.0,
                        },
                        lines,
                    });
                    widen(&mut frames, a.min(b) - 10.0, a.max(b) + 10.0);
                    scene.edges.push(SceneEdge {
                        from: *from,
                        to: *to,
                        path: straight((a, arrow_y), (b, arrow_y)),
                        line: *line,
                        start: Marker::None,
                        end: *end,
                        label,
                        start_label: None,
                        end_label: None,
                        number: numbered,
                        rank: row,
                    });
                    y = arrow_y + 16.0;
                }
            }
            SeqItem::Note { side, first, last, .. } => {
                let lines = texts[k].clone();
                let w = width_of(&lines) + 22.0;
                let h = lines.len().max(1) as f32 * slh + 14.0;
                let x = match side {
                    NoteSide::Left => cx[*first] - 12.0 - w,
                    NoteSide::Right => cx[*first] + 12.0,
                    NoteSide::Over => {
                        let (a, b) = (cx[*first] - 30.0, cx[*last] + 30.0);
                        let span = (b - a).max(w);
                        (a + b) / 2.0 - span / 2.0
                    }
                };
                let w =
                    if *side == NoteSide::Over { w.max(cx[*last] - cx[*first] + 60.0) } else { w };
                let rect = Rect { x, y: y + 2.0, w, h };
                widen(&mut frames, rect.x - 6.0, rect.right() + 6.0);
                scene.notes.push(SceneNote { rect, lines });
                y += h + 14.0;
            }
            SeqItem::Open { kind, label } => {
                frames.push(Frame {
                    keyword: kind.clone(),
                    label: label.clone(),
                    top: y,
                    dividers: Vec::new(),
                    min_x: f32::MAX,
                    max_x: f32::MIN,
                    height: 0,
                });
                y += 30.0;
            }
            SeqItem::Divide { label } => {
                if let Some(f) = frames.last_mut() {
                    f.dividers.push((y + 4.0, label.clone()));
                }
                y += 30.0;
            }
            SeqItem::Close => {
                if let Some(f) = frames.pop() {
                    let (lo, hi) = if f.min_x <= f.max_x {
                        (f.min_x, f.max_x)
                    } else {
                        (cx[0] - 40.0, cx[count - 1] + 40.0)
                    };
                    let pad = 14.0 + f.height as f32 * 10.0;
                    let tag = m.width(&f.keyword, SMALL_TEXT, false, true) + 18.0;
                    let title = tag + m.width(&f.label, SMALL_TEXT, false, false) + 24.0;
                    let mut rect =
                        Rect { x: lo - pad, y: f.top, w: hi - lo + pad * 2.0, h: y + 6.0 - f.top };
                    rect.w = rect.w.max(title);
                    if let Some(parent) = frames.last_mut() {
                        parent.height = parent.height.max(f.height + 1);
                    }
                    widen(&mut frames, rect.x, rect.right());
                    let depth = frames.len();
                    scene.groups.push(SceneGroup {
                        rect,
                        label: f.label,
                        keyword: Some(f.keyword),
                        depth,
                        hue: depth,
                        dividers: f.dividers,
                    });
                    y += 14.0;
                }
            }
        }
    }
    let bottom = y + 10.0;
    for (i, (lines, w, h)) in heads.iter().enumerate() {
        scene.lifelines.push(Lifeline { node: i, x: cx[i], top: head_h, bottom });
        scene.nodes.push(SceneNode {
            id: s.participants[i].id.clone(),
            rect: Rect { x: cx[i] - w / 2.0, y: bottom, w: *w, h: *h },
            shape: if s.participants[i].actor { Shape::Actor } else { Shape::Participant },
            title: lines.clone(),
            stereotype: None,
            sections: Vec::new(),
            columns: Vec::new(),
            hue: None,
            rank: row + 1,
            mirror_of: Some(i),
        });
    }
    // 外側の枠を先に描く
    scene.groups.sort_by_key(|g| g.depth);
    scene.ranks = row + 2;
    let _ = Line::Solid;
    scene.normalize(MARGIN);
    scene
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagram::ast::Diagram;
    use crate::diagram::parse::parse;

    fn scene(src: &str) -> Scene {
        let Diagram::Sequence(s) = parse(src).unwrap() else { panic!() };
        layout(&s, &FixedMeasure)
    }

    #[test]
    fn columns_make_room_for_labels_and_frames_wrap_content() {
        let s = scene(
            "sequenceDiagram\nautonumber\nA->>B: とても長いメッセージの本文をここに書く\nloop 繰り返し\nB->>B: 自分\nB-->>A: 返事\nend\nNote over A,B: 注釈",
        );
        let (a, b) = (&s.nodes[0].rect, &s.nodes[1].rect);
        let label = s.edges[0].label.as_ref().unwrap();
        assert!(b.cx() - a.cx() >= label.rect.w);
        assert_eq!(s.edges[0].number, Some(1));
        assert_eq!(s.groups.len(), 1);
        let frame = &s.groups[0].rect;
        for e in &s.edges[1..] {
            for p in &e.path {
                assert!(frame.contains(p.0, p.1), "{p:?} {frame:?}");
            }
        }
        // 下端に参加者を繰り返す
        assert_eq!(s.nodes.len(), 4);
        assert_eq!(s.nodes[2].mirror_of, Some(0));
        assert_eq!(s.lifelines.len(), 2);
        assert_eq!(s.notes.len(), 1);
    }

    #[test]
    fn nested_frames_get_wider_padding() {
        let s = scene("sequenceDiagram\nalt 外\nloop 内\nA->>B: x\nend\nelse 別\nA->>B: y\nend");
        assert_eq!(s.groups.len(), 2);
        let outer = &s.groups[0];
        let inner = &s.groups[1];
        assert!(outer.rect.x < inner.rect.x && outer.rect.right() > inner.rect.right());
        assert_eq!(outer.dividers.len(), 1);
    }
}
