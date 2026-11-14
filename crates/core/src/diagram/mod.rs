//! Mermaid の図を解析し、配置する。描画は呼び出し側が行う。
pub mod ast;
pub mod layered;
pub mod parse;
pub mod scene;
pub mod sequence;

pub use ast::{Diagram, Kind, ParseError};
pub use parse::parse;
pub use scene::{Measure, Scene};

/// 本文の幅 `width`、高さの上限 `max_height` に置いたとき、いちばん大きく読める向きで配置する。
/// 縦横どちらの向きでも意味の変わらないフローチャートとクラス図などで、向きを入れ替えて比べる。
pub fn layout_to_fit(
    diagram: &Diagram,
    measure: &dyn Measure,
    width: f32,
    max_height: f32,
) -> Scene {
    let score = |s: &Scene| (width / s.width).min(max_height / s.height).min(1.2);
    let first = layout(diagram, measure);
    let Diagram::Graph(g) = diagram else { return first };
    if score(&first) >= 0.95 {
        return first;
    }
    let mut turned = g.clone();
    turned.dir = match g.dir {
        ast::Dir::LeftRight | ast::Dir::RightLeft => ast::Dir::TopDown,
        ast::Dir::TopDown | ast::Dir::BottomUp => ast::Dir::LeftRight,
    };
    let second = layered::layout(&turned, measure);
    if score(&second) > score(&first) * 1.5 { second } else { first }
}

/// 解析済みの図を配置する。
pub fn layout(diagram: &Diagram, measure: &dyn Measure) -> Scene {
    match diagram {
        Diagram::Graph(g) => layered::layout(g, measure),
        Diagram::Sequence(s) => sequence::layout(s, measure),
    }
}
