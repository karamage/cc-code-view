//! 解析の実行。出力が不正なときだけ 1 回再試行する。
use crate::command::claude_args;
use crate::prompt::{analysis_prompt, deep_dive_prompt, skeleton};
use crate::runner::{ClaudeConfig, RunError, RunEvent, run_claude};
use crate::schema::{
    ANALYSIS_SCHEMA, DEEP_DIVE_SCHEMA, InvalidOutput, validate_analysis, validate_deep_dive,
};
use ccv_core::model::{Analysis, DeepDive, ModelChoice, Module, RepoIndex};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

pub const ANALYSIS_TIMEOUT: Duration = Duration::from_secs(600);
const SKELETON_BUDGET: usize = 60_000;

#[allow(clippy::too_many_arguments)]
fn with_retry<T>(
    cfg: &ClaudeConfig,
    repo: &Path,
    schema: &str,
    model: ModelChoice,
    cancel: Arc<AtomicBool>,
    on_event: &mut impl FnMut(RunEvent),
    prompt: impl Fn(Option<&str>) -> String,
    validate: impl Fn(&Value) -> Result<T, InvalidOutput>,
) -> Result<T, RunError> {
    let mut note: Option<String> = None;
    loop {
        let args = claude_args(&prompt(note.as_deref()), schema, repo, model);
        let raw = run_claude(cfg, args, ANALYSIS_TIMEOUT, cancel.clone(), &mut *on_event)?;
        match validate(&raw) {
            Ok(value) => return Ok(value),
            Err(InvalidOutput(reason)) if note.is_none() => note = Some(reason),
            Err(InvalidOutput(reason)) => {
                return Err(RunError::Failed(format!("出力を検証できませんでした: {reason}")));
            }
        }
    }
}

pub fn analyze_repo(
    cfg: &ClaudeConfig,
    repo: &Path,
    index: &RepoIndex,
    model: ModelChoice,
    cancel: Arc<AtomicBool>,
    mut on_event: impl FnMut(RunEvent),
) -> Result<Analysis, RunError> {
    let skel = skeleton(index, SKELETON_BUDGET);
    with_retry(
        cfg,
        repo,
        ANALYSIS_SCHEMA,
        model,
        cancel,
        &mut on_event,
        |note| analysis_prompt(repo, &skel, note),
        |raw| validate_analysis(raw, index, model),
    )
}

pub fn deep_dive(
    cfg: &ClaudeConfig,
    repo: &Path,
    index: &RepoIndex,
    module: &Module,
    model: ModelChoice,
    cancel: Arc<AtomicBool>,
    mut on_event: impl FnMut(RunEvent),
) -> Result<DeepDive, RunError> {
    with_retry(
        cfg,
        repo,
        DEEP_DIVE_SCHEMA,
        model,
        cancel,
        &mut on_event,
        |note| deep_dive_prompt(repo, module, note),
        |raw| validate_deep_dive(raw, index, &module.id, model),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::testing::fake_claude;
    use ccv_core::model::{FileEntry, Lang};

    fn index() -> RepoIndex {
        RepoIndex {
            head: Some("sha".into()),
            files: vec![FileEntry {
                path: "src/a.rs".into(),
                lang: Lang::Rust,
                lines: 100,
                bytes: 0,
                binary: false,
                churn: 0,
            }],
            symbols: Default::default(),
            edges: vec![],
        }
    }

    #[test]
    fn retries_once_on_invalid_output_then_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let count = dir.path().join("count");
        let script = format!(
            r#"
if [ -f {c} ]; then
echo '{{"type":"result","is_error":false,"structured_output":{{"overview":"o","modules":[{{"id":"m","name":"M","summary":"s","paths":["src"]}}],"tour":[]}}}}'
else touch {c}; echo '{{"type":"result","is_error":false,"structured_output":{{"bad":1}}}}'; fi"#,
            c = count.display()
        );
        let cfg = fake_claude(dir.path(), &script);
        let a = analyze_repo(&cfg, dir.path(), &index(), ModelChoice::Default, Default::default(), |_| {})
            .unwrap();
        assert_eq!(a.overview, "o");
    }

    #[test]
    fn gives_up_after_second_invalid_output_without_retrying_run_errors() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = fake_claude(
            dir.path(),
            r#"echo x >> runs_marker_$$; echo '{"type":"result","is_error":false,"structured_output":{"bad":1}}'"#,
        );
        let r = analyze_repo(&cfg, dir.path(), &index(), ModelChoice::Default, Default::default(), |_| {});
        assert!(matches!(r, Err(RunError::Failed(m)) if m.contains("検証")));
        let cfg = fake_claude(dir.path(), "exit 2");
        let r = analyze_repo(&cfg, dir.path(), &index(), ModelChoice::Default, Default::default(), |_| {});
        assert!(matches!(r, Err(RunError::Failed(m)) if !m.contains("検証")));
    }

    #[test]
    fn passes_selected_model_to_claude() {
        let dir = tempfile::tempdir().unwrap();
        let seen = dir.path().join("args");
        let script = format!(
            r#"echo "$@" > {s}
echo '{{"type":"result","is_error":false,"structured_output":{{"explanation":"e","key_files":[]}}}}'"#,
            s = seen.display()
        );
        let cfg = fake_claude(dir.path(), &script);
        let m = Module { id: "m".into(), name: "M".into(), summary: "s".into(), paths: vec!["src".into()] };
        let d = deep_dive(&cfg, dir.path(), &index(), &m, ModelChoice::Fable, Default::default(), |_| {}).unwrap();
        assert_eq!(d.model, ModelChoice::Fable);
        assert!(std::fs::read_to_string(seen).unwrap().contains("--model fable"));
    }
}
