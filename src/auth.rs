use anyhow::{Context, bail};
use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::Duration;
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::{FromRow, SqlitePool};

use crate::{
    models::UserPublic,
    timeutil::{now, to_sql_timestamp},
};

const SESSION_TTL_DAYS: i64 = 30;

#[derive(Debug, FromRow)]
struct UserRow {
    id: i64,
    username: String,
    password_hash: String,
}

pub async fn create_user(
    pool: &SqlitePool,
    username: &str,
    password: &str,
) -> anyhow::Result<UserPublic> {
    validate_username(username)?;
    validate_password(password)?;

    let password_hash = hash_password(password)?;
    let created_at = to_sql_timestamp(now());

    let result = sqlx::query(
        r#"
        INSERT INTO users (username, password_hash, created_at)
        VALUES (?1, ?2, ?3)
        "#,
    )
    .bind(username.trim())
    .bind(password_hash)
    .bind(created_at)
    .execute(pool)
    .await
    .with_context(|| format!("failed to create user '{}'", username))?;

    Ok(UserPublic {
        id: result.last_insert_rowid(),
        username: username.trim().to_string(),
    })
}

pub async fn find_user_by_username(
    pool: &SqlitePool,
    username: &str,
) -> anyhow::Result<Option<UserPublic>> {
    let user = sqlx::query_as::<_, UserPublicRow>(
        r#"
        SELECT id, username
        FROM users
        WHERE username = ?1
        "#,
    )
    .bind(username.trim())
    .fetch_optional(pool)
    .await?;

    Ok(user.map(|user| UserPublic {
        id: user.id,
        username: user.username,
    }))
}

pub async fn authenticate_user(
    pool: &SqlitePool,
    username: &str,
    password: &str,
) -> anyhow::Result<Option<UserPublic>> {
    let Some(user) = sqlx::query_as::<_, UserRow>(
        r#"
        SELECT id, username, password_hash
        FROM users
        WHERE username = ?1
        "#,
    )
    .bind(username.trim())
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };

    if verify_password(password, &user.password_hash)? {
        return Ok(Some(UserPublic {
            id: user.id,
            username: user.username,
        }));
    }

    Ok(None)
}

pub async fn create_session(pool: &SqlitePool, user_id: i64) -> anyhow::Result<String> {
    let token = generate_token();
    let token_hash = hash_token(&token);
    let created_at = now();
    let expires_at = created_at + Duration::days(SESSION_TTL_DAYS);

    sqlx::query(
        r#"
        INSERT INTO sessions (user_id, token_hash, created_at, expires_at)
        VALUES (?1, ?2, ?3, ?4)
        "#,
    )
    .bind(user_id)
    .bind(token_hash)
    .bind(to_sql_timestamp(created_at))
    .bind(to_sql_timestamp(expires_at))
    .execute(pool)
    .await
    .context("failed to create session")?;

    Ok(token)
}

pub async fn create_invite_code(
    pool: &SqlitePool,
    created_by_user_id: i64,
    enforce_limit: bool,
) -> anyhow::Result<String> {
    if enforce_limit {
        let (count,): (i64,) = sqlx::query_as(
            r#"
            SELECT COUNT(*)
            FROM invite_codes
            WHERE created_by_user_id = ?1
              AND redeemed_at IS NULL
            "#,
        )
        .bind(created_by_user_id)
        .fetch_one(pool)
        .await?;

        if count >= 5 {
            bail!("a user can have at most 5 unredeemed invite codes");
        }
    }

    for _ in 0..5 {
        let code = generate_invite_code();
        let result = sqlx::query(
            r#"
            INSERT INTO invite_codes (code, created_by_user_id, created_at)
            VALUES (?1, ?2, ?3)
            "#,
        )
        .bind(&code)
        .bind(created_by_user_id)
        .bind(to_sql_timestamp(now()))
        .execute(pool)
        .await;

        if result.is_ok() {
            return Ok(code);
        }
    }

    bail!("failed to generate a unique invite code")
}

pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

pub(crate) fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|err| anyhow::anyhow!(err.to_string()))?
        .to_string())
}

fn verify_password(password: &str, password_hash: &str) -> anyhow::Result<bool> {
    let parsed_hash =
        PasswordHash::new(password_hash).map_err(|err| anyhow::anyhow!(err.to_string()))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok())
}

fn generate_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

fn generate_invite_code() -> String {
    let mut bytes = [0u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("LAN-{}", URL_SAFE_NO_PAD.encode(bytes))
}

pub(crate) fn validate_username(username: &str) -> anyhow::Result<()> {
    let username = username.trim();
    if username.len() < 3 {
        bail!("username must be at least 3 characters long");
    }

    if username.len() > 64 {
        bail!("username must be 64 characters or fewer");
    }

    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        bail!("username can contain only letters, numbers, underscores, hyphens, and dots");
    }

    Ok(())
}

pub(crate) fn validate_password(password: &str) -> anyhow::Result<()> {
    if password.len() < 8 {
        bail!("password must be at least 8 characters long");
    }

    Ok(())
}

#[derive(Debug, FromRow)]
struct UserPublicRow {
    id: i64,
    username: String,
}
