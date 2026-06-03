use sqlx::SqlitePool;
use tracing::error;

use crate::models::NtfyAlertAuth;

pub async fn send_ntfy_alerts(
    db: &SqlitePool,
    http: &reqwest::Client,
    user_id: i64,
    title: &str,
    message: &str,
) {
    let alerts = match sqlx::query_as::<_, (String, String)>(
        r#"
        SELECT topic, auth_json
        FROM ntfy_alerts
        WHERE user_id = ?1
        "#,
    )
    .bind(user_id)
    .fetch_all(db)
    .await
    {
        Ok(alerts) => alerts,
        Err(err) => {
            error!(error = %err, "failed to fetch ntfy alerts");
            return;
        }
    };

    for (topic, auth_json) in &alerts {
        let topic = topic.trim().to_string();
        if topic.is_empty() {
            continue;
        }

        let auth: NtfyAlertAuth = serde_json::from_str(auth_json).unwrap_or(NtfyAlertAuth::Anonymous);

        let url = if topic.contains('/') {
            topic.clone()
        } else {
            format!("https://ntfy.sh/{}", topic)
        };

        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("Title", reqwest::header::HeaderValue::from_str(title).unwrap());
        headers.insert("Tags", reqwest::header::HeaderValue::from_static("warning"));
        auth.apply(&mut headers);

        if let Err(err) = http
            .post(&url)
            .headers(headers)
            .body(message.to_string())
            .send()
            .await
        {
            error!(error = %err, topic = %topic, "failed to send ntfy alert");
        }
    }
}

pub async fn send_test_alert(
    http: &reqwest::Client,
    topic: &str,
    auth: &NtfyAlertAuth,
) -> Result<String, String> {
    let url = if topic.contains('/') {
        topic.to_string()
    } else {
        format!("https://ntfy.sh/{}", topic)
    };

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("Title", reqwest::header::HeaderValue::from_static("Lantern Test"));
    headers.insert("Tags", reqwest::header::HeaderValue::from_static("white_check_mark"));
    auth.apply(&mut headers);

    let resp = http
        .post(&url)
        .headers(headers)
        .body("This is a test notification from Lantern.")
        .send()
        .await
        .map_err(|err| format!("request failed: {}", err))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("HTTP {}: {}", status, body));
    }

    Ok("Test notification sent successfully".to_string())
}