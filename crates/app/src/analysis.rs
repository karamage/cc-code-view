//! AI 解析の状態遷移。実行そのものには依存しない。
use ccv_ai::runner::RunError;
use ccv_core::model::ModelChoice;
use std::time::Instant;

#[derive(Clone, Debug)]
pub enum AnalysisState {
    /// claude を使えない。理由を表示する
    Unavailable(String),
    Idle,
    Running { model: ModelChoice, activity: String, started: Instant },
    Failed { message: String, can_retry: bool },
}

impl PartialEq for AnalysisState {
    fn eq(&self, other: &Self) -> bool {
        use AnalysisState::*;
        match (self, other) {
            (Unavailable(a), Unavailable(b)) => a == b,
            (Idle, Idle) => true,
            (Running { model: a, activity: b, .. }, Running { model: c, activity: d, .. }) => {
                a == c && b == d
            }
            (Failed { message: a, can_retry: b }, Failed { message: c, can_retry: d }) => {
                a == c && b == d
            }
            _ => false,
        }
    }
}

pub enum Ev {
    Start(ModelChoice),
    Activity(String),
    Finished,
    Failed(RunError),
}

pub fn transition(state: AnalysisState, event: Ev) -> AnalysisState {
    use AnalysisState::*;
    match (state, event) {
        (s @ Running { .. }, Ev::Start(_)) => s,
        (Unavailable(m), Ev::Start(_)) => Unavailable(m),
        (_, Ev::Start(model)) => {
            Running { model, activity: String::new(), started: Instant::now() }
        }
        (Running { model, started, .. }, Ev::Activity(activity)) => {
            Running { model, activity, started }
        }
        (Running { .. }, Ev::Finished) => Idle,
        (Running { .. }, Ev::Failed(e)) => match e {
            RunError::NotInstalled | RunError::Unsupported => Unavailable(e.to_string()),
            RunError::Cancelled => Idle,
            other => Failed { message: other.to_string(), can_retry: true },
        },
        (s, _) => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_lifecycle() {
        let idle = AnalysisState::Idle;
        let running = transition(idle.clone(), Ev::Start(ModelChoice::Opus));
        assert!(matches!(&running, AnalysisState::Running { model: ModelChoice::Opus, .. }));
        let running = transition(running, Ev::Activity("src/a.rs".into()));
        assert!(matches!(&running, AnalysisState::Running { activity, .. } if activity == "src/a.rs"));
        assert_eq!(transition(running, Ev::Finished), idle);
    }

    #[test]
    fn failures_are_retryable_except_when_unavailable() {
        let r = || AnalysisState::Running {
            model: ModelChoice::Default,
            activity: String::new(),
            started: Instant::now(),
        };
        assert!(matches!(
            transition(r(), Ev::Failed(RunError::TimedOut)),
            AnalysisState::Failed { can_retry: true, .. }
        ));
        assert!(matches!(
            transition(r(), Ev::Failed(RunError::NotInstalled)),
            AnalysisState::Unavailable(_)
        ));
        assert_eq!(transition(r(), Ev::Failed(RunError::Cancelled)), AnalysisState::Idle);
    }

    #[test]
    fn start_is_ignored_while_running_or_unavailable() {
        let r = AnalysisState::Running {
            model: ModelChoice::Haiku,
            activity: String::new(),
            started: Instant::now(),
        };
        assert!(matches!(
            transition(r, Ev::Start(ModelChoice::Opus)),
            AnalysisState::Running { model: ModelChoice::Haiku, .. }
        ));
        let u = AnalysisState::Unavailable("x".into());
        assert_eq!(transition(u.clone(), Ev::Start(ModelChoice::Opus)), u);
    }
}
