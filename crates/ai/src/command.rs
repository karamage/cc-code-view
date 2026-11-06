//! claude の引数。ここがセキュリティの境界なので、書き込み権限を決して足さない。
use ccv_core::model::ModelChoice;
use std::path::Path;

/// 作業ディレクトリの外を遮断し、手元にしか無い秘密情報（`.gitignore` された `.env` など）を
/// Claude に読ませない。読まれたファイルは外部の API に送られるため。
/// `**/x` は作業ディレクトリ（空の一時ディレクトリ）からの相対になり `--add-dir` に効かないので、
/// 必ず絶対パスの形 `//**/x` で書く。
pub const LOCKDOWN_SETTINGS: &str = r#"{"permissions":{"blockReadsOutsideWorkingDirectories":true,"blockWritesOutsideWorkingDirectories":true,"deny":["Read(//**/.env)","Read(//**/.env.*)","Read(//**/*.pem)","Read(//**/*.key)","Read(//**/*.p12)","Read(//**/id_rsa*)","Read(//**/id_ed25519*)","Read(//**/.aws/**)","Read(//**/.ssh/**)","Read(//**/.npmrc)","Read(//**/.netrc)","Read(//**/credentials*)","Read(//**/secrets*)"]}}"#;

pub fn claude_args(prompt: &str, schema: &str, repo: &Path, model: ModelChoice) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        prompt,
        "--restricted",
        "--strict-mcp-config",
        "--tools",
        "Read,Grep,Glob",
        "--permission-mode",
        "dontAsk",
        "--allowedTools",
        "Read,Grep,Glob",
        "--settings",
        LOCKDOWN_SETTINGS,
        "--no-session-persistence",
        "--json-schema",
        schema,
        "--output-format",
        "stream-json",
        "--verbose",
    ]
    .map(String::from)
    .to_vec();
    args.extend(["--add-dir".into(), repo.display().to_string()]);
    if let Some(alias) = model.cli_alias() {
        args.extend(["--model".into(), alias.into()]);
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_are_read_only_and_model_is_optional() {
        let a = claude_args("p", "{}", Path::new("/r"), ModelChoice::Default);
        let joined = a.join(" ");
        assert!(joined.contains("--restricted --strict-mcp-config"));
        assert!(joined.contains("--tools Read,Grep,Glob"));
        assert!(joined.contains("--allowedTools Read,Grep,Glob"));
        assert!(joined.contains("--permission-mode dontAsk"));
        assert!(joined.contains("--no-session-persistence"));
        assert!(joined.contains("--add-dir /r"));
        let without_settings: Vec<_> = a.iter().filter(|x| *x != LOCKDOWN_SETTINGS).collect();
        for banned in ["Write", "Edit", "Bash"] {
            assert!(!without_settings.iter().any(|x| x.contains(banned)), "{banned}");
        }
        assert!(LOCKDOWN_SETTINGS.contains("Read(//**/.env)"));
        assert!(!a.contains(&"--model".to_string()));
        let a = claude_args("p", "{}", Path::new("/r"), ModelChoice::Sonnet);
        assert_eq!(a[a.len() - 2..], ["--model".to_string(), "sonnet".to_string()]);
    }
}
