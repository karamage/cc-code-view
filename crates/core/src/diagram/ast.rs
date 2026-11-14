//! Mermaid の図を解析した結果。描き方には依存しない。

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dir {
    TopDown,
    BottomUp,
    LeftRight,
    RightLeft,
}

impl Dir {
    pub fn horizontal(self) -> bool {
        matches!(self, Dir::LeftRight | Dir::RightLeft)
    }
}

/// どの記法で書かれた図か。色や凡例を変えるのに使う。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Flowchart,
    Class,
    Entity,
    State,
    Sequence,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Flowchart => "フローチャート",
            Kind::Class => "クラス図",
            Kind::Entity => "ER 図",
            Kind::State => "状態遷移図",
            Kind::Sequence => "シーケンス図",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    Rect,
    Round,
    Stadium,
    Circle,
    Diamond,
    Hexagon,
    Cylinder,
    Subroutine,
    Parallelogram,
    Flag,
    /// クラス図とER図の箱。見出しと区画を持つ
    Table,
    /// 状態遷移図の始点と終点
    Start,
    End,
    /// 並行状態の分岐と合流
    Bar,
    /// シーケンス図の参加者
    Participant,
    Actor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Line {
    Solid,
    Dotted,
    Thick,
    /// 配置だけに使い、描かない
    Invisible,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Marker {
    None,
    Arrow,
    /// 矢じりを線だけで描く。非同期のメッセージ
    OpenArrow,
    Circle,
    Cross,
    /// 継承と実現。中が空の三角
    Triangle,
    /// コンポジション
    Diamond,
    /// 集約
    DiamondOpen,
    /// ER 図の多重度
    One,
    ZeroOrOne,
    OneOrMany,
    ZeroOrMany,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Node {
    pub id: String,
    pub label: String,
    pub shape: Shape,
    /// `<<interface>>` のような添え書き
    pub stereotype: Option<String>,
    /// クラスの属性と操作、エンティティの列。区画ごとに分ける
    pub sections: Vec<Vec<String>>,
    pub cluster: Option<usize>,
}

impl Node {
    pub fn new(id: &str, label: &str, shape: Shape) -> Node {
        Node {
            id: id.to_string(),
            label: label.to_string(),
            shape,
            stereotype: None,
            sections: Vec::new(),
            cluster: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    pub from: usize,
    pub to: usize,
    pub label: String,
    pub line: Line,
    pub start: Marker,
    pub end: Marker,
    /// クラス図の多重度
    pub start_label: String,
    pub end_label: String,
    /// 配置の上下を逆にする。継承は親を上に置く
    pub upward: bool,
    /// 囲み（subgraph）そのものを端にする線。線は囲みの枠で止める
    pub from_cluster: Option<usize>,
    pub to_cluster: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Cluster {
    pub id: String,
    pub label: String,
    pub parent: Option<usize>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Graph {
    pub kind: Kind,
    pub dir: Dir,
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
    pub clusters: Vec<Cluster>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Participant {
    pub id: String,
    pub label: String,
    pub actor: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoteSide {
    Left,
    Right,
    Over,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SeqItem {
    Message {
        from: usize,
        to: usize,
        text: String,
        line: Line,
        end: Marker,
    },
    Note {
        side: NoteSide,
        first: usize,
        last: usize,
        text: String,
    },
    /// `loop` `alt` `opt` `par` `critical` `break` `rect`
    Open {
        kind: String,
        label: String,
    },
    /// `else` `and` `option`
    Divide {
        label: String,
    },
    Close,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Sequence {
    pub participants: Vec<Participant>,
    pub items: Vec<SeqItem>,
    pub autonumber: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Diagram {
    Graph(Graph),
    Sequence(Sequence),
}

impl Diagram {
    pub fn kind(&self) -> Kind {
        match self {
            Diagram::Graph(g) => g.kind,
            Diagram::Sequence(_) => Kind::Sequence,
        }
    }

    /// 図の中の要素の識別子。AI がファイルに結び付けた要素を確かめるのに使う。
    pub fn has_node(&self, id: &str) -> bool {
        match self {
            Diagram::Graph(g) => g.nodes.iter().any(|n| n.id == id),
            Diagram::Sequence(s) => s.participants.iter().any(|p| p.id == id),
        }
    }

    pub fn node_count(&self) -> usize {
        match self {
            Diagram::Graph(g) => g.nodes.len(),
            Diagram::Sequence(s) => s.participants.len(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParseError {
    /// 1 始まり
    pub line: usize,
    pub message: String,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} 行目: {}", self.line, self.message)
    }
}
