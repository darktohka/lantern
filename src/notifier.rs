use sqlx::FromRow;
use sqlx::SqlitePool;
use tracing::error;

#[derive(Debug, FromRow)]
struct NtfyAlertRow {
    topic: String,
}

pub async fn send_ntfy_alerts(
    db: &SqlitePool,
    http: &reqwest::Client,
    user_id: i64,
    title: &str,
    message: &str,
) {
    let alerts = match sqlx::query_as::<_, NtfyAlertRow>(
        r#"
        SELECT topic
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

    for alert in &alerts {
        let topic = alert.topic.trim().to_string();
        if topic.is_empty() {
            continue;
        }

        let url = if topic.contains('/') {
            topic.clone()
        } else {
            format!("https://ntfy.sh/{}", topic)
        };

        if let Err(err) = http
            .post(&url)
            .header("Title", title)
            .header("Tags", "warning")
            .body(message.to_string())
            .send()
            .await
        {
            error!(error = %err, topic = %topic, "failed to send ntfy alert");
        }
    }
}
