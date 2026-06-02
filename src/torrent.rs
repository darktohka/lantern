use std::path::PathBuf;
use std::sync::Arc;

use irontide::prelude::*;
use rand::Rng;
use reqwest::Client;
use scraper::{Html, Selector};
use sqlx::SqlitePool;
use tokio::sync::Mutex;
use tokio::time::{Duration, sleep};
use tracing::{error, info, warn};

use crate::models::NcoreConfig;
use crate::timeutil::{now, to_sql_timestamp};

const HITNRUN_INTERVAL: Duration = Duration::from_secs(12 * 3600);
const HITNRUN_JITTER: Duration = Duration::from_secs(10 * 60);

#[derive(Clone)]
pub struct TorrentKeepalive {
    pub db: SqlitePool,
    pub http: Client,
    pub session: Arc<Mutex<Option<SessionHandle>>>,
    pub output_dir: PathBuf,
}

#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
pub struct NcoreTorrentRow {
    pub id: i64,
    pub account_id: i64,
    pub ncore_id: String,
    pub info_hash: Option<String>,
    pub name: String,
    pub status: String,
    pub hnr_timespent: Option<String>,
    pub hnr_seed: Option<String>,
    pub download_url: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, serde::Serialize)]
pub struct TorrentResponse {
    pub id: i64,
    pub account_id: i64,
    pub account_name: String,
    pub ncore_id: String,
    pub info_hash: Option<String>,
    pub name: String,
    pub status: String,
    pub hnr_timespent: Option<String>,
    pub hnr_seed: Option<String>,
    pub progress: f64,
    pub download_rate: u64,
    pub upload_rate: u64,
    pub total_download: u64,
    pub total_upload: u64,
    pub created_at: String,
    pub updated_at: String,
}

pub async fn run_keepalive(k: TorrentKeepalive) {
    if let Err(err) = ensure_session(&k).await {
        warn!(error = %err, "initial session setup failed");
    }

    let hitnrun_k = k.clone();
    let hitnrun = tokio::spawn(async move {
        loop {
            if let Err(err) = hitnrun_tick(&hitnrun_k).await {
                error!(error = %err, "hitnrun tick failed");
            }
            let jitter = rand::thread_rng().gen_range(Duration::ZERO..=HITNRUN_JITTER);
            sleep(HITNRUN_INTERVAL + jitter).await;
        }
    });

    alert_listener(&k).await;

    hitnrun.abort();
}

async fn hitnrun_tick(k: &TorrentKeepalive) -> anyhow::Result<()> {
    let ncore_accounts = sqlx::query_as::<_, (i64, i64, String, String)>(
        "SELECT a.id, a.user_id, a.name, a.config_json FROM accounts a WHERE a.service = 'ncore' AND a.enabled = 1",
    )
    .fetch_all(&k.db).await?;

    if ncore_accounts.is_empty() {
        return Ok(());
    }

    for (account_id, user_id, account_name, config_json) in &ncore_accounts {
        let config: NcoreConfig = match serde_json::from_str(config_json) {
            Ok(c) => c,
            Err(err) => { warn!(account_name, error = %err, "invalid ncore config"); continue; }
        };
        if let Err(err) = process_account(k, *account_id, *user_id, account_name, &config).await {
            warn!(account_name, error = %err, "failed to process account");
        }
    }

    if let Err(err) = ensure_session(k).await {
        warn!(error = %err, "session refresh after hitnrun failed");
    }

    Ok(())
}

async fn process_account(
    k: &TorrentKeepalive,
    account_id: i64,
    user_id: i64,
    account_name: &str,
    config: &NcoreConfig,
) -> anyhow::Result<()> {
    let base_url = config.base_url.trim_end_matches('/');
    let cookies = login(base_url, &config.username, &config.password).await?;
    process_hitnrun_torrents(&k.db, &k.http, base_url, &cookies, account_id, user_id, account_name).await
}

pub(crate) async fn process_hitnrun_torrents(
    db: &SqlitePool,
    http: &Client,
    base_url: &str,
    cookies: &str,
    account_id: i64,
    user_id: i64,
    account_name: &str,
) -> anyhow::Result<()> {
    let started_at = now();
    let torrents = fetch_hitnrun(http, base_url, cookies).await?;

    let mut added = Vec::new();
    let mut completed = Vec::new();

    for ncore_torrent in &torrents {
        let ncore_id = &ncore_torrent.ncore_id;
        let hnr_timespent = &ncore_torrent.hnr_timespent;
        let hnr_seed = &ncore_torrent.hnr_seed;
        let name = &ncore_torrent.name;
        let download_url = &ncore_torrent.download_url;

        let blacklisted = sqlx::query_as::<_, (i64,)>(
            "SELECT 1 FROM ncore_blacklist b JOIN ncore_torrents t ON t.info_hash = b.info_hash WHERE b.account_id = ?1 AND t.ncore_id = ?2",
        )
        .bind(account_id)
        .bind(ncore_id)
        .fetch_optional(db)
        .await?.is_some();

        if blacklisted {
            continue;
        }

        let existing = sqlx::query_as::<_, (String,)>(
            "SELECT status FROM ncore_torrents WHERE account_id = ?1 AND ncore_id = ?2",
        )
        .bind(account_id)
        .bind(ncore_id)
        .fetch_optional(db)
        .await?;

        match existing {
            None => {
                if let Some(ref seed) = *hnr_seed {
                    if seed == "Stopped" {
                        if let Some(ref timespent) = *hnr_timespent {
                            if timespent != "-" {
                                info!(account_name, ncore_id, name, "new stopped torrent with remaining time, queueing");
                                let now_ts = to_sql_timestamp(now());
                                sqlx::query(
                                    r#"
                                    INSERT INTO ncore_torrents
                                        (account_id, ncore_id, name, status, hnr_timespent, hnr_seed, download_url, created_at, updated_at)
                                    VALUES (?1, ?2, ?3, 'pending', ?4, ?5, ?6, ?7, ?7)
                                    "#,
                                )
                                .bind(account_id)
                                .bind(ncore_id)
                                .bind(name)
                                .bind(hnr_timespent)
                                .bind(hnr_seed)
                                .bind(download_url)
                                .bind(&now_ts)
                                .execute(db)
                                .await?;
                                added.push(format!("#{} {}", ncore_id, name));
                            }
                        }
                    }
                }
            }
            Some((status,)) => {
                let now_ts = to_sql_timestamp(now());
                if hnr_timespent.as_deref() == Some("-") && status != "complete" {
                    info!(account_name, ncore_id, name, "torrent completed (remaining = '-')");
                    sqlx::query(
                        "UPDATE ncore_torrents SET status = 'complete', hnr_timespent = ?1, hnr_seed = ?2, updated_at = ?3 WHERE account_id = ?4 AND ncore_id = ?5",
                    )
                    .bind(hnr_timespent)
                    .bind(hnr_seed)
                    .bind(&now_ts)
                    .bind(account_id)
                    .bind(ncore_id)
                    .execute(db)
                    .await?;
                    completed.push(format!("#{} {}", ncore_id, name));
                } else if status == "pending" || status == "downloading" || status == "seeding" {
                    sqlx::query(
                        "UPDATE ncore_torrents SET hnr_timespent = ?1, hnr_seed = ?2, updated_at = ?3 WHERE account_id = ?4 AND ncore_id = ?5",
                    )
                    .bind(hnr_timespent)
                    .bind(hnr_seed)
                    .bind(&now_ts)
                    .bind(account_id)
                    .bind(ncore_id)
                    .execute(db)
                    .await?;
                }
            }
        }
    }

    let finished_at = now();
    let duration_ms = (finished_at - started_at).num_milliseconds();
    let status = if !added.is_empty() || !completed.is_empty() { "success" } else { "info" };

    let mut msg_parts: Vec<String> = Vec::new();
    if !added.is_empty() {
        msg_parts.push(format!("Added: {}", added.join(", ")));
    }
    if !completed.is_empty() {
        msg_parts.push(format!("Completed: {}", completed.join(", ")));
    }
    if msg_parts.is_empty() {
        msg_parts.push("No changes".to_string());
    }
    let message = msg_parts.join(" | ");

    sqlx::query(
        r#"
        INSERT INTO execution_logs
            (user_id, account_id, task_type, status, started_at, finished_at, duration_ms, message)
        VALUES (?1, ?2, 'ncore_hitnrun_check', ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind(user_id)
    .bind(account_id)
    .bind(status)
    .bind(to_sql_timestamp(started_at))
    .bind(to_sql_timestamp(finished_at))
    .bind(duration_ms)
    .bind(&message)
    .execute(db)
    .await?;

    Ok(())
}

pub(crate) async fn fetch_hitnrun(
    http: &Client,
    base_url: &str,
    cookies: &str,
) -> anyhow::Result<Vec<NcoreHitnrunTorrent>> {
    let url = format!("{}/hitnrun.php?showall=true", base_url);
    let resp = http
        .get(&url)
        .header("Cookie", cookies)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:136.0) Gecko/20100101 Firefox/136.0",
        )
        .send()
        .await?;
    let html = resp.text().await?;
    parse_hitnrun_html(&html)
}

#[derive(Debug)]
pub(crate) struct NcoreHitnrunTorrent {
    pub(crate) ncore_id: String,
    pub(crate) name: String,
    pub(crate) hnr_timespent: Option<String>,
    pub(crate) hnr_seed: Option<String>,
    pub(crate) download_url: String,
}

pub(crate) fn parse_hitnrun_html(html: &str) -> anyhow::Result<Vec<NcoreHitnrunTorrent>> {
    let document = Html::parse_document(html);
    let row_selector = Selector::parse(".hnr_all, .hnr_all2")
        .map_err(|e| anyhow::anyhow!("invalid selector: {}", e))?;
    let link_selector =
        Selector::parse(".hnr_tname a").map_err(|e| anyhow::anyhow!("invalid selector: {}", e))?;
    let timespent_selector = Selector::parse(".hnr_ttimespent")
        .map_err(|e| anyhow::anyhow!("invalid selector: {}", e))?;
    let seed_selector =
        Selector::parse(".hnr_tseed").map_err(|e| anyhow::anyhow!("invalid selector: {}", e))?;

    let mut torrents = Vec::new();
    for row in document.select(&row_selector) {
        let name = row
            .select(&link_selector)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string())
            .unwrap_or_default();
        let href = row
            .select(&link_selector)
            .next()
            .and_then(|el| el.value().attr("href"))
            .unwrap_or("")
            .to_string();
        let ncore_id = href
            .split('?')
            .nth(1)
            .and_then(|q| {
                q.split('&').find_map(|p| {
                    let mut parts = p.splitn(2, '=');
                    if parts.next()? == "id" {
                        parts.next().map(|v| v.to_string())
                    } else {
                        None
                    }
                })
            })
            .unwrap_or_default();
        let hnr_timespent = row
            .select(&timespent_selector)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string());
        let hnr_seed = row
            .select(&seed_selector)
            .next()
            .map(|el| el.text().collect::<String>().trim().to_string());
        let download_url = if href.starts_with("http") {
            href.clone()
        } else {
            href
        };
        if !ncore_id.is_empty() {
            torrents.push(NcoreHitnrunTorrent {
                ncore_id,
                name,
                hnr_timespent,
                hnr_seed,
                download_url,
            });
        }
    }
    Ok(torrents)
}

async fn ensure_session(k: &TorrentKeepalive) -> anyhow::Result<()> {
    let active_count: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM ncore_torrents WHERE status IN ('pending', 'downloading')",
    )
    .fetch_one(&k.db)
    .await?;
    let seeding_count: (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM ncore_torrents WHERE status = 'seeding'")
            .fetch_one(&k.db)
            .await?;
    let has_work = active_count.0 > 0 || seeding_count.0 > 0;

    let session_exists = { k.session.lock().await.is_some() };

    if has_work && !session_exists {
        info!("starting irontide session");
        let handle = ClientBuilder::new()
            .download_dir(k.output_dir.clone())
            .enable_dht(true)
            .enable_lsd(true)
            .enable_pex(true)
            .enable_upnp(true)
            .enable_natpmp(true)
            .active_downloads(-1)
            .active_seeds(-1)
            .active_limit(-1)
            .listen_port(42020)
            .start()
            .await?;
        *k.session.lock().await = Some(handle);
    } else if !has_work && session_exists {
        info!("no active torrents, stopping irontide session");
        if let Some(handle) = k.session.lock().await.take() {
            let _ = handle.shutdown().await;
        }
        return Ok(());
    }

    let Some(ref session) = ({ k.session.lock().await.clone() }) else {
        return Ok(());
    };

    let pending = sqlx::query_as::<_, NcoreTorrentRow>(
        "SELECT * FROM ncore_torrents WHERE status = 'pending' AND (info_hash IS NULL OR info_hash = '') ORDER BY created_at ASC",
    )
    .fetch_all(&k.db).await?;
    if !pending.is_empty() {
        info!(count = pending.len(), "processing pending torrents");
        for t in &pending {
            if let Err(err) = start_downloading(k, t).await {
                warn!(ncore_id = t.ncore_id, error = %err, "failed to start download");
            }
        }
    }

    let active = sqlx::query_as::<_, NcoreTorrentRow>(
        "SELECT * FROM ncore_torrents WHERE status IN ('downloading', 'seeding') AND info_hash IS NOT NULL AND info_hash != '' ORDER BY created_at ASC",
    ).fetch_all(&k.db).await?;
    for t in &active {
        if let Some(ref info_hash) = t.info_hash {
            if let Ok(hash) = Id20::from_hex(info_hash) {
                if !session.is_valid(hash).await {
                    info!(
                        ncore_id = t.ncore_id,
                        info_hash, "re-adding torrent to session"
                    );
                    if let Err(err) = start_downloading(k, t).await {
                        warn!(ncore_id = t.ncore_id, error = %err, "failed to re-add torrent");
                    }
                }
            }
        }
    }

    Ok(())
}

async fn alert_listener(k: &TorrentKeepalive) {
    loop {
        let rx = {
            let guard = k.session.lock().await;
            guard.as_ref().map(|s| s.subscribe())
        };

        let Some(mut rx) = rx else {
            sleep(Duration::from_secs(1)).await;
            continue;
        };

        info!("alert listener started");
        loop {
            match rx.recv().await {
                Ok(alert) => {
                    if let Err(err) = handle_alert(k, &alert).await {
                        warn!(error = %err, "failed to handle alert");
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    warn!(count = n, "alert listener lagged");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    info!("alert channel closed, session likely stopped");
                    break;
                }
            }
        }
    }
}

async fn handle_alert(k: &TorrentKeepalive, alert: &Alert) -> anyhow::Result<()> {
    match &alert.kind {
        AlertKind::StateChanged {
            info_hash,
            prev_state: _,
            new_state,
        } => {
            let hex = info_hash.to_hex();
            let new_status = match new_state {
                TorrentState::Seeding | TorrentState::Complete => "seeding",
                TorrentState::Downloading
                | TorrentState::Checking
                | TorrentState::FetchingMetadata => "downloading",
                TorrentState::Paused => "paused",
                TorrentState::Queued => "pending",
                TorrentState::Stopped => "failed",
                TorrentState::Sharing => "seeding",
            };

            let now_ts = to_sql_timestamp(now());
            let updated = sqlx::query("UPDATE ncore_torrents SET status = ?1, updated_at = ?2 WHERE info_hash = ?3 AND status != ?1")
                .bind(new_status).bind(&now_ts).bind(&hex)
                .execute(&k.db).await?.rows_affected();

            if updated > 0 {
                info!(
                    info_hash = hex,
                    new = new_status,
                    "torrent state changed (alert)"
                );
                let handle = { k.session.lock().await.clone() };
                if let Some(ref session) = handle {
                    if new_status == "seeding" {
                        session
                            .force_reannounce(*info_hash)
                            .await
                            .expect("force reannounce failed");
                    }
                }
            }
        }
        AlertKind::TorrentFinished { info_hash } => {
            let hex = info_hash.to_hex();
            info!(info_hash = hex, "torrent finished downloading (alert)");
            let now_ts = to_sql_timestamp(now());
            sqlx::query("UPDATE ncore_torrents SET status = 'seeding', updated_at = ?1 WHERE info_hash = ?2")
                .bind(&now_ts).bind(&hex).execute(&k.db).await?;
            let handle = { k.session.lock().await.clone() };
            if let Some(ref session) = handle {
                session
                    .force_reannounce(*info_hash)
                    .await
                    .expect("force reannounce failed");
            }
        }
        AlertKind::TorrentError { info_hash, message } => {
            warn!(info_hash = info_hash.to_hex(), message, "torrent error (alert)");
        }
        AlertKind::TrackerReply { info_hash, url, num_peers } => {
            info!(info_hash = info_hash.to_hex(), %url, num_peers, "tracker announce success");
        }
        AlertKind::TrackerError { info_hash, url, message } => {
            warn!(info_hash = info_hash.to_hex(), %url, %message, "tracker announce error");
        }
        AlertKind::TrackerWarning { info_hash, url, message } => {
            warn!(info_hash = info_hash.to_hex(), %url, %message, "tracker warning");
        }
        _ => {}
    }
    Ok(())
}

async fn start_downloading(k: &TorrentKeepalive, torrent: &NcoreTorrentRow) -> anyhow::Result<()> {
    let ncore_info = sqlx::query_as::<_, (i64, String, String)>(
        "SELECT a.id, a.name, a.config_json FROM accounts a WHERE a.id = ?1",
    )
    .bind(torrent.account_id)
    .fetch_optional(&k.db)
    .await?;

    let (_account_id, account_name, config_json) = match ncore_info {
        Some(v) => v,
        None => return Err(anyhow::anyhow!("account not found")),
    };

    let cache_dir = k.output_dir.join(".torrent-cache");
    tokio::fs::create_dir_all(&cache_dir).await?;
    let cache_path = cache_dir.join(format!("{}.torrent", torrent.ncore_id));

    let bytes = match tokio::fs::read(&cache_path).await {
        Ok(data) => {
            info!(
                account_name,
                ncore_id = torrent.ncore_id,
                "using cached .torrent file"
            );
            data
        }
        Err(_) => {
            let config: NcoreConfig = serde_json::from_str(&config_json)?;
            let base_url = config.base_url.trim_end_matches('/');
            let cookies = login(base_url, &config.username, &config.password).await?;
            let download_url =
                get_download_url(&k.http, base_url, &cookies, &torrent.ncore_id).await?;
            info!(
                account_name,
                ncore_id = torrent.ncore_id,
                download_url,
                "downloading .torrent file"
            );
            let resp = k.http.get(&download_url)
                .header("Cookie", &cookies)
                .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:136.0) Gecko/20100101 Firefox/136.0")
                .send().await?;
            let data = resp.bytes().await?.to_vec();
            if let Err(err) = tokio::fs::write(&cache_path, &data).await {
                warn!(ncore_id = torrent.ncore_id, error = %err, "failed to cache .torrent file");
            }
            data
        }
    };

    let handle = { k.session.lock().await.clone() };
    let Some(ref session) = handle else {
        return Ok(());
    };

    let params = AddTorrentParams::from_bytes(bytes.to_vec())
        .download_dir(k.output_dir.join(&account_name))
        .paused(false);

    match params.add_to(session).await {
        Ok(info_hash) => {
            let hex_hash = info_hash.to_hex();
            let now_ts = to_sql_timestamp(now());
            let _ = session.force_reannounce(info_hash).await;
            sqlx::query(
                "UPDATE ncore_torrents SET status = 'downloading', info_hash = ?1, updated_at = ?2 WHERE id = ?3",
            )
            .bind(&hex_hash).bind(&now_ts).bind(torrent.id)
            .execute(&k.db).await?;
            info!(
                ncore_id = torrent.ncore_id,
                info_hash = hex_hash,
                "torrent added to session"
            );
            Ok(())
        }
        Err(err) => {
            let err_str = err.to_string();
            if err_str.contains("Duplicate") || err_str.contains("already") {
                info!(
                    ncore_id = torrent.ncore_id,
                    "torrent already in session, updating status"
                );
                let now_ts = to_sql_timestamp(now());
                sqlx::query("UPDATE ncore_torrents SET status = 'downloading', updated_at = ?1 WHERE id = ?2")
                    .bind(&now_ts).bind(torrent.id).execute(&k.db).await?;
                Ok(())
            } else {
                Err(anyhow::anyhow!("failed to add torrent: {}", err))
            }
        }
    }
}

async fn get_download_url(
    http: &Client,
    base_url: &str,
    cookies: &str,
    ncore_id: &str,
) -> anyhow::Result<String> {
    let url = format!("{}/torrents.php?action=details&id={}", base_url, ncore_id);
    let resp = http
        .get(&url)
        .header("Cookie", cookies)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:136.0) Gecko/20100101 Firefox/136.0",
        )
        .send()
        .await?;
    let html = resp.text().await?;
    let document = Html::parse_document(&html);
    let download_selector =
        Selector::parse(".download > a").map_err(|e| anyhow::anyhow!("invalid selector: {}", e))?;
    if let Some(el) = document.select(&download_selector).next() {
        if let Some(href) = el.value().attr("href") {
            if href.starts_with("http") {
                return Ok(href.to_string());
            }
            return Ok(format!("{}/{}", base_url, href.trim_start_matches('/')));
        }
    }
    Err(anyhow::anyhow!(
        "could not find download link for ncore_id {}",
        ncore_id
    ))
}

pub async fn remove_torrent(
    k: &TorrentKeepalive,
    torrent_id: i64,
    account_id: i64,
) -> anyhow::Result<()> {
    let torrent = sqlx::query_as::<_, NcoreTorrentRow>(
        "SELECT * FROM ncore_torrents WHERE id = ?1 AND account_id = ?2",
    )
    .bind(torrent_id)
    .bind(account_id)
    .fetch_optional(&k.db)
    .await?
    .ok_or_else(|| anyhow::anyhow!("torrent not found"))?;

    let account_name: String = sqlx::query_scalar(
        "SELECT name FROM accounts WHERE id = ?1",
    )
    .bind(account_id)
    .fetch_optional(&k.db)
    .await?
    .unwrap_or_default();

    let info_hash = torrent.info_hash.clone().unwrap_or_default();
    if !info_hash.is_empty() {
        let handle = { k.session.lock().await.clone() };
        if let Some(ref handle) = handle {
            if let Ok(hash) = Id20::from_hex(&info_hash) {
                let stats = handle.torrent_stats(hash).await.ok();
                let is_seeding = stats.as_ref().map(|s| s.is_seeding).unwrap_or(false);
                match handle.remove_torrent_with_files(hash).await {
                    Ok(_) => info!(info_hash, "torrent removed with files (via session)"),
                    Err(err) => {
                        warn!(info_hash, error = %err, "failed to remove torrent from irontide, removing files manually")
                    }
                }
                if is_seeding {
                    let now_ts = to_sql_timestamp(now());
                    sqlx::query(
                        "INSERT OR IGNORE INTO ncore_blacklist (account_id, info_hash, created_at) VALUES (?1, ?2, ?3)",
                    ).bind(account_id).bind(&info_hash).bind(&now_ts).execute(&k.db).await?;
                    info!(info_hash, "torrent was seeding, blacklisted");
                }
            }
        }
    }

    let torrent_dir = k.output_dir.join(&account_name).join(&torrent.name);
    if torrent_dir.exists() {
        tokio::fs::remove_dir_all(&torrent_dir).await?;
        info!(path = %torrent_dir.display(), "removed torrent files from disk");
    }

    let cache_path = k.output_dir.join(".torrent-cache").join(format!("{}.torrent", torrent.ncore_id));
    if cache_path.exists() {
        tokio::fs::remove_file(&cache_path).await?;
    }

    sqlx::query("DELETE FROM ncore_torrents WHERE id = ?1")
        .bind(torrent_id)
        .execute(&k.db)
        .await?;
    info!(
        torrent_id,
        ncore_id = torrent.ncore_id,
        "torrent removed from tracking"
    );
    Ok(())
}

async fn login(base_url: &str, username: &str, password: &str) -> anyhow::Result<String> {
    let form = [
        ("set_lang", "hu"),
        ("submitted", "1"),
        ("nev", username),
        ("pass", password),
        ("ne_leptessen_ki", "1"),
    ];
    let login_client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let resp = login_client.post(format!("{}/login.php", base_url)).form(&form)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/120.0.0.0 Safari/537.36")
        .header("Referer", format!("{}/login.php", base_url)).send().await?;
    let set_cookies: Vec<String> = resp
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(|s| s.to_string()))
        .collect();
    if set_cookies.is_empty() {
        anyhow::bail!("no cookies received - login likely failed");
    }
    let cookie_string = set_cookies
        .iter()
        .filter_map(|c| c.split(';').next().map(|s| s.to_string()))
        .collect::<Vec<_>>()
        .join("; ");
    if !set_cookies.iter().any(|c| c.starts_with("pass=")) {
        anyhow::bail!("no pass cookie received - login failed");
    }
    Ok(cookie_string)
}
