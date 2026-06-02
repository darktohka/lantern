use std::path::PathBuf;
use std::sync::Arc;

use sqlx::SqlitePool;
use tokio::sync::Mutex;

use crate::torrent::TorrentKeepalive;

#[derive(Clone)]
pub struct AppState {
    pub db: SqlitePool,
    pub http: reqwest::Client,
    pub torrents: TorrentKeepalive,
}

impl AppState {
    pub fn new(db: SqlitePool, http: reqwest::Client, output_dir: PathBuf) -> Self {
        Self {
            db: db.clone(),
            http: http.clone(),
            torrents: TorrentKeepalive {
                db,
                http,
                session: Arc::new(Mutex::new(None)),
                output_dir,
            },
        }
    }
}
