use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const TASK_HOYOVERSE_DAILY_CHECKIN: &str = "hoyoverse_daily_checkin";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Service {
    Hoyoverse,
    Ncore,
}

impl Service {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hoyoverse => "hoyoverse",
            Self::Ncore => "ncore",
        }
    }
}

impl TryFrom<&str> for Service {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "hoyoverse" => Ok(Self::Hoyoverse),
            "ncore" => Ok(Self::Ncore),
            other => Err(format!("unsupported service '{}'", other)),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct UserPublic {
    pub id: i64,
    pub username: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HoyoverseConfig {
    pub ltoken_v2: String,
    pub ltuid_v2: String,
    pub ltmid_v2: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NcoreConfig {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub password: String,
    pub invite_code: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct AuthResponse {
    pub token: String,
    pub user: UserPublic,
}

#[derive(Debug, Deserialize)]
pub struct UpsertAccountRequest {
    pub name: String,
    pub service: Service,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub config: Value,
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Serialize)]
pub struct AccountResponse {
    pub id: i64,
    pub name: String,
    pub service: String,
    pub enabled: bool,
    pub config: Value,
    pub created_at: String,
    pub updated_at: String,
    pub tasks: Vec<TaskResponse>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TaskResponse {
    pub id: i64,
    pub account_id: i64,
    pub account_name: String,
    pub task_type: String,
    pub enabled: bool,
    pub next_run_at: String,
    pub last_run_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct InviteResponse {
    pub id: i64,
    pub code: String,
    pub created_at: String,
    pub redeemed_at: Option<String>,
    pub redeemed_by_username: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct TaskLogResponse {
    pub id: i64,
    pub account_id: Option<i64>,
    pub account_name: Option<String>,
    pub task_type: String,
    pub status: String,
    pub started_at: String,
    pub finished_at: String,
    pub duration_ms: i64,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct PaginatedLogsResponse {
    pub page: u32,
    pub page_size: u32,
    pub total: i64,
    pub items: Vec<TaskLogResponse>,
}

pub fn validate_account_config(service: Service, config: Value) -> Result<Value, String> {
    match service {
        Service::Hoyoverse => {
            let parsed: HoyoverseConfig =
                serde_json::from_value(config).map_err(|err| err.to_string())?;
            require_non_empty("ltoken_v2", &parsed.ltoken_v2)?;
            require_non_empty("ltuid_v2", &parsed.ltuid_v2)?;
            require_non_empty("ltmid_v2", &parsed.ltmid_v2)?;
            serde_json::to_value(parsed).map_err(|err| err.to_string())
        }
        Service::Ncore => {
            let parsed: NcoreConfig =
                serde_json::from_value(config).map_err(|err| err.to_string())?;
            require_non_empty("username", &parsed.username)?;
            require_non_empty("password", &parsed.password)?;
            serde_json::to_value(parsed).map_err(|err| err.to_string())
        }
    }
}

fn require_non_empty(field: &str, value: &str) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{} is required", field));
    }

    Ok(())
}
