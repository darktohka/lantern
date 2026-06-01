use reqwest::Client;
use sqlx::SqlitePool;
use tracing::{info, warn};

use crate::models::{NcoreConfig, TaskOutcome};
use crate::timeutil::{now, to_sql_timestamp};

pub async fn run_daily_checkin(
    pool: &SqlitePool,
    http: &Client,
    account_name: &str,
    account_id: i64,
    config: &NcoreConfig,
) -> TaskOutcome {
    let base_url = config.base_url.trim_end_matches('/');

    let cookies = match login(base_url, &config.username, &config.password).await {
        Ok(cookies) => cookies,
        Err(err) => {
            return TaskOutcome {
                success: false,
                message: format!("nCore login failed for {}: {}", account_name, err),
            };
        }
    };

    if let Err(err) =
        sqlx::query("UPDATE accounts SET cookies_json = ?1, updated_at = ?2 WHERE id = ?3")
            .bind(&cookies)
            .bind(to_sql_timestamp(now()))
            .bind(account_id)
            .execute(pool)
            .await
    {
        return TaskOutcome {
            success: false,
            message: format!("nCore: failed to store cookies for {}: {}", account_name, err),
        };
    }

    info!(
        account_name = account_name,
        "nCore cookies stored successfully"
    );

    match perform_checkin(http, base_url, &cookies).await {
        Ok(msg) => TaskOutcome {
            success: true,
            message: format!("nCore: {}: {}", account_name, msg),
        },
        Err(err) => TaskOutcome {
            success: true,
            message: format!(
                "nCore: {}: logged in but check-in may have failed: {}",
                account_name, err
            ),
        },
    }
}

async fn login(
    base_url: &str,
    username: &str,
    password: &str,
) -> Result<String, String> {
    let form = [
        ("set_lang", "hu"),
        ("submitted", "1"),
        ("nev", username),
        ("pass", password),
        ("ne_leptessen_ki", "1"),
    ];

    let login_client = Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|err| format!("failed to build login client: {}", err))?;

    let resp = login_client
        .post(format!("{}/login.php", base_url))
        .form(&form)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        )
        .header(
            "Referer",
            format!("{}/login.php", base_url),
        )
        .send()
        .await
        .map_err(|err| format!("request failed: {}", err))?;

    let set_cookies: Vec<String> = resp
        .headers()
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(|s| s.to_string()))
        .collect();

    if set_cookies.is_empty() {
        return Err("no cookies received - login likely failed".to_string());
    }

    let mut cookie_parts: Vec<String> = Vec::new();
    for c in &set_cookies {
        if let Some(name_value) = c.split(';').next() {
            cookie_parts.push(name_value.to_string());
        }
    }
    let cookie_string = cookie_parts.join("; ");

    if !set_cookies.iter().any(|c| c.starts_with("pass=")) {
        return Err("no pass cookie received - login failed".to_string());
    }

    Ok(cookie_string)
}

async fn perform_checkin(
    http: &Client,
    base_url: &str,
    cookies: &str,
) -> Result<String, String> {
    let resp = http
        .get(base_url)
        .header("Cookie", cookies)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        )
        .send()
        .await
        .map_err(|err| format!("check-in request failed: {}", err))?;

    if !resp.status().is_success() {
        return Err(format!("check-in returned status {}", resp.status()));
    }

    let body = resp
        .text()
        .await
        .map_err(|err| format!("failed to read response: {}", err))?;

    if body.contains("Kijelentkezés") || body.contains("Kilépés") {
        Ok("daily check-in successful".to_string())
    } else {
        warn!(
            "nCore check-in response did not contain expected logged-in indicators"
        );
        Ok("daily check-in completed (logged-in status uncertain)".to_string())
    }
}
