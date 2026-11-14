//! Mermaid の一部の記法を解析する。AI の出力は信頼できない入力なので、
//! リンク、スタイル、初期化の指示は読み飛ばし、大きさにも上限を設ける。
//!
//! 対応する記法: flowchart / graph、sequenceDiagram、classDiagram、erDiagram、stateDiagram-v2
use super::ast::*;
use std::collections::HashMap;

const MAX_NODES: usize = 160;
const MAX_EDGES: usize = 400;
const MAX_LABEL: usize = 240;

type Res<T> = Result<T, ParseError>;

fn err<T>(line: usize, message: impl Into<String>) -> Res<T> {
    Err(ParseError { line, message: message.into() })
}

/// 行と、元の行番号。コメントと空行を除く。
fn statements(src: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    for (i, raw) in src.lines().enumerate() {
        let line = match raw.find("%%") {
            Some(at) => &raw[..at],
            None => raw,
        };
        let line = line.trim();
        if !line.is_empty() {
            out.push((i + 1, line.to_string()));
        }
    }
    out
}

/// ラベルの書式を外す。HTML は解釈せず、改行だけを生かす。
pub fn clean_label(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s = s[1..s.len() - 1].to_string();
    }
    // Markdown 文字列 "`...`"
    if s.len() >= 2 && s.starts_with('`') && s.ends_with('`') {
        s = s[1..s.len() - 1].to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s.as_str();
    while let Some(at) = rest.find('<') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        match tail.find('>') {
            Some(close)
                if close <= 12
                    && tail[1..close]
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '/' || c == ' ') =>
            {
                let tag = tail[1..close].trim().trim_end_matches('/').trim().to_ascii_lowercase();
                if tag == "br" {
                    out.push('\n');
                }
                rest = &tail[close + 1..];
            }
            _ => {
                out.push('<');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    let out = out
        .replace("\\n", "\n")
        .replace("&quot;", "\"")
        .replace("#quot;", "\"")
        .replace("&lt;", "<")
        .replace("#lt;", "<")
        .replace("&gt;", ">")
        .replace("#gt;", ">")
        .replace("&amp;", "&")
        .replace("#amp;", "&")
        .replace("**", "");
    let out: String = out.lines().map(str::trim).collect::<Vec<_>>().join("\n");
    if out.chars().count() > MAX_LABEL {
        out.chars().take(MAX_LABEL).collect::<String>() + "…"
    } else {
        out
    }
}

/// `List~int~` を `List<int>` にする。
fn generics(s: &str) -> String {
    let mut out = String::new();
    let mut open = true;
    for c in s.chars() {
        if c == '~' {
            out.push(if open { '<' } else { '>' });
            open = !open;
        } else {
            out.push(c);
        }
    }
    out
}

fn starts_with_ci(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len()
        && s.is_char_boundary(prefix.len())
        && s[..prefix.len()].eq_ignore_ascii_case(prefix)
}

/// 先頭の語を除いた残り。語の後ろは空白か行末でなければならない。
fn after_keyword<'a>(s: &'a str, keyword: &str) -> Option<&'a str> {
    if !starts_with_ci(s, keyword) {
        return None;
    }
    let rest = &s[keyword.len()..];
    if rest.is_empty() {
        return Some("");
    }
    rest.starts_with([' ', '\t']).then(|| rest.trim())
}

pub fn parse(src: &str) -> Res<Diagram> {
    let lines = statements(src);
    let Some((first_line, header)) = lines.first().cloned() else {
        return err(1, "図が空です");
    };
    let lower = header.to_ascii_lowercase();
    let body = &lines[1..];
    let word = lower.split_whitespace().next().unwrap_or("");
    let diagram = match word {
        "flowchart" | "graph" | "flowchart-elk" => {
            let dir =
                parse_dir(lower.split_whitespace().nth(1).unwrap_or("td")).unwrap_or(Dir::TopDown);
            Diagram::Graph(parse_flow(body, dir)?)
        }
        "classdiagram" | "classdiagram-v2" => Diagram::Graph(parse_class(body)?),
        "erdiagram" => Diagram::Graph(parse_er(body)?),
        "statediagram" | "statediagram-v2" => Diagram::Graph(parse_state(body)?),
        "sequencediagram" => Diagram::Sequence(parse_sequence(body)?),
        _ => {
            return err(
                first_line,
                format!(
                    "対応していない図の種類です: {word}。flowchart、sequenceDiagram、classDiagram、erDiagram、stateDiagram-v2 のいずれかで書いてください"
                ),
            );
        }
    };
    if diagram.node_count() == 0 {
        return err(first_line, "要素が一つもありません");
    }
    Ok(diagram)
}

fn parse_dir(s: &str) -> Option<Dir> {
    Some(match s.to_ascii_uppercase().as_str() {
        "TD" | "TB" => Dir::TopDown,
        "BT" => Dir::BottomUp,
        "LR" => Dir::LeftRight,
        "RL" => Dir::RightLeft,
        _ => return None,
    })
}

// ───────────────────────── フローチャート ─────────────────────────

struct GraphBuilder {
    g: Graph,
    ids: HashMap<String, usize>,
    stack: Vec<usize>,
}

impl GraphBuilder {
    fn new(kind: Kind, dir: Dir) -> GraphBuilder {
        GraphBuilder {
            g: Graph { kind, dir, nodes: Vec::new(), edges: Vec::new(), clusters: Vec::new() },
            ids: HashMap::new(),
            stack: Vec::new(),
        }
    }

    fn node(
        &mut self,
        line: usize,
        id: &str,
        label: Option<String>,
        shape: Option<Shape>,
    ) -> Res<usize> {
        if let Some(&i) = self.ids.get(id) {
            let n = &mut self.g.nodes[i];
            if let Some(l) = label.filter(|l| !l.is_empty()) {
                n.label = l;
            }
            if let Some(s) = shape {
                n.shape = s;
            }
            return Ok(i);
        }
        if self.g.nodes.len() >= MAX_NODES {
            return err(line, format!("要素が多すぎます（{MAX_NODES} 個まで）"));
        }
        let default_shape = match self.g.kind {
            Kind::Class | Kind::Entity => Shape::Table,
            Kind::State => Shape::Round,
            _ => Shape::Rect,
        };
        let mut n =
            Node::new(id, &label.unwrap_or_else(|| id.to_string()), shape.unwrap_or(default_shape));
        n.cluster = self.stack.last().copied();
        self.g.nodes.push(n);
        self.ids.insert(id.to_string(), self.g.nodes.len() - 1);
        Ok(self.g.nodes.len() - 1)
    }

    fn edge(&mut self, line: usize, e: Edge) -> Res<()> {
        if self.g.edges.len() >= MAX_EDGES {
            return err(line, format!("線が多すぎます（{MAX_EDGES} 本まで）"));
        }
        self.g.edges.push(e);
        Ok(())
    }

    fn open_cluster(&mut self, id: String, label: String) {
        self.g.clusters.push(Cluster { id, label, parent: self.stack.last().copied() });
        self.stack.push(self.g.clusters.len() - 1);
    }
}

fn plain_edge(from: usize, to: usize) -> Edge {
    Edge {
        from,
        to,
        label: String::new(),
        line: Line::Solid,
        start: Marker::None,
        end: Marker::Arrow,
        start_label: String::new(),
        end_label: String::new(),
        upward: false,
        from_cluster: None,
        to_cluster: None,
    }
}

/// subgraph の名前を端にした線を、囲みの中の要素へつなぎ替える。描くときは囲みの枠で止める。
fn redirect_cluster_edges(g: &mut Graph) {
    let mut remove: Vec<usize> = Vec::new();
    for (ci, c) in g.clusters.iter().enumerate() {
        let Some(ni) = g.nodes.iter().position(|n| n.id == c.id) else { continue };
        let node = &g.nodes[ni];
        if node.label != node.id || node.shape != Shape::Rect {
            continue;
        }
        let inside = |n: &Node| {
            let mut cur = n.cluster;
            while let Some(x) = cur {
                if x == ci {
                    return true;
                }
                cur = g.clusters[x].parent;
            }
            false
        };
        let Some(rep) = g.nodes.iter().enumerate().position(|(i, n)| i != ni && inside(n)) else {
            continue;
        };
        for e in &mut g.edges {
            if e.from == ni {
                e.from = rep;
                e.from_cluster = Some(ci);
            }
            if e.to == ni {
                e.to = rep;
                e.to_cluster = Some(ci);
            }
        }
        remove.push(ni);
    }
    remove.sort_unstable();
    for ni in remove.into_iter().rev() {
        g.nodes.remove(ni);
        for e in &mut g.edges {
            if e.from > ni {
                e.from -= 1;
            }
            if e.to > ni {
                e.to -= 1;
            }
        }
    }
    // 囲みの中から自分の囲みへの線は意味を持たないので捨てる
    g.edges.retain(|e| e.from != e.to || (e.from_cluster.is_none() && e.to_cluster.is_none()));
}

/// `;` で区切る。引用符と括弧の中の `;` は区切らない。
fn split_semicolons(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let (mut depth, mut quoted) = (0i32, false);
    let mut cur = String::new();
    for c in s.chars() {
        match c {
            '"' => quoted = !quoted,
            '[' | '(' | '{' if !quoted => depth += 1,
            ']' | ')' | '}' if !quoted => depth -= 1,
            ';' if !quoted && depth <= 0 => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    out.push(cur);
    out.into_iter().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

const SKIPPED: [&str; 8] =
    ["classdef", "class", "style", "linkstyle", "click", "acctitle", "accdescr", "direction"];

fn parse_flow(lines: &[(usize, String)], dir: Dir) -> Res<Graph> {
    let mut b = GraphBuilder::new(Kind::Flowchart, dir);
    for (n, raw) in lines {
        for stmt in split_semicolons(raw) {
            let n = *n;
            if let Some(rest) = after_keyword(&stmt, "subgraph") {
                let (id, label) = subgraph_title(rest, b.g.clusters.len());
                b.open_cluster(id, label);
                continue;
            }
            if stmt.eq_ignore_ascii_case("end") {
                if b.stack.pop().is_none() {
                    return err(n, "対応する subgraph のない end があります");
                }
                continue;
            }
            let first = stmt
                .split(|c: char| c.is_whitespace() || c == ':')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase();
            if SKIPPED.contains(&first.as_str()) || stmt.starts_with("%%{") {
                continue;
            }
            parse_chain(&mut b, n, &stmt)?;
        }
    }
    if !b.stack.is_empty() {
        let line = lines.last().map_or(1, |l| l.0);
        return err(line, "subgraph が end で閉じられていません");
    }
    let mut g = b.g;
    redirect_cluster_edges(&mut g);
    Ok(g)
}

fn subgraph_title(rest: &str, ordinal: usize) -> (String, String) {
    let rest = rest.trim();
    if rest.is_empty() {
        return (format!("subgraph{ordinal}"), String::new());
    }
    if rest.starts_with('"') {
        let label = clean_label(rest);
        return (format!("subgraph{ordinal}"), label);
    }
    if let Some(open) = rest.find('[')
        && rest.ends_with(']')
    {
        let id = rest[..open].trim().to_string();
        let label = clean_label(&rest[open + 1..rest.len() - 1]);
        return (id, label);
    }
    (rest.split_whitespace().next().unwrap_or(rest).to_string(), clean_label(rest))
}

struct Cursor<'a> {
    chars: Vec<char>,
    pos: usize,
    line: usize,
    src: &'a str,
}

impl Cursor<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }
    fn peek_at(&self, k: usize) -> Option<char> {
        self.chars.get(self.pos + k).copied()
    }
    fn eof(&self) -> bool {
        self.pos >= self.chars.len()
    }
    fn skip_ws(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.pos += 1;
        }
    }
    fn rest(&self) -> String {
        self.chars[self.pos.min(self.chars.len())..].iter().collect()
    }
    fn starts_with(&self, s: &str) -> bool {
        s.chars().enumerate().all(|(i, c)| self.peek_at(i) == Some(c))
    }
    fn fail<T>(&self, message: &str) -> Res<T> {
        err(self.line, format!("{message}: 「{}」", self.src.trim()))
    }
}

fn is_id_char(c: char) -> bool {
    c.is_alphanumeric()
        || matches!(c, '_' | '.' | '-' | '$' | '/' | '\'' | '#' | '!' | '?' | '+' | '*')
}

fn parse_chain(b: &mut GraphBuilder, line: usize, stmt: &str) -> Res<()> {
    let mut c = Cursor { chars: stmt.chars().collect(), pos: 0, line, src: stmt };
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut ops: Vec<EdgeOp> = Vec::new();
    loop {
        let mut group = vec![parse_node_ref(b, &mut c)?];
        loop {
            c.skip_ws();
            if c.peek() == Some('&') {
                c.pos += 1;
                group.push(parse_node_ref(b, &mut c)?);
            } else {
                break;
            }
        }
        groups.push(group);
        c.skip_ws();
        if c.eof() {
            break;
        }
        ops.push(parse_edge_op(&mut c)?);
    }
    if groups.len() != ops.len() + 1 {
        return c.fail("矢印の先に要素がありません");
    }
    for (i, op) in ops.iter().enumerate() {
        for &from in &groups[i] {
            for &to in &groups[i + 1] {
                let mut e = plain_edge(from, to);
                e.label = op.label.clone();
                e.line = op.line;
                e.start = op.start;
                e.end = op.end;
                b.edge(line, e)?;
            }
        }
    }
    Ok(())
}

fn parse_node_ref(b: &mut GraphBuilder, c: &mut Cursor) -> Res<usize> {
    c.skip_ws();
    let start = c.pos;
    while let Some(ch) = c.peek() {
        if ch == '-' && matches!(c.peek_at(1), Some('-' | '.' | '>' | '=')) {
            break;
        }
        if !is_id_char(ch) {
            break;
        }
        c.pos += 1;
    }
    let id: String = c.chars[start..c.pos].iter().collect();
    if id.is_empty() {
        return c.fail("要素の名前がありません");
    }
    let shapes: [(&str, &str, Shape); 13] = [
        ("(((", ")))", Shape::Circle),
        ("((", "))", Shape::Circle),
        ("([", "])", Shape::Stadium),
        ("[[", "]]", Shape::Subroutine),
        ("[(", ")]", Shape::Cylinder),
        ("[/", "/]", Shape::Parallelogram),
        ("[\\", "\\]", Shape::Parallelogram),
        ("{{", "}}", Shape::Hexagon),
        ("(", ")", Shape::Round),
        ("[", "]", Shape::Rect),
        ("{", "}", Shape::Diamond),
        (">", "]", Shape::Flag),
        ("@{", "}", Shape::Rect),
    ];
    let mut label = None;
    let mut shape = None;
    for (open, close, s) in shapes {
        if !c.starts_with(open) {
            continue;
        }
        c.pos += open.chars().count();
        let text = read_until_closer(c, open, close)?;
        if open == "@{" {
            // 新しい書き方の形の指定は読み飛ばす
            break;
        }
        label = Some(clean_label(&text));
        shape = Some(s);
        break;
    }
    // :::class は読み飛ばす
    if c.starts_with(":::") {
        c.pos += 3;
        while c.peek().is_some_and(is_id_char) {
            c.pos += 1;
        }
    }
    let line = c.line;
    b.node(line, &id, label, shape)
}

/// 閉じ記号まで読む。引用符の中と、同じ種類の括弧の入れ子は飛ばす。
fn read_until_closer(c: &mut Cursor, open: &str, close: &str) -> Res<String> {
    let mut out = String::new();
    let mut quoted = false;
    let opener = open.chars().last().unwrap_or('[');
    let closer = close.chars().next().unwrap_or(']');
    let mut depth = 0;
    while !c.eof() {
        let ch = c.peek().unwrap_or(' ');
        if ch == '"' {
            quoted = !quoted;
        } else if !quoted {
            if c.starts_with(close) && depth == 0 {
                c.pos += close.chars().count();
                return Ok(out);
            }
            if close.len() == 1 && ch == opener && opener != closer {
                depth += 1;
            } else if close.len() == 1 && ch == closer && depth > 0 {
                depth -= 1;
            }
        }
        out.push(ch);
        c.pos += 1;
    }
    c.fail(&format!("「{close}」で閉じられていません"))
}

struct EdgeOp {
    line: Line,
    start: Marker,
    end: Marker,
    label: String,
}

fn line_of(body: &str) -> Line {
    if body.contains('~') {
        Line::Invisible
    } else if body.contains('=') {
        Line::Thick
    } else if body.contains('.') {
        Line::Dotted
    } else {
        Line::Solid
    }
}

fn read_body(c: &mut Cursor) -> String {
    let mut body = String::new();
    while let Some(ch) = c.peek() {
        if matches!(ch, '-' | '=' | '.' | '~') {
            body.push(ch);
            c.pos += 1;
        } else {
            break;
        }
    }
    body
}

fn read_end_marker(c: &mut Cursor) -> Marker {
    match c.peek() {
        Some('>') => {
            c.pos += 1;
            Marker::Arrow
        }
        Some(m @ ('x' | 'o')) if c.peek_at(1).is_none_or(|n| n.is_whitespace() || n == '|') => {
            c.pos += 1;
            if m == 'x' { Marker::Cross } else { Marker::Circle }
        }
        _ => Marker::None,
    }
}

fn parse_edge_op(c: &mut Cursor) -> Res<EdgeOp> {
    let mut start = Marker::None;
    match c.peek() {
        Some('<') => {
            start = Marker::Arrow;
            c.pos += 1;
        }
        Some(m @ ('x' | 'o')) if matches!(c.peek_at(1), Some('-' | '=' | '.')) => {
            start = if m == 'x' { Marker::Cross } else { Marker::Circle };
            c.pos += 1;
        }
        _ => {}
    }
    let body = read_body(c);
    if body.is_empty() {
        return c.fail("矢印（--> など）が必要です");
    }
    let mut line = line_of(&body);
    let mut end = read_end_marker(c);
    let mut label = String::new();
    // `A -- 文字 --> B` の形
    if end == Marker::None
        && matches!(body.as_str(), "--" | "==" | "-.")
        && c.peek().is_some_and(char::is_whitespace)
    {
        let rest = c.rest();
        const CLOSERS: [&str; 12] =
            ["-->", "---", "==>", "===", ".->", "-.-", "--x", "--o", "==x", "==o", "~~~", ".-"];
        let found = CLOSERS
            .iter()
            .filter_map(|p| rest.find(p).map(|at| (at, *p)))
            .min_by_key(|(at, p)| (*at, std::cmp::Reverse(p.len())));
        let Some((at, _)) = found else {
            return c.fail("矢印の文字が閉じられていません");
        };
        label = clean_label(&rest[..at]);
        c.pos += rest[..at].chars().count();
        let closing = read_body(c);
        line = line_of(&format!("{body}{closing}"));
        end = read_end_marker(c);
    }
    c.skip_ws();
    if c.peek() == Some('|') {
        c.pos += 1;
        let mut text = String::new();
        while let Some(ch) = c.peek() {
            c.pos += 1;
            if ch == '|' {
                break;
            }
            text.push(ch);
        }
        label = clean_label(&text);
    }
    Ok(EdgeOp { line, start, end, label })
}

// ───────────────────────── クラス図 ─────────────────────────

fn split_label(s: &str) -> (&str, Option<String>) {
    // 引用符の外の最初の `:` で分ける
    let mut quoted = false;
    for (i, ch) in s.char_indices() {
        match ch {
            '"' => quoted = !quoted,
            ':' if !quoted => return (s[..i].trim(), Some(clean_label(&s[i + 1..]))),
            _ => {}
        }
    }
    (s.trim(), None)
}

fn class_name(raw: &str) -> (String, Option<String>) {
    let raw = raw.trim();
    let raw = raw.split(":::").next().unwrap_or(raw).trim();
    if let Some(open) = raw.find('[')
        && raw.ends_with(']')
    {
        return (raw[..open].trim().to_string(), Some(clean_label(&raw[open + 1..raw.len() - 1])));
    }
    (raw.trim_matches('`').to_string(), None)
}

fn add_member(node: &mut Node, raw: &str) {
    let m = generics(raw.trim()).trim_end_matches(['$', '*']).trim().to_string();
    if m.is_empty() {
        return;
    }
    if m.starts_with("<<") && m.ends_with(">>") {
        node.stereotype = Some(m.trim_matches(['<', '>']).trim().to_string());
        return;
    }
    while node.sections.len() < 2 {
        node.sections.push(Vec::new());
    }
    let at = usize::from(m.contains('('));
    node.sections[at].push(m.chars().take(MAX_LABEL).collect());
}

struct Relation<'a> {
    left: &'a str,
    right: &'a str,
    start: Marker,
    end: Marker,
    line: Line,
}

fn class_relation(s: &str) -> Option<Relation<'_>> {
    let bytes = s.as_bytes();
    let at = s.find("--").or_else(|| s.find(".."))?;
    let line = if &s[at..at + 2] == ".." { Line::Dotted } else { Line::Solid };
    let mut lo = at;
    let mut hi = at + 2;
    let is_word = |i: usize| bytes.get(i).is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_');
    let start = if s[..lo].ends_with("<|") {
        lo -= 2;
        Marker::Triangle
    } else if s[..lo].ends_with('*') {
        lo -= 1;
        Marker::Diamond
    } else if s[..lo].ends_with('o') && !(lo >= 2 && is_word(lo - 2)) {
        lo -= 1;
        Marker::DiamondOpen
    } else if s[..lo].ends_with('<') {
        lo -= 1;
        Marker::Arrow
    } else {
        Marker::None
    };
    let tail = &s[hi..];
    let end = if tail.starts_with("|>") {
        hi += 2;
        Marker::Triangle
    } else if tail.starts_with('*') {
        hi += 1;
        Marker::Diamond
    } else if tail.starts_with('o') && !is_word(hi + 1) {
        hi += 1;
        Marker::DiamondOpen
    } else if tail.starts_with('>') {
        hi += 1;
        Marker::Arrow
    } else {
        Marker::None
    };
    Some(Relation { left: s[..lo].trim(), right: s[hi..].trim(), start, end, line })
}

/// `Name "1"` から名前と多重度を取り出す。
fn with_cardinality(s: &str, quoted_first: bool) -> (String, String) {
    let s = s.trim();
    let parts: Vec<&str> = s.split('"').collect();
    if parts.len() >= 3 {
        let card = parts[1].trim().to_string();
        let name = if quoted_first { parts[2] } else { parts[0] };
        return (name.trim().to_string(), card);
    }
    (s.to_string(), String::new())
}

fn parse_class(lines: &[(usize, String)]) -> Res<Graph> {
    let mut b = GraphBuilder::new(Kind::Class, Dir::TopDown);
    let mut body_of: Option<usize> = None;
    for (n, stmt) in lines {
        let n = *n;
        let stmt = stmt.as_str();
        if let Some(target) = body_of {
            if stmt.starts_with('}') {
                body_of = None;
            } else {
                add_member(&mut b.g.nodes[target], stmt);
            }
            continue;
        }
        if let Some(rest) = after_keyword(stmt, "direction") {
            if let Some(d) = parse_dir(rest) {
                b.g.dir = d;
            }
            continue;
        }
        let first = stmt.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
        if matches!(
            first.as_str(),
            "note"
                | "click"
                | "link"
                | "callback"
                | "cssclass"
                | "style"
                | "classdef"
                | "namespace"
                | "}"
        ) {
            continue;
        }
        if stmt.starts_with("<<") {
            if let Some(close) = stmt.find(">>") {
                let stereo = stmt[2..close].trim().to_string();
                let (name, label) = class_name(&stmt[close + 2..]);
                if !name.is_empty() {
                    let i = b.node(
                        n,
                        &name,
                        label.map(|l| generics(&l)).or_else(|| Some(generics(&name))),
                        None,
                    )?;
                    b.g.nodes[i].stereotype = Some(stereo);
                }
            }
            continue;
        }
        if let Some(rest) = after_keyword(stmt, "class") {
            let (head, inline) = match rest.find('{') {
                Some(at) => (&rest[..at], Some(&rest[at + 1..])),
                None => (rest, None),
            };
            let (name, label) = class_name(head);
            if name.is_empty() {
                return err(n, "クラス名がありません");
            }
            let shown = label.unwrap_or_else(|| generics(&name));
            let i = b.node(n, &name, Some(shown), None)?;
            if let Some(inline) = inline {
                match inline.find('}') {
                    Some(close) => {
                        for m in inline[..close].split(';') {
                            add_member(&mut b.g.nodes[i], m);
                        }
                    }
                    None => {
                        add_member(&mut b.g.nodes[i], inline);
                        body_of = Some(i);
                    }
                }
            }
            continue;
        }
        let (core, label) = split_label(stmt);
        if let Some(rel) = class_relation(core) {
            let (left, start_label) = with_cardinality(rel.left, false);
            let (right, end_label) = with_cardinality(rel.right, true);
            let (left, _) = class_name(&left);
            let (right, _) = class_name(&right);
            if left.is_empty() || right.is_empty() {
                return err(n, format!("関係の両端にクラス名が必要です: 「{stmt}」"));
            }
            let from =
                b.node(n, &left, (!b.ids.contains_key(&left)).then(|| generics(&left)), None)?;
            let to =
                b.node(n, &right, (!b.ids.contains_key(&right)).then(|| generics(&right)), None)?;
            let upward =
                matches!(rel.end, Marker::Triangle | Marker::Diamond | Marker::DiamondOpen)
                    && !matches!(
                        rel.start,
                        Marker::Triangle | Marker::Diamond | Marker::DiamondOpen
                    );
            let mut e = plain_edge(from, to);
            e.start = rel.start;
            e.end = rel.end;
            e.line = rel.line;
            e.label = label.unwrap_or_default();
            e.start_label = start_label;
            e.end_label = end_label;
            e.upward = upward;
            b.edge(n, e)?;
            continue;
        }
        if let Some(member) = label {
            let (name, _) = class_name(core);
            let i =
                b.node(n, &name, (!b.ids.contains_key(&name)).then(|| generics(&name)), None)?;
            add_member(&mut b.g.nodes[i], &member);
            continue;
        }
        let (name, label) = class_name(core);
        if name.chars().all(|c| is_id_char(c) || c == '~') && !name.is_empty() {
            b.node(n, &name, label.or_else(|| Some(generics(&name))), None)?;
            continue;
        }
        return err(n, format!("クラス図の行を読めません: 「{stmt}」"));
    }
    Ok(b.g)
}

// ───────────────────────── ER 図 ─────────────────────────

fn er_marker(s: &str, left: bool) -> Option<Marker> {
    Some(match (s, left) {
        ("||", _) => Marker::One,
        ("|o", true) | ("o|", false) => Marker::ZeroOrOne,
        ("}|", true) | ("|{", false) => Marker::OneOrMany,
        ("}o", true) | ("o{", false) => Marker::ZeroOrMany,
        _ => return None,
    })
}

fn er_relation(s: &str) -> Option<(String, String, Marker, Marker, Line)> {
    let chars: Vec<char> = s.chars().collect();
    for i in 0..chars.len().saturating_sub(5) {
        let part = |a: usize| -> String { chars[i + a..i + a + 2].iter().collect() };
        let (l, mid, r) = (part(0), part(2), part(4));
        if !matches!(mid.as_str(), "--" | "..") {
            continue;
        }
        if let (Some(start), Some(end)) = (er_marker(&l, true), er_marker(&r, false)) {
            let left: String = chars[..i].iter().collect();
            let right: String = chars[i + 6..].iter().collect();
            let line = if mid == ".." { Line::Dotted } else { Line::Solid };
            return Some((left.trim().to_string(), right.trim().to_string(), start, end, line));
        }
    }
    None
}

fn er_row(raw: &str) -> String {
    // 型 名前 キー "注釈" を、列を揃えて描けるようタブで区切る
    let (main, comment) = match raw.find('"') {
        Some(at) => (&raw[..at], Some(clean_label(&raw[at..]))),
        None => (raw, None),
    };
    let mut parts: Vec<String> = main.split_whitespace().map(generics).collect();
    if parts.len() > 3 {
        let keys = parts.split_off(2).join(" ");
        parts.push(keys);
    }
    let mut row = parts.join("\t");
    if let Some(c) = comment.filter(|c| !c.is_empty()) {
        row.push('\t');
        row.push_str(&c);
    }
    row.chars().take(MAX_LABEL).collect()
}

fn parse_er(lines: &[(usize, String)]) -> Res<Graph> {
    let mut b = GraphBuilder::new(Kind::Entity, Dir::TopDown);
    let mut body_of: Option<usize> = None;
    for (n, stmt) in lines {
        let n = *n;
        let stmt = stmt.as_str();
        if let Some(target) = body_of {
            if stmt.starts_with('}') {
                body_of = None;
            } else {
                let node = &mut b.g.nodes[target];
                if node.sections.is_empty() {
                    node.sections.push(Vec::new());
                }
                node.sections[0].push(er_row(stmt));
            }
            continue;
        }
        if let Some(rest) = after_keyword(stmt, "direction") {
            if let Some(d) = parse_dir(rest) {
                b.g.dir = d;
            }
            continue;
        }
        let first = stmt.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
        if matches!(first.as_str(), "style" | "classdef" | "class") {
            continue;
        }
        let (core, label) = split_label(stmt);
        if let Some((left, right, start, end, line)) = er_relation(core) {
            let (l, ll) = class_name(&left);
            let (r, rl) = class_name(&right);
            if l.is_empty() || r.is_empty() {
                return err(n, format!("関係の両端にエンティティ名が必要です: 「{stmt}」"));
            }
            let from = b.node(n, &l, ll, None)?;
            let to = b.node(n, &r, rl, None)?;
            let mut e = plain_edge(from, to);
            e.start = start;
            e.end = end;
            e.line = line;
            e.label = label.unwrap_or_default();
            b.edge(n, e)?;
            continue;
        }
        if let Some(at) = stmt.find('{') {
            let (name, label) = class_name(&stmt[..at]);
            if name.is_empty() {
                return err(n, "エンティティ名がありません");
            }
            let i = b.node(n, &name, label, None)?;
            let rest = stmt[at + 1..].trim();
            if let Some(close) = rest.find('}') {
                for row in rest[..close].split(['\n', ',']).filter(|r| !r.trim().is_empty()) {
                    let node = &mut b.g.nodes[i];
                    if node.sections.is_empty() {
                        node.sections.push(Vec::new());
                    }
                    node.sections[0].push(er_row(row));
                }
            } else {
                body_of = Some(i);
            }
            continue;
        }
        let (name, label) = class_name(stmt);
        if !name.is_empty() && name.chars().all(is_id_char) {
            b.node(n, &name, label, None)?;
            continue;
        }
        return err(n, format!("ER 図の行を読めません: 「{stmt}」"));
    }
    Ok(b.g)
}

// ───────────────────────── 状態遷移図 ─────────────────────────

struct Transition {
    line: usize,
    from: String,
    to: String,
    label: String,
    scope: Option<usize>,
}

fn parse_state(lines: &[(usize, String)]) -> Res<Graph> {
    let mut b = GraphBuilder::new(Kind::State, Dir::TopDown);
    let mut transitions: Vec<Transition> = Vec::new();
    let mut composite: HashMap<String, usize> = HashMap::new();
    let mut in_note = false;
    for (n, stmt) in lines {
        let n = *n;
        let stmt = stmt.as_str();
        if in_note {
            if starts_with_ci(stmt, "end note") {
                in_note = false;
            }
            continue;
        }
        if let Some(rest) = after_keyword(stmt, "direction") {
            if let Some(d) = parse_dir(rest) {
                b.g.dir = d;
            }
            continue;
        }
        if starts_with_ci(stmt, "note ") {
            if !stmt.contains(':') {
                in_note = true;
            }
            continue;
        }
        let first = stmt.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
        if matches!(first.as_str(), "classdef" | "class" | "style" | "--" | "||") {
            continue;
        }
        if stmt == "}" {
            if b.stack.pop().is_none() {
                return err(n, "対応する state { のない } があります");
            }
            continue;
        }
        if let Some(rest) = after_keyword(stmt, "state") {
            let opens = rest.ends_with('{');
            let rest = rest.trim_end_matches('{').trim();
            let (id, label, kind) = state_decl(rest);
            if id.is_empty() {
                return err(n, "状態の名前がありません");
            }
            if opens {
                b.open_cluster(id.clone(), label.unwrap_or_else(|| id.clone()));
                composite.insert(id, b.g.clusters.len() - 1);
            } else {
                let shape = match kind.as_deref() {
                    Some("choice") => Some(Shape::Diamond),
                    Some("fork" | "join") => Some(Shape::Bar),
                    _ => None,
                };
                let label = if shape == Some(Shape::Bar) || shape == Some(Shape::Diamond) {
                    Some(String::new())
                } else {
                    label
                };
                b.node(n, &id, label, shape)?;
            }
            continue;
        }
        if let Some(at) = stmt.find("-->") {
            let from = stmt[..at].trim().to_string();
            let (to, label) = split_label(&stmt[at + 3..]);
            if from.is_empty() || to.is_empty() {
                return err(n, format!("遷移の両端に状態が必要です: 「{stmt}」"));
            }
            transitions.push(Transition {
                line: n,
                from,
                to: to.to_string(),
                label: label.unwrap_or_default(),
                scope: b.stack.last().copied(),
            });
            continue;
        }
        let (id, label) = split_label(stmt);
        if !id.is_empty() && id.chars().all(is_id_char) {
            let i = b.node(n, id, None, None)?;
            if let Some(desc) = label {
                let node = &mut b.g.nodes[i];
                node.label =
                    if node.label == node.id { desc } else { format!("{}\n{desc}", node.label) };
            }
            continue;
        }
        return err(n, format!("状態遷移図の行を読めません: 「{stmt}」"));
    }
    if !b.stack.is_empty() {
        return err(lines.last().map_or(1, |l| l.0), "state { が } で閉じられていません");
    }
    for t in transitions {
        let saved = std::mem::take(&mut b.stack);
        b.stack = t.scope.into_iter().collect();
        let from = state_end(&mut b, &composite, &t.from, t.scope, t.line, true)?;
        let to = state_end(&mut b, &composite, &t.to, t.scope, t.line, false)?;
        b.stack = saved;
        let mut e = plain_edge(from, to);
        e.label = t.label;
        b.edge(t.line, e)?;
    }
    Ok(b.g)
}

fn state_decl(rest: &str) -> (String, Option<String>, Option<String>) {
    let (rest, kind) = match rest.find("<<") {
        Some(at) => (
            rest[..at].trim(),
            Some(rest[at + 2..].trim_end_matches(">>").trim().to_ascii_lowercase()),
        ),
        None => (rest.trim(), None),
    };
    if rest.starts_with('"')
        && let Some(as_at) = rest.rfind(" as ")
    {
        return (rest[as_at + 4..].trim().to_string(), Some(clean_label(&rest[..as_at])), kind);
    }
    if let Some((id, desc)) = rest.split_once(':') {
        return (id.trim().to_string(), Some(clean_label(desc)), kind);
    }
    (rest.to_string(), None, kind)
}

/// 遷移の端を要素にする。`[*]` は囲みごとの始点と終点、複合状態はその中の始点か終点へつなぐ。
fn state_end(
    b: &mut GraphBuilder,
    composite: &HashMap<String, usize>,
    raw: &str,
    scope: Option<usize>,
    line: usize,
    outgoing: bool,
) -> Res<usize> {
    if raw == "[*]" {
        let (id, shape) = if outgoing {
            (format!("[*]start:{scope:?}"), Shape::Start)
        } else {
            (format!("[*]end:{scope:?}"), Shape::End)
        };
        return b.node(line, &id, Some(String::new()), Some(shape));
    }
    if let Some(&cluster) = composite.get(raw) {
        // 入るときは中の始点へ、出るときは中の終点（無ければ最後の状態）から
        let inner = Some(cluster);
        let wanted =
            if outgoing { format!("[*]end:{inner:?}") } else { format!("[*]start:{inner:?}") };
        if let Some(&i) = b.ids.get(&wanted) {
            return Ok(i);
        }
        if let Some(i) = b.g.nodes.iter().rposition(|n| n.cluster == inner) {
            return Ok(i);
        }
        let saved = std::mem::replace(&mut b.stack, vec![cluster]);
        let i = b.node(
            line,
            &wanted,
            Some(String::new()),
            Some(if outgoing { Shape::End } else { Shape::Start }),
        );
        b.stack = saved;
        return i;
    }
    b.node(line, raw, None, None)
}

// ───────────────────────── シーケンス図 ─────────────────────────

const ARROWS: [(&str, Line, Marker); 10] = [
    ("<<-->>", Line::Dotted, Marker::Arrow),
    ("<<->>", Line::Solid, Marker::Arrow),
    ("-->>", Line::Dotted, Marker::Arrow),
    ("->>", Line::Solid, Marker::Arrow),
    ("--x", Line::Dotted, Marker::Cross),
    ("-x", Line::Solid, Marker::Cross),
    ("--)", Line::Dotted, Marker::OpenArrow),
    ("-)", Line::Solid, Marker::OpenArrow),
    ("-->", Line::Dotted, Marker::None),
    ("->", Line::Solid, Marker::None),
];

struct SeqBuilder {
    s: Sequence,
    ids: HashMap<String, usize>,
}

impl SeqBuilder {
    fn participant(
        &mut self,
        line: usize,
        id: &str,
        label: Option<String>,
        actor: bool,
    ) -> Res<usize> {
        let id = id.trim();
        if id.is_empty() {
            return err(line, "参加者の名前がありません");
        }
        if let Some(&i) = self.ids.get(id) {
            if let Some(l) = label {
                self.s.participants[i].label = l;
            }
            self.s.participants[i].actor |= actor;
            return Ok(i);
        }
        if self.s.participants.len() >= 24 {
            return err(line, "参加者が多すぎます（24 まで）");
        }
        self.s.participants.push(Participant {
            id: id.to_string(),
            label: label.unwrap_or_else(|| clean_label(id)),
            actor,
        });
        self.ids.insert(id.to_string(), self.s.participants.len() - 1);
        Ok(self.s.participants.len() - 1)
    }
}

fn parse_sequence(lines: &[(usize, String)]) -> Res<Sequence> {
    let mut b = SeqBuilder {
        s: Sequence { participants: Vec::new(), items: Vec::new(), autonumber: false },
        ids: HashMap::new(),
    };
    // box は参加者をまとめるだけなので描かない。対応する end も捨てる
    let mut blocks: Vec<bool> = Vec::new();
    for (n, stmt) in lines {
        let n = *n;
        let stmt = stmt.trim_start_matches("create ").trim();
        if b.s.items.len() >= MAX_EDGES {
            return err(n, "メッセージが多すぎます");
        }
        let first = stmt.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
        match first.as_str() {
            "autonumber" => {
                b.s.autonumber = true;
                continue;
            }
            "participant" | "actor" => {
                let rest = stmt[first.len()..].trim();
                let (id, label) = match rest.split_once(" as ") {
                    Some((id, label)) => (id.trim(), Some(clean_label(label))),
                    None => (rest, None),
                };
                b.participant(n, id, label, first == "actor")?;
                continue;
            }
            "activate" | "deactivate" | "destroy" | "title" | "link" | "links" | "properties"
            | "details" | "acctitle" | "accdescr" => continue,
            "box" => {
                blocks.push(false);
                continue;
            }
            "loop" | "alt" | "opt" | "par" | "critical" | "break" | "rect" => {
                let label = if first == "rect" {
                    String::new()
                } else {
                    clean_label(stmt[first.len()..].trim())
                };
                blocks.push(true);
                b.s.items.push(SeqItem::Open { kind: first.clone(), label });
                continue;
            }
            "else" | "and" | "option" => {
                b.s.items.push(SeqItem::Divide { label: clean_label(stmt[first.len()..].trim()) });
                continue;
            }
            "end" => {
                match blocks.pop() {
                    Some(true) => b.s.items.push(SeqItem::Close),
                    Some(false) => {}
                    None => return err(n, "対応するブロックのない end があります"),
                }
                continue;
            }
            "note" => {
                let rest = stmt[4..].trim();
                let (head, text) = match rest.split_once(':') {
                    Some((h, t)) => (h.trim(), clean_label(t)),
                    None => (rest, String::new()),
                };
                let (side, targets) = if let Some(t) = after_keyword(head, "left of") {
                    (NoteSide::Left, t)
                } else if let Some(t) = after_keyword(head, "right of") {
                    (NoteSide::Right, t)
                } else if let Some(t) = after_keyword(head, "over") {
                    (NoteSide::Over, t)
                } else {
                    return err(n, format!("注釈の位置を読めません: 「{stmt}」"));
                };
                let ids: Vec<&str> = targets.split(',').map(str::trim).collect();
                let first_p = b.participant(n, ids[0], None, false)?;
                let last_p = match ids.get(1) {
                    Some(id) => b.participant(n, id, None, false)?,
                    None => first_p,
                };
                b.s.items.push(SeqItem::Note {
                    side,
                    first: first_p.min(last_p),
                    last: first_p.max(last_p),
                    text,
                });
                continue;
            }
            _ => {}
        }
        let (head, text) = match stmt.split_once(':') {
            Some((h, t)) => (h, clean_label(t)),
            None => (stmt, String::new()),
        };
        let found = ARROWS
            .iter()
            .filter_map(|(a, l, m)| head.find(a).map(|at| (at, *a, *l, *m)))
            .min_by_key(|(at, a, ..)| (*at, std::cmp::Reverse(a.len())));
        let Some((at, arrow, line, end)) = found else {
            return err(n, format!("シーケンス図の行を読めません: 「{stmt}」"));
        };
        let from_id = head[..at].trim();
        let to_id = head[at + arrow.len()..].trim().trim_start_matches(['+', '-']).trim();
        let from = b.participant(n, from_id, None, false)?;
        let to = b.participant(n, to_id, None, false)?;
        b.s.items.push(SeqItem::Message { from, to, text, line, end });
    }
    if blocks.iter().any(|v| *v) {
        return err(
            lines.last().map_or(1, |l| l.0),
            "ブロック（loop、alt など）が end で閉じられていません",
        );
    }
    Ok(b.s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(src: &str) -> Graph {
        match parse(src).unwrap() {
            Diagram::Graph(g) => g,
            Diagram::Sequence(_) => panic!("graph expected"),
        }
    }

    #[test]
    fn flowchart_shapes_chains_and_labels() {
        let g = graph(
            "flowchart LR\n  A[\"入口<br/>main.rs\"] -->|起動| B(状態)\n  B --> C{分岐} & D[(DB)]\n  C -- はい --> E([終了])\n  D -.-> E\n  E ==> F((丸))\n  click A href \"http://x\"\n  classDef x fill:#f00\n",
        );
        assert_eq!(g.dir, Dir::LeftRight);
        let label = |id: &str| g.nodes.iter().find(|n| n.id == id).unwrap();
        assert_eq!(label("A").label, "入口\nmain.rs");
        assert_eq!(label("C").shape, Shape::Diamond);
        assert_eq!(label("D").shape, Shape::Cylinder);
        assert_eq!(label("E").shape, Shape::Stadium);
        assert_eq!(label("F").shape, Shape::Circle);
        assert_eq!(g.edges.len(), 6);
        assert_eq!(g.edges[0].label, "起動");
        let yes = g.edges.iter().find(|e| e.label == "はい").unwrap();
        assert_eq!(g.nodes[yes.to].id, "E");
        assert!(g.edges.iter().any(|e| e.line == Line::Dotted));
        assert!(g.edges.iter().any(|e| e.line == Line::Thick));
    }

    #[test]
    fn flowchart_subgraphs_nest_and_assign_nodes() {
        let g = graph(
            "graph TD\nsubgraph core[\"コア\"]\n  a --> b\n  subgraph inner\n    c\n  end\nend\nb --> c\nx-->y;y---z",
        );
        assert_eq!(g.clusters.len(), 2);
        assert_eq!(g.clusters[0].label, "コア");
        assert_eq!(g.clusters[1].parent, Some(0));
        let c = g.nodes.iter().find(|n| n.id == "c").unwrap();
        assert_eq!(c.cluster, Some(1));
        assert!(g.nodes.iter().any(|n| n.id == "z"));
        let open = g.edges.iter().find(|e| g.nodes[e.to].id == "z").unwrap();
        assert_eq!(open.end, Marker::None);
    }

    #[test]
    fn edges_to_subgraphs_attach_to_the_cluster() {
        let g = graph(
            "flowchart TD\nsubgraph client[画面]\n a --> b\nend\nsubgraph shared[共有]\n c\nend\nclient --> shared\nx --> client",
        );
        assert!(!g.nodes.iter().any(|n| n.id == "client" || n.id == "shared"));
        let e = g.edges.iter().find(|e| e.from_cluster == Some(0)).unwrap();
        assert_eq!(e.to_cluster, Some(1));
        assert_eq!(g.nodes[e.to].id, "c");
        assert!(g.edges.iter().any(|e| g.nodes[e.from].id == "x" && e.to_cluster == Some(0)));
    }

    #[test]
    fn flowchart_ids_with_dashes_and_parens_in_labels() {
        let g = graph("flowchart TD\n  api-server[load(path)] --> db_1\n  A-->B");
        assert_eq!(g.nodes[0].id, "api-server");
        assert_eq!(g.nodes[0].label, "load(path)");
        assert_eq!(g.edges.len(), 2);
    }

    #[test]
    fn reports_line_numbers_for_errors() {
        let e = parse("flowchart TD\n  A --> B\n  C -->\n").unwrap_err();
        assert_eq!(e.line, 3);
        let e = parse("flowchart TD\nsubgraph x\nA\n").unwrap_err();
        assert!(e.message.contains("end"));
        assert!(parse("pie title x").is_err());
        assert!(parse("").is_err());
    }

    #[test]
    fn class_diagram_members_and_relations() {
        let g = graph(
            "classDiagram\n  direction LR\n  class Animal {\n    <<abstract>>\n    +String name\n    +speak() void\n  }\n  Animal <|-- Dog\n  Dog \"1\" *-- \"many\" Leg : has\n  Cat --|> Animal\n  Dog ..> Food\n  Dog : +bark()",
        );
        assert_eq!(g.dir, Dir::LeftRight);
        let animal = g.nodes.iter().find(|n| n.id == "Animal").unwrap();
        assert_eq!(animal.stereotype.as_deref(), Some("abstract"));
        assert_eq!(animal.sections[0], ["+String name"]);
        assert_eq!(animal.sections[1], ["+speak() void"]);
        let inherit = &g.edges[0];
        assert_eq!(inherit.start, Marker::Triangle);
        assert!(!inherit.upward);
        let legs = &g.edges[1];
        assert_eq!(
            (legs.start_label.as_str(), legs.end_label.as_str(), legs.label.as_str()),
            ("1", "many", "has")
        );
        assert_eq!(legs.start, Marker::Diamond);
        assert!(g.edges[2].upward);
        assert_eq!(g.edges[3].line, Line::Dotted);
        let dog = g.nodes.iter().find(|n| n.id == "Dog").unwrap();
        assert_eq!(dog.sections[1], ["+bark()"]);
    }

    #[test]
    fn er_diagram_entities_and_cardinality() {
        let g = graph(
            "erDiagram\n  CUSTOMER ||--o{ ORDER : places\n  ORDER ||--|{ LINE_ITEM : \"contains\"\n  CUSTOMER {\n    string name PK \"氏名\"\n    int age\n  }\n  ORDER[\"注文\"] {\n  int id\n  }",
        );
        assert_eq!(g.edges[0].start, Marker::One);
        assert_eq!(g.edges[0].end, Marker::ZeroOrMany);
        assert_eq!(g.edges[1].end, Marker::OneOrMany);
        assert_eq!(g.edges[1].label, "contains");
        let c = g.nodes.iter().find(|n| n.id == "CUSTOMER").unwrap();
        assert_eq!(c.sections[0][0], "string\tname\tPK\t氏名");
        let o = g.nodes.iter().find(|n| n.id == "ORDER").unwrap();
        assert_eq!(o.label, "注文");
    }

    #[test]
    fn state_diagram_start_end_and_composites() {
        let g = graph(
            "stateDiagram-v2\n  [*] --> Idle\n  Idle --> Running : start\n  state Running {\n    [*] --> Reading\n    Reading --> [*]\n  }\n  Running --> Idle : done\n  state choose <<choice>>\n  Idle --> choose\n  note right of Idle : 待機\n",
        );
        assert_eq!(g.clusters.len(), 1);
        let starts = g.nodes.iter().filter(|n| n.shape == Shape::Start).count();
        assert_eq!(starts, 2);
        // 複合状態への遷移は中の始点へつながる
        let into = g.edges.iter().find(|e| e.label == "start").unwrap();
        assert_eq!(g.nodes[into.to].shape, Shape::Start);
        assert_eq!(g.nodes[into.to].cluster, Some(0));
        let out = g.edges.iter().find(|e| e.label == "done").unwrap();
        assert_eq!(g.nodes[out.from].shape, Shape::End);
        assert!(g.nodes.iter().any(|n| n.id == "choose" && n.shape == Shape::Diamond));
    }

    #[test]
    fn sequence_messages_notes_and_blocks() {
        let d = parse(
            "sequenceDiagram\n  autonumber\n  actor U as 利用者\n  participant W as Workspace\n  U->>W: 開く\n  W-->>+C: claude -p\n  loop 出力ごと\n    C--)W: イベント\n  end\n  alt 成功\n    W->>W: 保存\n  else 失敗\n    W-xU: エラー\n  end\n  Note over W,C: 読み取り専用\n  box 外部\n  participant Z\n  end\n",
        )
        .unwrap();
        let Diagram::Sequence(s) = d else { panic!() };
        assert!(s.autonumber);
        assert_eq!(s.participants.len(), 4);
        assert!(s.participants[0].actor);
        assert_eq!(s.participants[0].label, "利用者");
        assert_eq!(s.participants[2].id, "C");
        let messages = s.items.iter().filter(|i| matches!(i, SeqItem::Message { .. })).count();
        assert_eq!(messages, 5);
        assert!(
            s.items.iter().any(|i| matches!(i, SeqItem::Message { end: Marker::OpenArrow, .. }))
        );
        assert!(s.items.iter().any(|i| matches!(i, SeqItem::Message { end: Marker::Cross, .. })));
        assert_eq!(s.items.iter().filter(|i| matches!(i, SeqItem::Close)).count(), 2);
        assert!(s.items.iter().any(|i| matches!(i, SeqItem::Note { first: 1, last: 2, .. })));
    }

    #[test]
    fn labels_drop_html_but_keep_breaks() {
        assert_eq!(clean_label("\"a<br>b <b>c</b>\""), "a\nb c");
        assert_eq!(clean_label("x < y"), "x < y");
        assert_eq!(clean_label("&quot;q&quot;"), "\"q\"");
    }

    #[test]
    fn limits_untrusted_input() {
        let mut src = String::from("flowchart TD\n");
        for i in 0..200 {
            src.push_str(&format!("n{i} --> n{}\n", i + 1));
        }
        assert!(parse(&src).is_err());
        let long = format!("flowchart TD\nA[{}]", "x".repeat(1000));
        let g = graph(&long);
        assert!(g.nodes[0].label.chars().count() <= MAX_LABEL + 1);
    }
}
