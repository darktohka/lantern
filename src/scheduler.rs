use chrono::{DateTime, Datelike, Duration, NaiveTime, Utc};
use rand::RngExt;
use sqlx::{FromRow, SqlitePool};
use tokio::time::{Duration as TokioDuration, sleep};
use tracing::{error, info, warn};

use crate::{
    hoyoverse, ncore, notifier,
    models::{
        HoyoverseConfig, NcoreConfig, Service, TaskLogResponse, TaskOutcome,
        TASK_HOYOVERSE_DAILY_CHECKIN, TASK_NCORE_DAILY_CHECKIN,
    },
    state::AppState,
    timeutil::{now, to_sql_timestamp},
    torrent::TorrentKeepalive,
};

#[derive(Debug, FromRow)]
struct DueTaskRow {
    task_id: i64,
    account_id: i64,
    user_id: i64,
    account_name: String,
    service: String,
    task_type: String,
    config_json: String,
}

pub async fn run(state: AppState) {
    loop {
        if let Err(err) = run_due_tasks(&state).await {
            error!(error = %err, "scheduler tick failed");
        }

        let sleep_for = next_sleep_duration(&state.db).await.unwrap_or_else(|err| {
            warn!(error = %err, "failed to compute scheduler sleep duration");
            TokioDuration::from_secs(60)
        });

        sleep(sleep_for).await;
    }
}

pub async fn reconcile_account_tasks(
    pool: &SqlitePool,
    account_id: i64,
    service: Service,
) -> anyhow::Result<()> {
    match service {
        Service::Hoyoverse => {
            let timestamp = now();
            sqlx::query(
                r#"
                INSERT OR IGNORE INTO account_tasks
                    (account_id, task_type, enabled, next_run_at, created_at, updated_at)
                VALUES (?1, ?2, 1, ?3, ?4, ?4)
                "#,
            )
            .bind(account_id)
            .bind(TASK_HOYOVERSE_DAILY_CHECKIN)
            .bind(to_sql_timestamp(next_hoyoverse_daily_run_from(timestamp)))
            .bind(to_sql_timestamp(timestamp))
            .execute(pool)
            .await?;
        }
        Service::Ncore => {
            let timestamp = now();
            sqlx::query(
                r#"
                INSERT OR IGNORE INTO account_tasks
                    (account_id, task_type, enabled, next_run_at, created_at, updated_at)
                VALUES (?1, ?2, 1, ?3, ?4, ?4)
                "#,
            )
            .bind(account_id)
            .bind(TASK_NCORE_DAILY_CHECKIN)
            .bind(to_sql_timestamp(next_ncore_daily_run_from(timestamp)))
            .bind(to_sql_timestamp(timestamp))
            .execute(pool)
            .await?;
        }
    }

    Ok(())
}

pub async fn run_task_now(
    pool: &SqlitePool,
    http: &reqwest::Client,
    torrents: &TorrentKeepalive,
    task_id: i64,
) -> anyhow::Result<TaskLogResponse> {
    let task = sqlx::query_as::<_, DueTaskRow>(
        r#"
        SELECT
            t.id AS task_id,
            t.account_id AS account_id,
            a.user_id AS user_id,
            a.name AS account_name,
            a.service AS service,
            t.task_type AS task_type,
            a.config_json AS config_json
        FROM account_tasks t
        JOIN accounts a ON a.id = t.account_id
        WHERE t.id = ?1
        "#,
    )
    .bind(task_id)
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| anyhow::anyhow!("task not found"))?;

    let started_at = now();
    let log = execute_and_log(pool, http, torrents, &task, started_at).await?;
    info!(
        task_id = task.task_id,
        account_id = task.account_id,
        task_type = task.task_type,
        "manual task completed"
    );
    Ok(log)
}

async fn execute_and_log(
    pool: &SqlitePool,
    http: &reqwest::Client,
    torrents: &TorrentKeepalive,
    task: &DueTaskRow,
    started_at: chrono::DateTime<Utc>,
) -> Result<TaskLogResponse, sqlx::Error> {
    let outcome = run_task_by_type(http, pool, torrents, task).await;
    let finished_at = now();
    let duration_ms = (finished_at - started_at).num_milliseconds();
    let status = if outcome.success { "success" } else { "failed" };

    if !outcome.success {
        let title = format!("Check-in failed: {}", task.account_name);
        let message = format!(
            "[{}] {} - {}",
            task.task_type, task.account_name, outcome.message
        );
        notifier::send_ntfy_alerts(pool, http, task.user_id, &title, &message).await;
    }

    let result = sqlx::query(
        r#"
        INSERT INTO execution_logs
            (user_id, account_id, task_id, task_type, status, started_at, finished_at, duration_ms, message)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
    )
    .bind(task.user_id)
    .bind(task.account_id)
    .bind(task.task_id)
    .bind(&task.task_type)
    .bind(status)
    .bind(to_sql_timestamp(started_at))
    .bind(to_sql_timestamp(finished_at))
    .bind(duration_ms)
    .bind(&outcome.message)
    .execute(pool)
    .await?;

    Ok(TaskLogResponse {
        id: result.last_insert_rowid(),
        account_id: Some(task.account_id),
        account_name: Some(task.account_name.clone()),
        task_type: task.task_type.clone(),
        status: status.to_string(),
        started_at: to_sql_timestamp(started_at),
        finished_at: to_sql_timestamp(finished_at),
        duration_ms,
        message: outcome.message,
    })
}

pub fn next_hoyoverse_daily_run_from(after: DateTime<Utc>) -> DateTime<Utc> {
    let utc8_now = after + Duration::hours(8);
    let local_date = utc8_now.date_naive();
    let mut base_utc = local_date.and_time(NaiveTime::MIN).and_utc() - Duration::hours(8);

    if base_utc + Duration::seconds(60) <= after {
        base_utc = (local_date + Duration::days(1))
            .and_time(NaiveTime::MIN)
            .and_utc()
            - Duration::hours(8);
    }

    with_hoyoverse_random_delay(base_utc)
}

fn next_hoyoverse_daily_run_next_day(after: DateTime<Utc>) -> DateTime<Utc> {
    let utc8_now = after + Duration::hours(8);
    let local_date = utc8_now.date_naive() + Duration::days(1);
    let base_utc = local_date.and_time(NaiveTime::MIN).and_utc() - Duration::hours(8);
    with_hoyoverse_random_delay(base_utc)
}

fn with_hoyoverse_random_delay(base: DateTime<Utc>) -> DateTime<Utc> {
    let delay_seconds = rand::rng().random_range(10..=60);
    base + Duration::seconds(delay_seconds)
}

fn hungarian_offset_now() -> i64 {
    use chrono::Datelike;
    let now = Utc::now();
    let year = now.year();

    let dst_start = last_sunday_of_month(year, 3).and_hms_opt(1, 0, 0).unwrap().and_utc();
    let dst_end = last_sunday_of_month(year, 10).and_hms_opt(1, 0, 0).unwrap().and_utc();

    if now >= dst_start && now < dst_end { 7200 } else { 3600 }
}

pub fn last_sunday_of_month(year: i32, month: u32) -> chrono::NaiveDate {
    let mut day = if month == 2 {
        if chrono::NaiveDate::from_ymd_opt(year, month, 29).is_some() { 29 } else { 28 }
    } else if month == 4 || month == 6 || month == 9 || month == 11 {
        30
    } else {
        31
    };
    loop {
        if let Some(date) = chrono::NaiveDate::from_ymd_opt(year, month, day) {
            if date.weekday() == chrono::Weekday::Sun {
                return date;
            }
        }
        day -= 1;
    }
}

pub fn next_ncore_daily_run_from(after: DateTime<Utc>) -> DateTime<Utc> {
    let offset = hungarian_offset_now();
    let hu_now = after + Duration::seconds(offset);
    let local_date = hu_now.date_naive();
    let today_start_utc = local_date.and_time(chrono::NaiveTime::MIN).and_utc() - Duration::seconds(offset);
    let mut base_utc = today_start_utc + Duration::hours(10);

    if base_utc + Duration::seconds(60) <= after {
        let tomorrow = local_date + Duration::days(1);
        let tomorrow_start_utc = tomorrow.and_time(chrono::NaiveTime::MIN).and_utc() - Duration::seconds(offset);
        base_utc = tomorrow_start_utc + Duration::hours(10);
    }

    with_ncore_random_delay(base_utc)
}

fn next_ncore_daily_run_next_day(after: DateTime<Utc>) -> DateTime<Utc> {
    let offset = hungarian_offset_now();
    let hu_now = after + Duration::seconds(offset);
    let local_date = hu_now.date_naive() + Duration::days(1);
    let tomorrow_start_utc = local_date.and_time(chrono::NaiveTime::MIN).and_utc() - Duration::seconds(offset);
    let base_utc = tomorrow_start_utc + Duration::hours(10);
    with_ncore_random_delay(base_utc)
}

fn with_ncore_random_delay(base: DateTime<Utc>) -> DateTime<Utc> {
    let delay_seconds = rand::rng().random_range(0..=14400);
    base + Duration::seconds(delay_seconds)
}

async fn run_due_tasks(state: &AppState) -> anyhow::Result<()> {
    let due_at = to_sql_timestamp(now());
    let tasks = sqlx::query_as::<_, DueTaskRow>(
        r#"
        SELECT
            t.id AS task_id,
            t.account_id AS account_id,
            a.user_id AS user_id,
            a.name AS account_name,
            a.service AS service,
            t.task_type AS task_type,
            a.config_json AS config_json
        FROM account_tasks t
        JOIN accounts a ON a.id = t.account_id
        WHERE t.enabled = 1
          AND a.enabled = 1
          AND t.next_run_at <= ?1
        ORDER BY t.next_run_at ASC
        LIMIT 20
        "#,
    )
    .bind(due_at)
    .fetch_all(&state.db)
    .await?;

    for task in tasks {
        execute_task(state, task).await;
    }

    Ok(())
}

async fn execute_task(state: &AppState, task: DueTaskRow) {
    let started_at = now();
    let next_run_at = match task.task_type.as_str() {
        TASK_HOYOVERSE_DAILY_CHECKIN => next_hoyoverse_daily_run_next_day(started_at),
        TASK_NCORE_DAILY_CHECKIN => next_ncore_daily_run_next_day(started_at),
        other => {
            warn!(
                task_type = other,
                task_id = task.task_id,
                "unknown task type"
            );
            return;
        }
    };

    if let Err(err) = sqlx::query(
        r#"
        UPDATE account_tasks
        SET last_run_at = ?1,
            next_run_at = ?2,
            updated_at = ?1
        WHERE id = ?3
        "#,
    )
    .bind(to_sql_timestamp(started_at))
    .bind(to_sql_timestamp(next_run_at))
    .bind(task.task_id)
    .execute(&state.db)
    .await
    {
        error!(error = %err, task_id = task.task_id, "failed to reserve task");
        return;
    }

    info!(
        task_id = task.task_id,
        account_id = task.account_id,
        task_type = task.task_type,
        "running scheduled task"
    );

    if let Err(err) = execute_and_log(&state.db, &state.http, &state.torrents, &task, started_at).await {
        error!(error = %err, task_id = task.task_id, "scheduled task log failed");
    }
}

async fn run_task_by_type(
    http: &reqwest::Client,
    pool: &SqlitePool,
    torrents: &TorrentKeepalive,
    task: &DueTaskRow,
) -> TaskOutcome {
    match task.task_type.as_str() {
        TASK_HOYOVERSE_DAILY_CHECKIN => {
            if task.service != Service::Hoyoverse.as_str() {
                return TaskOutcome {
                    success: false,
                    message: format!(
                        "task {} cannot run for service {}",
                        task.task_type, task.service
                    ),
                };
            }

            match serde_json::from_str::<HoyoverseConfig>(&task.config_json) {
                Ok(config) => {
                    hoyoverse::run_daily_checkin(http, &task.account_name, &config).await
                }
                Err(err) => TaskOutcome {
                    success: false,
                    message: format!("invalid Hoyoverse account configuration: {}", err),
                },
            }
        }
        TASK_NCORE_DAILY_CHECKIN => {
            if task.service != Service::Ncore.as_str() {
                return TaskOutcome {
                    success: false,
                    message: format!(
                        "task {} cannot run for service {}",
                        task.task_type, task.service
                    ),
                };
            }

            match serde_json::from_str::<NcoreConfig>(&task.config_json) {
                Ok(config) => {
                    ncore::run_daily_checkin(pool, http, torrents, &task.account_name, task.account_id, task.user_id, &config)
                        .await
                }
                Err(err) => TaskOutcome {
                    success: false,
                    message: format!("invalid nCore account configuration: {}", err),
                },
            }
        }
        other => TaskOutcome {
            success: false,
            message: format!("unknown task type {}", other),
        },
    }
}

async fn next_sleep_duration(pool: &SqlitePool) -> anyhow::Result<TokioDuration> {
    let (next_run_at,): (Option<String>,) = sqlx::query_as(
        r#"
        SELECT MIN(t.next_run_at)
        FROM account_tasks t
        JOIN accounts a ON a.id = t.account_id
        WHERE t.enabled = 1
          AND a.enabled = 1
        "#,
    )
    .fetch_one(pool)
    .await?;

    let Some(next_run_at) = next_run_at else {
        return Ok(TokioDuration::from_secs(60));
    };

    let parsed = DateTime::parse_from_rfc3339(&next_run_at)?.with_timezone(&Utc);
    let seconds = (parsed - now()).num_seconds().clamp(1, 3600) as u64;
    Ok(TokioDuration::from_secs(seconds))
}
