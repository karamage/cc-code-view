//! claude の存在確認と PATH の復元。
use crate::runner::{ClaudeConfig, RunError};
use std::process::{Command, Stdio};

const MARKER: &str = "__CCV_PATH__";

/// Finder から起動したアプリには PATH が不足するので、ログインシェルに尋ねる。
pub fn login_shell_path() -> Option<String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
    let out = Command::new(shell)
        .args(["-ilc", &format!("printf '{MARKER}%s' \"$PATH\"")])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    parse_path(&String::from_utf8_lossy(&out.stdout))
}

fn parse_path(output: &str) -> Option<String> {
    let (_, path) = output.rsplit_once(MARKER)?;
    let path = path.trim();
    (!path.is_empty()).then(|| path.to_string())
}

/// PATH がまだ分かっていなければ、ログインシェルから補う。
/// 起動直後の確認が終わる前に実行しても、誤って「見つからない」とならないように、
/// 実行する側のスレッドで呼ぶ。
pub fn with_resolved_path(cfg: ClaudeConfig) -> ClaudeConfig {
    if cfg.path_env.is_some() {
        return cfg;
    }
    ClaudeConfig { path_env: login_shell_path(), ..cfg }
}

/// 使える claude があるか確認する。隔離に必要な `--restricted` が無い版は使わない。
pub fn preflight(cfg: &ClaudeConfig) -> Result<(), RunError> {
    let mut command = Command::new(&cfg.bin);
    command.arg("--help").stdin(Stdio::null()).stderr(Stdio::null());
    if let Some(path) = &cfg.path_env {
        command.env("PATH", path);
    }
    let out = command.output().map_err(|_| RunError::NotInstalled)?;
    if String::from_utf8_lossy(&out.stdout).contains("--restricted") {
        Ok(())
    } else {
        Err(RunError::Unsupported)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::testing::fake_claude;

    #[test]
    fn preflight_rejects_old_cli_without_restricted() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(preflight(&fake_claude(dir.path(), "echo usage")), Err(RunError::Unsupported));
        assert_eq!(preflight(&fake_claude(dir.path(), "echo --restricted")), Ok(()));
        let cfg = ClaudeConfig { bin: "/nonexistent/claude".into(), path_env: None };
        assert_eq!(preflight(&cfg), Err(RunError::NotInstalled));
    }

    #[test]
    fn known_path_is_kept_without_asking_the_shell() {
        let cfg = ClaudeConfig { bin: "claude".into(), path_env: Some("/a".into()) };
        assert_eq!(with_resolved_path(cfg).path_env.as_deref(), Some("/a"));
    }

    #[test]
    fn parses_path_after_shell_startup_noise() {
        assert_eq!(parse_path("welcome!\n__CCV_PATH__/a:/b\n"), Some("/a:/b".into()));
        assert_eq!(parse_path("nothing"), None);
    }
}
