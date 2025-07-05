use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppState {
    Starting,
    Running,
    Stopping,
    Stopped,
    Failed,
    Crashed,
}

impl std::fmt::Display for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppState::Starting => write!(f, "starting"),
            AppState::Running => write!(f, "running"),
            AppState::Stopping => write!(f, "stopping"),
            AppState::Stopped => write!(f, "stopped"),
            AppState::Failed => write!(f, "failed"),
            AppState::Crashed => write!(f, "crashed"),
        }
    }
}

pub fn parse_app_state(state_str: &str) -> AppState {
    let state = match state_str {
        "starting" => AppState::Starting,
        "running" => AppState::Running,
        "stopping" => AppState::Stopping,
        "stopped" => AppState::Stopped,
        "failed" => AppState::Failed,
        "crashed" => AppState::Crashed,
        _ => AppState::Stopped,
    };

    state
}
