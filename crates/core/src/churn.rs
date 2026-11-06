//! Git の情報。失敗しても解析全体は止めない。
use std::collections::HashMap;
use std::path::Path;
use std::process::Command;

fn git(root: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).current_dir(root).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// 直近 1 年のコミット数をファイルごとに数える。
pub fn churn_counts(root: &Path) -> HashMap<String, u32> {
    let mut counts = HashMap::new();
    let log = git(root, &["log", "--since=1.year", "--name-only", "--pretty=format:"]);
    for line in log.unwrap_or_default().lines().filter(|l| !l.is_empty()) {
        *counts.entry(line.to_string()).or_insert(0) += 1;
    }
    counts
}

pub fn head_sha(root: &Path) -> Option<String> {
    git(root, &["rev-parse", "HEAD"]).map(|s| s.trim().to_string())
}

pub fn is_git_repo(root: &Path) -> bool {
    git(root, &["rev-parse", "--is-inside-work-tree"]).is_some_and(|s| s.trim() == "true")
}
