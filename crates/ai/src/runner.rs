//! claude の実行。呼び出し側がバックグラウンドで動かす。
use serde_json::Value;
use std::fmt;
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct ClaudeConfig {
    pub bin: PathBuf,
    /// Finder から起動すると PATH が不足するので、ログインシェルの値を渡す
    pub path_env: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RunEvent {
    /// 読んでいるファイルの絶対パス
    Reading(String),
    /// その他のツール（Grep など）
    Tool(String),
}

#[derive(Clone, Debug, PartialEq)]
pub enum RunError {
    NotInstalled,
    Unsupported,
    Failed(String),
    LimitReached(String),
    TimedOut,
    Cancelled,
}

impl fmt::Display for RunError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RunError::NotInstalled => write!(f, "claude コマンドが見つかりません。Claude Code をインストールしてください。"),
            RunError::Unsupported => write!(f, "Claude Code が古いようです。`claude update` で更新してください。"),
            RunError::Failed(m) => write!(f, "解析に失敗しました: {m}"),
            RunError::LimitReached(m) => write!(f, "利用枠の上限に達しました: {m}"),
            RunError::TimedOut => write!(f, "時間内に解析が終わりませんでした。"),
            RunError::Cancelled => write!(f, "解析を中断しました。"),
        }
    }
}

const STDERR_KEEP: usize = 8 * 1024;

pub fn run_claude(
    cfg: &ClaudeConfig,
    args: Vec<String>,
    timeout: Duration,
    cancel: Arc<AtomicBool>,
    on_event: impl FnMut(RunEvent),
) -> Result<Value, RunError> {
    run_with_log(cfg, args, timeout, cancel, on_event, |_| {})
}

/// `run_claude` と同じ経路で、stdout の全行も `on_line` に渡す。結合テストで使う。
pub fn run_with_log(
    cfg: &ClaudeConfig,
    args: Vec<String>,
    timeout: Duration,
    cancel: Arc<AtomicBool>,
    mut on_event: impl FnMut(RunEvent),
    mut on_line: impl FnMut(&str),
) -> Result<Value, RunError> {
    let workdir = tempfile::tempdir().map_err(|e| RunError::Failed(e.to_string()))?;
    let mut command = Command::new(&cfg.bin);
    command
        .args(&args)
        .current_dir(workdir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(path) = &cfg.path_env {
        command.env("PATH", path);
    }
    let mut child = command.spawn().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => RunError::NotInstalled,
        _ => RunError::Failed(e.to_string()),
    })?;

    let (tx, rx) = mpsc::channel::<String>();
    let stdout = child.stdout.take().expect("stdout を piped にしています");
    thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut stderr = child.stderr.take().expect("stderr を piped にしています");
    let stderr_thread = thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf);
        let start = buf.len().saturating_sub(STDERR_KEEP);
        String::from_utf8_lossy(&buf[start..]).into_owned()
    });

    let deadline = Instant::now() + timeout;
    let mut result: Option<Value> = None;
    let stop = loop {
        if cancel.load(Ordering::Relaxed) {
            break Some(RunError::Cancelled);
        }
        if Instant::now() >= deadline {
            break Some(RunError::TimedOut);
        }
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(line) => {
                on_line(&line);
                let Ok(v) = serde_json::from_str::<Value>(&line) else { continue };
                match v["type"].as_str() {
                    Some("assistant") => report_tools(&v, &mut on_event),
                    Some("result") => result = Some(v),
                    _ => {}
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break None,
        }
    };

    if let Some(error) = stop {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let status = child.wait().map_err(|e| RunError::Failed(e.to_string()))?;
    let stderr_text = stderr_thread.join().unwrap_or_default();

    match result {
        Some(v) if v["is_error"].as_bool() == Some(true) => {
            let message = v["result"].as_str().unwrap_or("不明なエラー").to_string();
            let lower = message.to_lowercase();
            if lower.contains("limit") || lower.contains("quota") {
                Err(RunError::LimitReached(message))
            } else {
                Err(RunError::Failed(message))
            }
        }
        Some(v) => match v.get("structured_output") {
            Some(out) if !out.is_null() => Ok(out.clone()),
            _ => Err(RunError::Failed("結果に structured_output がありません".into())),
        },
        None if !status.success() => Err(RunError::Failed(last_line(&stderr_text))),
        None => Err(RunError::Failed("結果を受け取れませんでした".into())),
    }
}

fn last_line(text: &str) -> String {
    let line = text.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("終了コードが 0 以外です");
    line.trim().to_string()
}

fn report_tools(v: &Value, on_event: &mut impl FnMut(RunEvent)) {
    let Some(content) = v["message"]["content"].as_array() else { return };
    for c in content.iter().filter(|c| c["type"] == "tool_use") {
        let name = c["name"].as_str().unwrap_or_default();
        match c["input"]["file_path"].as_str() {
            Some(path) if name == "Read" => on_event(RunEvent::Reading(path.to_string())),
            _ => on_event(RunEvent::Tool(name.to_string())),
        }
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use std::path::Path;

    pub(crate) fn fake_claude(dir: &Path, script: &str) -> ClaudeConfig {
        use std::os::unix::fs::PermissionsExt;
        let bin = dir.join("claude");
        std::fs::write(&bin, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        ClaudeConfig { bin, path_env: None }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::fake_claude;
    use super::*;

    fn run(cfg: &ClaudeConfig) -> Result<Value, RunError> {
        run_claude(cfg, vec![], Duration::from_secs(5), Default::default(), |_| {})
    }

    #[test]
    fn returns_structured_output_and_reports_progress() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = fake_claude(
            dir.path(),
            r#"
echo '{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/r/a.rs"}},{"type":"tool_use","name":"Grep","input":{}}]}}'
echo 'not json'
echo '{"type":"result","subtype":"success","is_error":false,"structured_output":{"ok":true}}'
"#,
        );
        let mut events = vec![];
        let v = run_claude(&cfg, vec![], Duration::from_secs(5), Default::default(), |e| {
            events.push(e)
        })
        .unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(events, [RunEvent::Reading("/r/a.rs".into()), RunEvent::Tool("Grep".into())]);
    }

    #[test]
    fn runs_in_an_empty_temp_dir_with_stdin_closed() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = fake_claude(
            dir.path(),
            r#"
[ -z "$(ls -A)" ] || exit 9
[ -z "$(cat)" ] || exit 8
echo '{"type":"result","is_error":false,"structured_output":{}}'
"#,
        );
        run(&cfg).unwrap();
    }

    #[test]
    fn maps_failures() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = fake_claude(dir.path(), "echo boom >&2; exit 3");
        assert!(matches!(run(&cfg), Err(RunError::Failed(m)) if m.contains("boom")));
        let cfg = fake_claude(
            dir.path(),
            r#"echo '{"type":"result","is_error":true,"result":"Claude usage limit reached"}'"#,
        );
        assert!(matches!(run(&cfg), Err(RunError::LimitReached(_))));
        let cfg = fake_claude(dir.path(), r#"echo '{"type":"result","is_error":false}'"#);
        assert!(matches!(run(&cfg), Err(RunError::Failed(_))));
        let cfg = ClaudeConfig { bin: "/nonexistent/claude".into(), path_env: None };
        assert!(matches!(run(&cfg), Err(RunError::NotInstalled)));
    }

    #[test]
    fn timeout_and_cancel_kill_the_process() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = fake_claude(dir.path(), "sleep 30");
        let start = Instant::now();
        assert_eq!(
            run_claude(&cfg, vec![], Duration::from_millis(200), Default::default(), |_| {}),
            Err(RunError::TimedOut)
        );
        let cancel = Arc::new(AtomicBool::new(true));
        assert_eq!(
            run_claude(&cfg, vec![], Duration::from_secs(5), cancel, |_| {}),
            Err(RunError::Cancelled)
        );
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn large_stderr_does_not_block() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = fake_claude(
            dir.path(),
            r#"
i=0; while [ $i -lt 20000 ]; do echo 'xxxxxxxxxxxxxxxxxxxxxxxx' >&2; i=$((i+1)); done
echo '{"type":"result","is_error":false,"structured_output":{}}'
"#,
        );
        assert!(run(&cfg).is_ok());
    }
}
