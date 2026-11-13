//! 起動引数。`--demo` と `--data-dir` は撮影と検証に使う。
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DemoState {
    Home,
    Fresh,
    Analyzing,
    Done,
    Tour,
    Code,
    /// コードを開き、ファイル内を検索した状態
    Find,
    Search,
    Deep,
    /// 深掘りを実際に実行する。入力を送らずに確認するためのもの
    DeepRun,
    Error,
    Unavailable,
}

impl DemoState {
    fn parse(s: &str) -> Option<DemoState> {
        Some(match s {
            "home" => DemoState::Home,
            "fresh" => DemoState::Fresh,
            "analyzing" => DemoState::Analyzing,
            "done" => DemoState::Done,
            "tour" => DemoState::Tour,
            "code" => DemoState::Code,
            "find" => DemoState::Find,
            "search" => DemoState::Search,
            "deep" => DemoState::Deep,
            "deep-run" => DemoState::DeepRun,
            "error" => DemoState::Error,
            "unavailable" => DemoState::Unavailable,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Args {
    pub data_dir: Option<PathBuf>,
    pub open: Option<PathBuf>,
    pub demo: Option<DemoState>,
    /// 画面を出さずに解析して保存する
    pub analyze: Option<PathBuf>,
    pub model: Option<String>,
}

pub fn parse_args(args: impl IntoIterator<Item = String>) -> Args {
    let mut out = Args::default();
    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--data-dir" => out.data_dir = it.next().map(PathBuf::from),
            "--open" => out.open = it.next().map(PathBuf::from),
            "--demo" => out.demo = it.next().as_deref().and_then(DemoState::parse),
            "--analyze" => out.analyze = it.next().map(PathBuf::from),
            "--model" => out.model = it.next(),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn parses_demo_flags() {
        let a = parse_args(["--data-dir", "/d", "--open", "/r", "--demo", "tour"].map(String::from));
        assert_eq!(a.data_dir.as_deref(), Some(Path::new("/d")));
        assert_eq!(a.open.as_deref(), Some(Path::new("/r")));
        assert_eq!(a.demo, Some(DemoState::Tour));
    }

    #[test]
    fn ignores_unknown_input() {
        let a = parse_args(["--demo", "nope", "--wat"].map(String::from));
        assert_eq!(a, Args::default());
    }
}
