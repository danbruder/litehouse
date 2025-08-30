use serde::{Deserialize, Serialize};
use sqlx::{Decode, Encode, Sqlite, Type};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppState {
    Created,
    Starting,
    Running,
    Stopping,
    Stopped,
    Failed,
    Crashed,
}

impl From<String> for AppState {
    fn from(state: String) -> Self {
        parse_app_state(&state)
    }
}

impl Type<Sqlite> for AppState {
    fn type_info() -> <Sqlite as sqlx::Database>::TypeInfo {
        <String as Type<Sqlite>>::type_info()
    }
}

impl<'r> Decode<'r, Sqlite> for AppState {
    fn decode(
        value: <Sqlite as sqlx::database::HasValueRef<'r>>::ValueRef,
    ) -> Result<Self, sqlx::error::BoxDynError> {
        let naive = <String as Decode<Sqlite>>::decode(value)?;
        let parsed = parse_app_state(&naive);
        Ok(parsed)
    }
}

impl<'q> Encode<'q, Sqlite> for AppState {
    fn encode_by_ref(
        &self,
        buf: &mut <Sqlite as sqlx::database::HasArguments<'q>>::ArgumentBuffer,
    ) -> sqlx::encode::IsNull {
        let naive = app_state_to_string(self);
        <String as Encode<Sqlite>>::encode_by_ref(&naive, buf)
    }
}

impl std::fmt::Display for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppState::Created => write!(f, "created"),
            AppState::Starting => write!(f, "starting"),
            AppState::Running => write!(f, "running"),
            AppState::Stopping => write!(f, "stopping"),
            AppState::Stopped => write!(f, "stopped"),
            AppState::Failed => write!(f, "failed"),
            AppState::Crashed => write!(f, "crashed"),
        }
    }
}

pub fn app_state_to_string(state: &AppState) -> String {
    match state {
        AppState::Created => "created".into(),
        AppState::Starting => "starting".into(),
        AppState::Running => "running".into(),
        AppState::Stopping => "stopping".into(),
        AppState::Stopped => "stopped".into(),
        AppState::Failed => "failed".into(),
        AppState::Crashed => "crashed".into(),
    }
}

pub fn parse_app_state(state_str: &str) -> AppState {
    match state_str {
        "created" => AppState::Created,
        "starting" => AppState::Starting,
        "running" => AppState::Running,
        "stopping" => AppState::Stopping,
        "stopped" => AppState::Stopped,
        "failed" => AppState::Failed,
        "crashed" => AppState::Crashed,
        _ => AppState::Stopped,
    }
}
