use std::str::FromStr;

use anyhow::Context;
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};

pub async fn connect(database_url: &str) -> anyhow::Result<SqlitePool> {
    let options = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal);

    SqlitePoolOptions::new()
        .max_connections(10)
        .connect_with(options)
        .await
        .context("failed to open sqlite database")
}

pub async fn migrate(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::raw_sql(
        r#"
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS users (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            username TEXT NOT NULL UNIQUE,
            password_hash TEXT NOT NULL,
            created_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS invite_codes (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            code TEXT NOT NULL UNIQUE,
            created_by_user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            redeemed_by_user_id INTEGER REFERENCES users(id) ON DELETE SET NULL,
            created_at TEXT NOT NULL,
            redeemed_at TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_invite_codes_created_by
            ON invite_codes(created_by_user_id, redeemed_at);

        CREATE TABLE IF NOT EXISTS sessions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            token_hash TEXT NOT NULL UNIQUE,
            created_at TEXT NOT NULL,
            expires_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_sessions_token_hash
            ON sessions(token_hash, expires_at);

        CREATE TABLE IF NOT EXISTS accounts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            service TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            config_json TEXT NOT NULL,
            cookies_json TEXT NOT NULL DEFAULT '{}',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_accounts_user_id
            ON accounts(user_id);

        CREATE TABLE IF NOT EXISTS account_tasks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            task_type TEXT NOT NULL,
            enabled INTEGER NOT NULL DEFAULT 1,
            next_run_at TEXT NOT NULL,
            last_run_at TEXT,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE UNIQUE INDEX IF NOT EXISTS idx_account_tasks_unique
            ON account_tasks(account_id, task_type);

        CREATE INDEX IF NOT EXISTS idx_account_tasks_next_run
            ON account_tasks(enabled, next_run_at);

        CREATE TABLE IF NOT EXISTS execution_logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            account_id INTEGER REFERENCES accounts(id) ON DELETE SET NULL,
            task_id INTEGER REFERENCES account_tasks(id) ON DELETE SET NULL,
            task_type TEXT NOT NULL,
            status TEXT NOT NULL,
            started_at TEXT NOT NULL,
            finished_at TEXT NOT NULL,
            duration_ms INTEGER NOT NULL,
            message TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_execution_logs_user_started
            ON execution_logs(user_id, started_at DESC);
        "#,
    )
    .execute(pool)
    .await?;

    let _ = sqlx::raw_sql(
        "ALTER TABLE accounts ADD COLUMN cookies_json TEXT NOT NULL DEFAULT '{}'",
    )
    .execute(pool)
    .await;

    sqlx::raw_sql(
        r#"
        CREATE TABLE IF NOT EXISTS ncore_torrents (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            ncore_id TEXT NOT NULL,
            info_hash TEXT,
            name TEXT NOT NULL DEFAULT '',
            status TEXT NOT NULL DEFAULT 'pending',
            hnr_timespent TEXT,
            hnr_seed TEXT,
            download_url TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE UNIQUE INDEX IF NOT EXISTS idx_ncore_torrents_ncore_id
            ON ncore_torrents(account_id, ncore_id);

        CREATE TABLE IF NOT EXISTS ncore_blacklist (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
            info_hash TEXT NOT NULL,
            created_at TEXT NOT NULL
        );

        CREATE UNIQUE INDEX IF NOT EXISTS idx_ncore_blacklist_hash
            ON ncore_blacklist(account_id, info_hash);

        CREATE TABLE IF NOT EXISTS ntfy_alerts (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
            name TEXT NOT NULL,
            topic TEXT NOT NULL,
            created_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_ntfy_alerts_user_id
            ON ntfy_alerts(user_id);
        "#,
    )
    .execute(pool)
    .await?;

    let _ = sqlx::raw_sql(
        "ALTER TABLE ntfy_alerts ADD COLUMN auth_json TEXT NOT NULL DEFAULT '{\"type\":\"anonymous\"}'",
    )
    .execute(pool)
    .await;

    Ok(())
}
