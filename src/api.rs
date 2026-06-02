use std::{collections::HashMap, path::PathBuf};

use axum::{
    Json, Router,
    extract::{FromRef, FromRequestParts, Path, Query, State},
    http::{HeaderMap, StatusCode, header::AUTHORIZATION, request::Parts},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::FromRow;
use tokio::net::TcpListener;
use tower_http::{
    cors::CorsLayer,
    services::{ServeDir, ServeFile},
    trace::TraceLayer,
};
use tracing::{error, info};

use crate::{
    auth,
    models::{
        AccountResponse, AuthResponse, CreateNtfyAlertRequest, InviteResponse, LoginRequest,
        NtfyAlertResponse, PaginatedLogsResponse, RegisterRequest, Service, TaskLogResponse,
        TaskResponse, UpsertAccountRequest, UserPublic, validate_account_config,
    },
    scheduler,
    state::AppState,
    timeutil::{now, to_sql_timestamp},
    torrent::{TorrentResponse, self},
};

pub async fn serve(db: sqlx::SqlitePool, bind: String, static_dir: String, torrent_dir: String) -> anyhow::Result<()> {
    let http = reqwest::Client::new();
    let torrent_dir = std::path::PathBuf::from(torrent_dir);
    tokio::fs::create_dir_all(&torrent_dir).await?;
    let state = AppState::new(db, http, torrent_dir);

    let api = Router::new()
        .route("/health", get(health))
        .route("/auth/register", post(register))
        .route("/auth/login", post(login))
        .route("/auth/logout", post(logout))
        .route("/auth/me", get(me))
        .route("/accounts", get(list_accounts).post(create_account))
        .route("/accounts/{id}", put(update_account).delete(delete_account))
        .route("/invites", get(list_invites).post(create_invite))
        .route("/invites/{id}", delete(delete_invite))
        .route("/tasks", get(list_tasks))
        .route("/tasks/{id}/run", post(run_task))
        .route("/task-logs", get(list_task_logs))
        .route("/torrents", get(list_torrents))
        .route("/torrents/{id}", delete(delete_torrent))
        .route("/ntfy-alerts", get(list_ntfy_alerts).post(create_ntfy_alert))
        .route("/ntfy-alerts/{id}", delete(delete_ntfy_alert));

    let static_path = PathBuf::from(static_dir);
    let index_path = static_path.join("index.html");
    let static_service = ServeDir::new(static_path).fallback(ServeFile::new(index_path));

    let app = Router::new()
        .nest("/api", api)
        .fallback_service(static_service)
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone());

    let scheduler_state = state.clone();
    let scheduler_handle = tokio::spawn(async move {
        scheduler::run(scheduler_state).await;
    });

    let torrent_keepalive = state.torrents.clone();
    let torrent_handle = tokio::spawn(async move {
        torrent::run_keepalive(torrent_keepalive).await;
    });

    let listener = TcpListener::bind(&bind).await?;
    let local_addr = listener.local_addr()?;
    info!(%local_addr, "Lantern is listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    scheduler_handle.abort();
    torrent_handle.abort();
    Ok(())
}

#[derive(Debug, Clone)]
struct AuthUser {
    id: i64,
    username: String,
    token_hash: String,
}

#[derive(Debug, FromRow)]
struct AuthUserRow {
    id: i64,
    username: String,
}

impl<S> FromRequestParts<S> for AuthUser
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let token = bearer_token(&parts.headers).ok_or(ApiError::Unauthorized)?;
        let token_hash = auth::hash_token(token);
        let state = AppState::from_ref(state);
        let current_time = to_sql_timestamp(now());

        let user = sqlx::query_as::<_, AuthUserRow>(
            r#"
            SELECT u.id, u.username
            FROM sessions s
            JOIN users u ON u.id = s.user_id
            WHERE s.token_hash = ?1
              AND s.expires_at > ?2
            "#,
        )
        .bind(&token_hash)
        .bind(current_time)
        .fetch_optional(&state.db)
        .await?;

        let Some(user) = user else {
            return Err(ApiError::Unauthorized);
        };

        Ok(Self {
            id: user.id,
            username: user.username,
            token_hash,
        })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    Conflict(String),
    #[error("not found")]
    NotFound,
    #[error("unauthorized")]
    Unauthorized,
    #[error("internal server error")]
    Internal(#[from] anyhow::Error),
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        Self::Internal(err.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self {
            Self::BadRequest(_) => StatusCode::BAD_REQUEST,
            Self::Conflict(_) => StatusCode::CONFLICT,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::Unauthorized => StatusCode::UNAUTHORIZED,
            Self::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };

        if status == StatusCode::INTERNAL_SERVER_ERROR {
            error!(error = %self, "api request failed");
        }

        let message = self.to_string();
        (status, Json(json!({ "error": message }))).into_response()
    }
}

#[derive(Serialize)]
struct HealthResponse {
    ok: bool,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse { ok: true })
}

async fn register(
    State(state): State<AppState>,
    Json(payload): Json<RegisterRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    auth::validate_username(&payload.username)
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;
    auth::validate_password(&payload.password)
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;

    let mut tx = state.db.begin().await?;
    let invite = sqlx::query_as::<_, (i64,)>(
        r#"
        SELECT id
        FROM invite_codes
        WHERE code = ?1
          AND redeemed_at IS NULL
        "#,
    )
    .bind(payload.invite_code.trim())
    .fetch_optional(&mut *tx)
    .await?;

    let Some((invite_id,)) = invite else {
        return Err(ApiError::BadRequest(
            "invite code is invalid or redeemed".to_string(),
        ));
    };

    let created_at = to_sql_timestamp(now());
    let password_hash = auth::hash_password(&payload.password)
        .map_err(|err| ApiError::BadRequest(err.to_string()))?;

    let result = sqlx::query(
        r#"
        INSERT INTO users (username, password_hash, created_at)
        VALUES (?1, ?2, ?3)
        "#,
    )
    .bind(payload.username.trim())
    .bind(password_hash)
    .bind(&created_at)
    .execute(&mut *tx)
    .await
    .map_err(map_database_insert_error)?;

    let user = UserPublic {
        id: result.last_insert_rowid(),
        username: payload.username.trim().to_string(),
    };

    let redeemed_at = to_sql_timestamp(now());
    let update = sqlx::query(
        r#"
        UPDATE invite_codes
        SET redeemed_by_user_id = ?1,
            redeemed_at = ?2
        WHERE id = ?3
          AND redeemed_at IS NULL
        "#,
    )
    .bind(user.id)
    .bind(redeemed_at)
    .bind(invite_id)
    .execute(&mut *tx)
    .await?;

    if update.rows_affected() != 1 {
        return Err(ApiError::Conflict(
            "invite code was already redeemed".to_string(),
        ));
    }

    tx.commit().await?;
    let token = auth::create_session(&state.db, user.id).await?;

    Ok(Json(AuthResponse { token, user }))
}

async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    let user = auth::authenticate_user(&state.db, &payload.username, &payload.password)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    let token = auth::create_session(&state.db, user.id).await?;

    Ok(Json(AuthResponse { token, user }))
}

async fn logout(State(state): State<AppState>, user: AuthUser) -> Result<StatusCode, ApiError> {
    sqlx::query(
        r#"
        DELETE FROM sessions
        WHERE user_id = ?1
          AND token_hash = ?2
        "#,
    )
    .bind(user.id)
    .bind(user.token_hash)
    .execute(&state.db)
    .await?;

    Ok(StatusCode::NO_CONTENT)
}

async fn me(user: AuthUser) -> Json<UserPublic> {
    Json(UserPublic {
        id: user.id,
        username: user.username,
    })
}

async fn list_invites(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<InviteResponse>>, ApiError> {
    let invites = sqlx::query_as::<_, InviteRow>(
        r#"
        SELECT
            i.id,
            i.code,
            i.created_at,
            i.redeemed_at,
            u.username AS redeemed_by_username
        FROM invite_codes i
        LEFT JOIN users u ON u.id = i.redeemed_by_user_id
        WHERE i.created_by_user_id = ?1
        ORDER BY i.created_at DESC
        "#,
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(InviteRow::into_response)
    .collect();

    Ok(Json(invites))
}

async fn create_invite(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<InviteResponse>, ApiError> {
    let code = auth::create_invite_code(&state.db, user.id, true)
        .await
        .map_err(|err| {
            if err.to_string().contains("at most 5") {
                ApiError::Conflict(err.to_string())
            } else {
                ApiError::Internal(err)
            }
        })?;

    let invite = sqlx::query_as::<_, InviteRow>(
        r#"
        SELECT
            i.id,
            i.code,
            i.created_at,
            i.redeemed_at,
            u.username AS redeemed_by_username
        FROM invite_codes i
        LEFT JOIN users u ON u.id = i.redeemed_by_user_id
        WHERE i.code = ?1
        "#,
    )
    .bind(code)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(invite.into_response()))
}

async fn delete_invite(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let result = sqlx::query(
        r#"
        DELETE FROM invite_codes
        WHERE id = ?1
          AND created_by_user_id = ?2
          AND redeemed_at IS NULL
        "#,
    )
    .bind(id)
    .bind(user.id)
    .execute(&state.db)
    .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn list_accounts(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<AccountResponse>>, ApiError> {
    Ok(Json(load_accounts(&state.db, user.id).await?))
}

async fn create_account(
    State(state): State<AppState>,
    user: AuthUser,
    Json(payload): Json<UpsertAccountRequest>,
) -> Result<Json<AccountResponse>, ApiError> {
    let payload = normalize_account_payload(payload)?;
    let timestamp = to_sql_timestamp(now());
    let result = sqlx::query(
        r#"
        INSERT INTO accounts
            (user_id, name, service, enabled, config_json, created_at, updated_at)
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
        "#,
    )
    .bind(user.id)
    .bind(&payload.name)
    .bind(payload.service.as_str())
    .bind(i64::from(payload.enabled))
    .bind(payload.config.to_string())
    .bind(timestamp)
    .execute(&state.db)
    .await?;

    let account_id = result.last_insert_rowid();
    scheduler::reconcile_account_tasks(&state.db, account_id, payload.service).await?;

    Ok(Json(load_account(&state.db, user.id, account_id).await?))
}

async fn update_account(
    State(state): State<AppState>,
    user: AuthUser,
    Path(account_id): Path<i64>,
    Json(payload): Json<UpsertAccountRequest>,
) -> Result<Json<AccountResponse>, ApiError> {
    let payload = normalize_account_payload(payload)?;
    let timestamp = to_sql_timestamp(now());
    let result = sqlx::query(
        r#"
        UPDATE accounts
        SET name = ?1,
            service = ?2,
            enabled = ?3,
            config_json = ?4,
            updated_at = ?5
        WHERE id = ?6
          AND user_id = ?7
        "#,
    )
    .bind(&payload.name)
    .bind(payload.service.as_str())
    .bind(i64::from(payload.enabled))
    .bind(payload.config.to_string())
    .bind(timestamp)
    .bind(account_id)
    .bind(user.id)
    .execute(&state.db)
    .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }

    scheduler::reconcile_account_tasks(&state.db, account_id, payload.service).await?;

    Ok(Json(load_account(&state.db, user.id, account_id).await?))
}

async fn delete_account(
    State(state): State<AppState>,
    user: AuthUser,
    Path(account_id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let result = sqlx::query(
        r#"
        DELETE FROM accounts
        WHERE id = ?1
          AND user_id = ?2
        "#,
    )
    .bind(account_id)
    .bind(user.id)
    .execute(&state.db)
    .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn list_tasks(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<TaskResponse>>, ApiError> {
    let tasks = load_tasks(&state.db, user.id).await?;
    Ok(Json(tasks))
}

async fn run_task(
    State(state): State<AppState>,
    user: AuthUser,
    Path(task_id): Path<i64>,
) -> Result<Json<TaskLogResponse>, ApiError> {
    let (owner_id,): (i64,) = sqlx::query_as(
        r#"
        SELECT a.user_id
        FROM account_tasks t
        JOIN accounts a ON a.id = t.account_id
        WHERE t.id = ?1
        "#,
    )
    .bind(task_id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    if owner_id != user.id {
        return Err(ApiError::NotFound);
    }

    let log = scheduler::run_task_now(&state.db, &state.http, task_id)
        .await
        .map_err(ApiError::Internal)?;

    Ok(Json(log))
}

#[derive(Debug, Deserialize)]
struct LogsQuery {
    page: Option<u32>,
    page_size: Option<u32>,
    account_id: Option<i64>,
}

async fn list_task_logs(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<LogsQuery>,
) -> Result<Json<PaginatedLogsResponse>, ApiError> {
    let page = query.page.unwrap_or(1).max(1);
    let page_size = query.page_size.unwrap_or(20).clamp(1, 100);
    let offset = i64::from((page - 1) * page_size);
    let limit = i64::from(page_size);

    let (total,): (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*)
        FROM execution_logs l
        WHERE l.user_id = ?1
          AND (?2 IS NULL OR l.account_id = ?2)
        "#,
    )
    .bind(user.id)
    .bind(query.account_id)
    .fetch_one(&state.db)
    .await?;

    let items = sqlx::query_as::<_, TaskLogRow>(
        r#"
        SELECT
            l.id,
            l.account_id,
            a.name AS account_name,
            l.task_type,
            l.status,
            l.started_at,
            l.finished_at,
            l.duration_ms,
            l.message
        FROM execution_logs l
        LEFT JOIN accounts a ON a.id = l.account_id
        WHERE l.user_id = ?1
          AND (?2 IS NULL OR l.account_id = ?2)
        ORDER BY l.started_at DESC
        LIMIT ?3 OFFSET ?4
        "#,
    )
    .bind(user.id)
    .bind(query.account_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(TaskLogRow::into_response)
    .collect();

    Ok(Json(PaginatedLogsResponse {
        page,
        page_size,
        total,
        items,
    }))
}

struct NormalizedAccountPayload {
    name: String,
    service: Service,
    enabled: bool,
    config: Value,
}

fn normalize_account_payload(
    payload: UpsertAccountRequest,
) -> Result<NormalizedAccountPayload, ApiError> {
    let name = payload.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::BadRequest("account name is required".to_string()));
    }

    let config =
        validate_account_config(payload.service, payload.config).map_err(ApiError::BadRequest)?;

    Ok(NormalizedAccountPayload {
        name,
        service: payload.service,
        enabled: payload.enabled,
        config,
    })
}

async fn load_accounts(
    db: &sqlx::SqlitePool,
    user_id: i64,
) -> Result<Vec<AccountResponse>, ApiError> {
    let accounts = sqlx::query_as::<_, AccountRow>(
        r#"
        SELECT id, name, service, enabled, config_json, created_at, updated_at
        FROM accounts
        WHERE user_id = ?1
        ORDER BY created_at DESC
        "#,
    )
    .bind(user_id)
    .fetch_all(db)
    .await?;

    let tasks = load_tasks(db, user_id).await?;
    let mut tasks_by_account: HashMap<i64, Vec<TaskResponse>> = HashMap::new();
    for task in tasks {
        tasks_by_account
            .entry(task.account_id)
            .or_default()
            .push(task);
    }

    accounts
        .into_iter()
        .map(|account| {
            let account_id = account.id;
            account.into_response(tasks_by_account.remove(&account_id).unwrap_or_default())
        })
        .collect()
}

async fn load_account(
    db: &sqlx::SqlitePool,
    user_id: i64,
    account_id: i64,
) -> Result<AccountResponse, ApiError> {
    let account = sqlx::query_as::<_, AccountRow>(
        r#"
        SELECT id, name, service, enabled, config_json, created_at, updated_at
        FROM accounts
        WHERE user_id = ?1
          AND id = ?2
        "#,
    )
    .bind(user_id)
    .bind(account_id)
    .fetch_optional(db)
    .await?
    .ok_or(ApiError::NotFound)?;

    let tasks = load_tasks(db, user_id)
        .await?
        .into_iter()
        .filter(|task| task.account_id == account_id)
        .collect();

    Ok(account.into_response(tasks)?)
}

async fn load_tasks(db: &sqlx::SqlitePool, user_id: i64) -> Result<Vec<TaskResponse>, ApiError> {
    let tasks = sqlx::query_as::<_, TaskRow>(
        r#"
        SELECT
            t.id,
            t.account_id,
            a.name AS account_name,
            t.task_type,
            t.enabled,
            t.next_run_at,
            t.last_run_at
        FROM account_tasks t
        JOIN accounts a ON a.id = t.account_id
        WHERE a.user_id = ?1
        ORDER BY t.next_run_at ASC
        "#,
    )
    .bind(user_id)
    .fetch_all(db)
    .await?
    .into_iter()
    .map(TaskRow::into_response)
    .collect();

    Ok(tasks)
}

#[derive(Debug, Deserialize)]
struct TorrentsQuery {
    account_id: Option<i64>,
}

async fn list_torrents(
    State(state): State<AppState>,
    user: AuthUser,
    Query(query): Query<TorrentsQuery>,
) -> Result<Json<Vec<TorrentResponse>>, ApiError> {
    let account_ids: Vec<i64> = if let Some(account_id) = query.account_id {
        let owner: (i64,) = sqlx::query_as(
            "SELECT user_id FROM accounts WHERE id = ?1",
        )
        .bind(account_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or(ApiError::NotFound)?;
        if owner.0 != user.id {
            return Err(ApiError::NotFound);
        }
        vec![account_id]
    } else {
        sqlx::query_as::<_, (i64,)>(
            "SELECT id FROM accounts WHERE user_id = ?1 AND service = 'ncore'",
        )
        .bind(user.id)
        .fetch_all(&state.db)
        .await?
        .into_iter()
        .map(|r| r.0)
        .collect()
    };

    let mut torrents = Vec::new();
    for account_id in account_ids {
        let rows = sqlx::query_as::<_, crate::torrent::NcoreTorrentRow>(
            r#"
            SELECT * FROM ncore_torrents WHERE account_id = ?1 ORDER BY created_at DESC
            "#,
        )
        .bind(account_id)
        .fetch_all(&state.db)
        .await?;

        let account_name: String = sqlx::query_scalar(
            "SELECT name FROM accounts WHERE id = ?1",
        )
        .bind(account_id)
        .fetch_optional(&state.db)
        .await?
        .unwrap_or_default();

        let session_handle = {
            let guard = state.torrents.session.lock().await;
            guard.clone()
        };
        for row in rows {
            let (progress, download_rate, upload_rate, total_download, total_upload) =
                if let Some(ref info_hash) = row.info_hash {
                    if !info_hash.is_empty() {
                        if let Ok(hash) = irontide::prelude::Id20::from_hex(info_hash) {
                            if let Some(ref handle) = session_handle {
                                if let Ok(stats) = handle.torrent_stats(hash).await {
                                    (
                                        f64::from(stats.progress),
                                        stats.download_payload_rate,
                                        stats.upload_payload_rate,
                                        stats.all_time_download,
                                        stats.all_time_upload,
                                    )
                                } else {
                                    (0.0, 0, 0, 0, 0)
                                }
                            } else {
                                (0.0, 0, 0, 0, 0)
                            }
                        } else {
                            (0.0, 0, 0, 0, 0)
                        }
                    } else {
                        (0.0, 0, 0, 0, 0)
                    }
                } else {
                    (0.0, 0, 0, 0, 0)
                };

            torrents.push(TorrentResponse {
                id: row.id,
                account_id: row.account_id,
                account_name: account_name.clone(),
                ncore_id: row.ncore_id,
                info_hash: row.info_hash,
                name: row.name,
                status: row.status,
                hnr_timespent: row.hnr_timespent,
                hnr_seed: row.hnr_seed,
                progress,
                download_rate,
                upload_rate,
                total_download,
                total_upload,
                created_at: row.created_at,
                updated_at: row.updated_at,
            });
        }
    }

    Ok(Json(torrents))
}

async fn delete_torrent(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let row = sqlx::query_as::<_, (i64, i64)>(
        r#"
        SELECT t.id, t.account_id
        FROM ncore_torrents t
        JOIN accounts a ON a.id = t.account_id
        WHERE t.id = ?1 AND a.user_id = ?2
        "#,
    )
    .bind(id)
    .bind(user.id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    torrent::remove_torrent(&state.torrents, row.0, row.1)
        .await
        .map_err(|err| ApiError::Internal(err.into()))?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, FromRow)]
struct NtfyAlertRow {
    id: i64,
    name: String,
    topic: String,
    created_at: String,
}

impl NtfyAlertRow {
    fn into_response(self) -> NtfyAlertResponse {
        NtfyAlertResponse {
            id: self.id,
            name: self.name,
            topic: self.topic,
            created_at: self.created_at,
        }
    }
}

async fn list_ntfy_alerts(
    State(state): State<AppState>,
    user: AuthUser,
) -> Result<Json<Vec<NtfyAlertResponse>>, ApiError> {
    let alerts = sqlx::query_as::<_, NtfyAlertRow>(
        r#"
        SELECT id, name, topic, created_at
        FROM ntfy_alerts
        WHERE user_id = ?1
        ORDER BY created_at DESC
        "#,
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(NtfyAlertRow::into_response)
    .collect();

    Ok(Json(alerts))
}

async fn create_ntfy_alert(
    State(state): State<AppState>,
    user: AuthUser,
    Json(payload): Json<CreateNtfyAlertRequest>,
) -> Result<Json<NtfyAlertResponse>, ApiError> {
    let name = payload.name.trim().to_string();
    if name.is_empty() {
        return Err(ApiError::BadRequest("alert name is required".to_string()));
    }
    let topic = payload.topic.trim().to_string();
    if topic.is_empty() {
        return Err(ApiError::BadRequest("topic is required".to_string()));
    }

    let created_at = to_sql_timestamp(now());
    let result = sqlx::query(
        r#"
        INSERT INTO ntfy_alerts (user_id, name, topic, created_at)
        VALUES (?1, ?2, ?3, ?4)
        "#,
    )
    .bind(user.id)
    .bind(&name)
    .bind(&topic)
    .bind(&created_at)
    .execute(&state.db)
    .await?;

    Ok(Json(NtfyAlertResponse {
        id: result.last_insert_rowid(),
        name,
        topic,
        created_at,
    }))
}

async fn delete_ntfy_alert(
    State(state): State<AppState>,
    user: AuthUser,
    Path(id): Path<i64>,
) -> Result<StatusCode, ApiError> {
    let result = sqlx::query(
        r#"
        DELETE FROM ntfy_alerts
        WHERE id = ?1
          AND user_id = ?2
        "#,
    )
    .bind(id)
    .bind(user.id)
    .execute(&state.db)
    .await?;

    if result.rows_affected() == 0 {
        return Err(ApiError::NotFound);
    }

    Ok(StatusCode::NO_CONTENT)
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

fn map_database_insert_error(err: sqlx::Error) -> ApiError {
    match &err {
        sqlx::Error::Database(database_error)
            if database_error
                .message()
                .contains("UNIQUE constraint failed: users.username") =>
        {
            ApiError::Conflict("username is already registered".to_string())
        }
        _ => ApiError::from(err),
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}

#[derive(Debug, FromRow)]
struct AccountRow {
    id: i64,
    name: String,
    service: String,
    enabled: i64,
    config_json: String,
    created_at: String,
    updated_at: String,
}

impl AccountRow {
    fn into_response(self, tasks: Vec<TaskResponse>) -> Result<AccountResponse, ApiError> {
        Ok(AccountResponse {
            id: self.id,
            name: self.name,
            service: self.service,
            enabled: self.enabled != 0,
            config: serde_json::from_str(&self.config_json)
                .map_err(|err| ApiError::Internal(err.into()))?,
            created_at: self.created_at,
            updated_at: self.updated_at,
            tasks,
        })
    }
}

#[derive(Debug, FromRow)]
struct TaskRow {
    id: i64,
    account_id: i64,
    account_name: String,
    task_type: String,
    enabled: i64,
    next_run_at: String,
    last_run_at: Option<String>,
}

impl TaskRow {
    fn into_response(self) -> TaskResponse {
        TaskResponse {
            id: self.id,
            account_id: self.account_id,
            account_name: self.account_name,
            task_type: self.task_type,
            enabled: self.enabled != 0,
            next_run_at: self.next_run_at,
            last_run_at: self.last_run_at,
        }
    }
}

#[derive(Debug, FromRow)]
struct InviteRow {
    id: i64,
    code: String,
    created_at: String,
    redeemed_at: Option<String>,
    redeemed_by_username: Option<String>,
}

impl InviteRow {
    fn into_response(self) -> InviteResponse {
        InviteResponse {
            id: self.id,
            code: self.code,
            created_at: self.created_at,
            redeemed_at: self.redeemed_at,
            redeemed_by_username: self.redeemed_by_username,
        }
    }
}

#[derive(Debug, FromRow)]
struct TaskLogRow {
    id: i64,
    account_id: Option<i64>,
    account_name: Option<String>,
    task_type: String,
    status: String,
    started_at: String,
    finished_at: String,
    duration_ms: i64,
    message: String,
}

impl TaskLogRow {
    fn into_response(self) -> TaskLogResponse {
        TaskLogResponse {
            id: self.id,
            account_id: self.account_id,
            account_name: self.account_name,
            task_type: self.task_type,
            status: self.status,
            started_at: self.started_at,
            finished_at: self.finished_at,
            duration_ms: self.duration_ms,
            message: self.message,
        }
    }
}
