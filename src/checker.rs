use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use reqwest::Url;
use scraper::{Html, Selector};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, SqlitePool};
use tracing::{error, info};

use crate::{
    models::{
        CheckType, CheckerResultResponse, ContentChangedConfig, StatusAlertMode, StatusCodeConfig,
    },
    notifier,
    state::AppState,
    timeutil::{now, to_sql_timestamp},
};

const CHECK_TIMEOUT_SECS: u64 = 30;
const MAX_CONTENT_BYTES: usize = 1_048_576;

#[derive(Debug, FromRow)]
struct DueCheckerRow {
    id: i64,
    user_id: i64,
    name: String,
    url: String,
    check_type: String,
    interval_seconds: i64,
    config_json: String,
    #[expect(dead_code, reason = "selected for the full checker row shape")]
    previous_status_code: Option<i64>,
    last_status_code: Option<i64>,
    content_hash: Option<String>,
    #[expect(dead_code, reason = "selected for the full checker row shape")]
    previous_content: Option<String>,
    current_content: Option<String>,
}

struct CheckOutcome {
    success: bool,
    triggered: bool,
    status_code: Option<i64>,
    status_changed: bool,
    content_changed: bool,
    message: String,
    hash: Option<String>,
    content: Option<String>,
}

impl CheckOutcome {
    fn failure(alert_on_error: bool, message: String) -> Self {
        Self {
            success: false,
            triggered: alert_on_error,
            status_code: None,
            status_changed: false,
            content_changed: false,
            message,
            hash: None,
            content: None,
        }
    }
}

enum ParsedConfig {
    Content(ContentChangedConfig),
    Status(StatusCodeConfig),
}

impl ParsedConfig {
    fn alert_on_error(&self) -> bool {
        match self {
            Self::Content(config) => config.alert_on_error,
            Self::Status(config) => config.alert_on_error,
        }
    }
}

pub async fn run_due(state: &AppState) -> anyhow::Result<()> {
    let due_at = to_sql_timestamp(now());
    let checkers = sqlx::query_as::<_, DueCheckerRow>(
        r#"
        SELECT
            id,
            user_id,
            name,
            url,
            check_type,
            interval_seconds,
            config_json,
            previous_status_code,
            last_status_code,
            content_hash,
            previous_content,
            current_content
        FROM checkers
        WHERE enabled = 1
          AND next_run_at <= ?1
        ORDER BY next_run_at ASC
        LIMIT 20
        "#,
    )
    .bind(due_at)
    .fetch_all(&state.db)
    .await?;

    for checker in checkers {
        execute_checker(state, &checker).await;
    }

    Ok(())
}

pub async fn run_now(state: &AppState, checker_id: i64) -> anyhow::Result<CheckerResultResponse> {
    let checker = load_checker_by_id(&state.db, checker_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("checker not found"))?;

    let started_at = now();
    let result = execute_and_log(state, &checker, started_at).await?;
    info!(checker_id, "manual checker run completed");
    Ok(result)
}

async fn load_checker_by_id(
    pool: &SqlitePool,
    checker_id: i64,
) -> anyhow::Result<Option<DueCheckerRow>> {
    let checker = sqlx::query_as::<_, DueCheckerRow>(
        r#"
        SELECT
            id,
            user_id,
            name,
            url,
            check_type,
            interval_seconds,
            config_json,
            previous_status_code,
            last_status_code,
            content_hash,
            previous_content,
            current_content
        FROM checkers
        WHERE id = ?1
        "#,
    )
    .bind(checker_id)
    .fetch_optional(pool)
    .await?;

    Ok(checker)
}

async fn execute_checker(state: &AppState, checker: &DueCheckerRow) {
    let started_at = now();
    let next_run_at = started_at + ChronoDuration::seconds(checker.interval_seconds);
    if let Err(err) = sqlx::query(
        r#"
        UPDATE checkers
        SET next_run_at = ?1,
            last_run_at = ?2,
            updated_at = ?2
        WHERE id = ?3
        "#,
    )
    .bind(to_sql_timestamp(next_run_at))
    .bind(to_sql_timestamp(started_at))
    .bind(checker.id)
    .execute(&state.db)
    .await
    {
        error!(error = %err, checker_id = checker.id, "failed to reserve checker");
        return;
    }

    info!(checker_id = checker.id, "running scheduled checker");

    if let Err(err) = execute_and_log(state, checker, started_at).await {
        error!(error = %err, checker_id = checker.id, "scheduled checker log failed");
    }
}

async fn execute_and_log(
    state: &AppState,
    checker: &DueCheckerRow,
    started_at: DateTime<Utc>,
) -> anyhow::Result<CheckerResultResponse> {
    let outcome = run_check(state, checker).await;
    let finished_at = now();
    let duration_ms = (finished_at - started_at).num_milliseconds();

    let (hash, previous_content, current_content, last_changed_at) = if outcome.success {
        match CheckType::try_from(checker.check_type.as_str()) {
            Ok(CheckType::ContentChanged) => {
                let first_run = checker.content_hash.is_none();
                if first_run || outcome.content_changed {
                    (
                        outcome.hash,
                        if first_run {
                            None
                        } else {
                            checker.current_content.clone()
                        },
                        outcome.content,
                        Some(to_sql_timestamp(started_at)),
                    )
                } else {
                    (None, None, None, None)
                }
            }
            _ => (None, None, None, None),
        }
    } else {
        (None, None, None, None)
    };

    let (previous_status_code, last_status_code) = match outcome.status_code {
        Some(status_code) => (checker.last_status_code, Some(status_code)),
        None => (None, None),
    };

    sqlx::query(
        r#"
        UPDATE checkers
        SET content_hash = COALESCE(?1, content_hash),
            previous_content = COALESCE(?2, previous_content),
            current_content = COALESCE(?3, current_content),
            last_changed_at = COALESCE(?4, last_changed_at),
            previous_status_code = COALESCE(?5, previous_status_code),
            last_status_code = COALESCE(?6, last_status_code),
            last_run_at = ?7,
            updated_at = ?7
        WHERE id = ?8
        "#,
    )
    .bind(hash)
    .bind(previous_content)
    .bind(current_content)
    .bind(last_changed_at)
    .bind(previous_status_code)
    .bind(last_status_code)
    .bind(to_sql_timestamp(started_at))
    .bind(checker.id)
    .execute(&state.db)
    .await?;

    let inserted = sqlx::query(
        r#"
        INSERT INTO checker_results
            (checker_id, user_id, status_code, status_changed, content_changed, triggered,
             message, started_at, finished_at, duration_ms)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        "#,
    )
    .bind(checker.id)
    .bind(checker.user_id)
    .bind(outcome.status_code)
    .bind(i64::from(outcome.status_changed))
    .bind(i64::from(outcome.content_changed))
    .bind(i64::from(outcome.triggered))
    .bind(&outcome.message)
    .bind(to_sql_timestamp(started_at))
    .bind(to_sql_timestamp(finished_at))
    .bind(duration_ms)
    .execute(&state.db)
    .await?;

    let result = CheckerResultResponse {
        id: inserted.last_insert_rowid(),
        checker_id: checker.id,
        status_code: outcome.status_code,
        status_changed: outcome.status_changed,
        content_changed: outcome.content_changed,
        triggered: outcome.triggered,
        message: outcome.message.clone(),
        started_at: to_sql_timestamp(started_at),
        finished_at: to_sql_timestamp(finished_at),
        duration_ms,
    };

    if outcome.triggered {
        notifier::send_ntfy_alerts(
            &state.db,
            &state.http,
            checker.user_id,
            &format!("Checker alert: {}", checker.name),
            &format!(
                "[{}] {} ({}) - {}",
                checker.check_type, checker.name, checker.url, outcome.message
            ),
        )
        .await;
    }

    Ok(result)
}

async fn run_check(state: &AppState, checker: &DueCheckerRow) -> CheckOutcome {
    let check_type = match CheckType::try_from(checker.check_type.as_str()) {
        Ok(check_type) => check_type,
        Err(err) => return CheckOutcome::failure(true, err),
    };

    let config = match parse_config(check_type, &checker.config_json) {
        Ok(config) => config,
        Err(err) => return CheckOutcome::failure(true, err),
    };
    let alert_on_error = config.alert_on_error();

    if let Err(err) = ensure_public_url(&checker.url).await {
        return CheckOutcome::failure(alert_on_error, err);
    }

    let mut response = match state
        .http
        .get(&checker.url)
        .timeout(Duration::from_secs(CHECK_TIMEOUT_SECS))
        .header("User-Agent", "LanternChecker/0.1")
        .send()
        .await
    {
        Ok(response) => response,
        Err(err) => {
            return CheckOutcome::failure(
                alert_on_error,
                format!("request failed: {}", err),
            );
        }
    };

    let status = response.status().as_u16();
    let status_code = Some(i64::from(status));
    let status_changed = checker
        .last_status_code
        .map_or(false, |previous| previous != i64::from(status));

    match config {
        ParsedConfig::Status(config) => {
            let triggered = status_changed
                && match config.target_status {
                    None => true,
                    Some(target) => match config.mode {
                        StatusAlertMode::Match => status == target,
                        StatusAlertMode::Mismatch => status != target,
                        StatusAlertMode::Any => true,
                    },
                };

            let message = if checker.last_status_code.is_none() {
                format!("initial status {}", status)
            } else if status_changed {
                format!(
                    "status changed {} -> {}",
                    checker.last_status_code.unwrap_or_default(),
                    status
                )
            } else {
                format!("status unchanged ({})", status)
            };

            CheckOutcome {
                success: true,
                triggered,
                status_code,
                status_changed,
                content_changed: false,
                message,
                hash: None,
                content: None,
            }
        }
        ParsedConfig::Content(config) => {
            let body = match read_body_capped(&mut response).await {
                Ok(body) => body,
                Err(err) => {
                    return CheckOutcome::failure(
                        alert_on_error,
                        format!("failed to read response body: {}", err),
                    );
                }
            };

            let extracted = extract_content(&body, config.selector.as_deref());
            let normalized = if config.ignore_whitespace {
                normalize_whitespace(&extracted)
            } else {
                extracted
            };
            let hash = hash_content(&normalized);
            let first_run = checker.content_hash.is_none();
            let content_changed = checker
                .content_hash
                .as_deref()
                .map_or(false, |previous| previous != hash);

            let message = if first_run {
                "initial snapshot".to_string()
            } else if content_changed {
                "content changed".to_string()
            } else {
                "content unchanged".to_string()
            };

            CheckOutcome {
                success: true,
                triggered: content_changed,
                status_code,
                status_changed,
                content_changed,
                message,
                hash: Some(hash),
                content: Some(normalized),
            }
        }
    }
}

fn parse_config(check_type: CheckType, config_json: &str) -> Result<ParsedConfig, String> {
    match check_type {
        CheckType::ContentChanged => serde_json::from_str::<ContentChangedConfig>(config_json)
            .map(ParsedConfig::Content)
            .map_err(|err| format!("invalid checker configuration: {}", err)),
        CheckType::StatusCodeChanged => serde_json::from_str::<StatusCodeConfig>(config_json)
            .map(ParsedConfig::Status)
            .map_err(|err| format!("invalid checker configuration: {}", err)),
    }
}

async fn read_body_capped(response: &mut reqwest::Response) -> Result<String, reqwest::Error> {
    let mut buffer: Vec<u8> = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        let remaining = MAX_CONTENT_BYTES.saturating_sub(buffer.len());
        if remaining == 0 {
            break;
        }
        if chunk.len() > remaining {
            buffer.extend_from_slice(&chunk[..remaining]);
            break;
        }
        buffer.extend_from_slice(&chunk);
    }

    Ok(String::from_utf8_lossy(&buffer).into_owned())
}

fn extract_content(body: &str, selector: Option<&str>) -> String {
    let Some(selector) = selector else {
        return body.to_string();
    };
    let Ok(selector) = Selector::parse(selector) else {
        return body.to_string();
    };

    let document = Html::parse_document(body);
    document
        .select(&selector)
        .map(|element| element.text().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join(" ")
}

fn normalize_whitespace(content: &str) -> String {
    content.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn hash_content(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    STANDARD.encode(hasher.finalize())
}

async fn ensure_public_url(url: &str) -> Result<(), String> {
    if private_hosts_allowed() {
        return Ok(());
    }

    let parsed = Url::parse(url).map_err(|err| format!("invalid URL: {}", err))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| "url must include a host".to_string())?;
    let port = parsed
        .port_or_known_default()
        .ok_or_else(|| "url must include a port".to_string())?;

    let addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|err| format!("failed to resolve host: {}", err))?;

    let mut resolved = false;
    for address in addresses {
        resolved = true;
        if is_blocked_ip(address.ip()) {
            return Err(format!(
                "refusing to request a private address: {}",
                address.ip()
            ));
        }
    }

    if !resolved {
        return Err("host did not resolve to any address".to_string());
    }

    Ok(())
}

fn private_hosts_allowed() -> bool {
    matches!(
        std::env::var("LANTERN_CHECKER_ALLOW_PRIVATE").as_deref(),
        Ok("1") | Ok("true")
    )
}

fn is_blocked_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(v4) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
        }
        std::net::IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_unique_local()
                || v6.is_unicast_link_local()
        }
    }
}
