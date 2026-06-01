use reqwest::{
    Client,
    header::{HeaderMap, HeaderValue},
};
use serde::{Deserialize, Serialize};

use crate::models::{HoyoverseConfig, TaskOutcome};

pub struct Game {
    name: &'static str,
    act_id: &'static str,
    url_get_status: &'static str,
    url_sign: &'static str,
    rpc_sign_game: Option<&'static str>,
}

const GAMES: &[Game] = &[
    Game {
        name: "Genshin Impact",
        act_id: "e202102251931481",
        url_get_status: "https://sg-hk4e-api.hoyolab.com/event/sol/info",
        url_sign: "https://sg-hk4e-api.hoyolab.com/event/sol/sign",
        rpc_sign_game: None,
    },
    Game {
        name: "Honkai Star Rail",
        act_id: "e202303301540311",
        url_get_status: "https://sg-public-api.hoyolab.com/event/luna/os/info",
        url_sign: "https://sg-public-api.hoyolab.com/event/luna/os/sign",
        rpc_sign_game: None,
    },
    Game {
        name: "Zenless Zone Zero",
        act_id: "e202406031448091",
        url_get_status: "https://sg-public-api.hoyolab.com/event/luna/zzz/os/info",
        url_sign: "https://sg-public-api.hoyolab.com/event/luna/zzz/os/sign",
        rpc_sign_game: Some("zzz"),
    },
];

#[derive(Serialize)]
struct SignRequest {
    act_id: String,
}

#[derive(Deserialize)]
struct SignData {
    is_sign: Option<bool>,
}

#[derive(Deserialize)]
struct SignResponse {
    retcode: Option<i32>,
    message: Option<String>,
    data: Option<SignData>,
}

pub async fn run_daily_checkin(
    client: &Client,
    account_name: &str,
    config: &HoyoverseConfig,
) -> TaskOutcome {
    let checkin = HoyolabCheckin {
        account_name,
        config,
        client,
        games: GAMES,
    };

    checkin.process().await
}

struct HoyolabCheckin<'a> {
    account_name: &'a str,
    config: &'a HoyoverseConfig,
    client: &'a Client,
    games: &'a [Game],
}

impl HoyolabCheckin<'_> {
    async fn get_status(&self, game: &Game) -> Result<bool, String> {
        let response: SignResponse = self
            .client
            .get(game.url_get_status)
            .query(&[("lang", "en-us"), ("act_id", game.act_id)])
            .headers(self.build_headers(game)?)
            .send()
            .await
            .map_err(|err| err.to_string())?
            .json()
            .await
            .map_err(|err| err.to_string())?;

        let return_code = response.retcode.unwrap_or(0);

        if return_code != 0 {
            return Err(response
                .message
                .unwrap_or_else(|| format!("return code is {}", return_code)));
        }

        Ok(response
            .data
            .map_or(false, |data| data.is_sign.unwrap_or(false)))
    }

    async fn sign(&self, game: &Game) -> Result<(), String> {
        let data = serde_json::to_string(&SignRequest {
            act_id: game.act_id.to_string(),
        })
        .map_err(|err| err.to_string())?;

        let response: SignResponse = self
            .client
            .post(game.url_sign)
            .query(&[("lang", "en-us")])
            .headers(self.build_headers(game)?)
            .body(data)
            .send()
            .await
            .map_err(|err| err.to_string())?
            .json()
            .await
            .map_err(|err| err.to_string())?;

        let return_code = response.retcode.unwrap_or(0);

        if return_code == -5003 {
            return Ok(());
        }

        if return_code != 0 {
            return Err(response
                .message
                .unwrap_or_else(|| format!("return code is {}", return_code)));
        }

        Ok(())
    }

    async fn process_game(&self, game: &Game) -> (bool, String) {
        match self.get_status(game).await {
            Ok(false) => {
                if let Err(err) = self.sign(game).await {
                    return (
                        false,
                        format!(
                            "{}: failed to sign in for {}: {}",
                            game.name, self.account_name, err
                        ),
                    );
                }

                match self.get_status(game).await {
                    Ok(true) => (true, format!("{}: daily check-in successful", game.name)),
                    Ok(false) => (
                        false,
                        format!(
                            "{}: check-in did not register after sign request",
                            game.name
                        ),
                    ),
                    Err(err) => (
                        false,
                        format!("{}: failed to verify sign-in: {}", game.name, err),
                    ),
                }
            }
            Ok(true) => (true, format!("{}: daily check-in already done", game.name)),
            Err(err) => (
                false,
                format!("{}: failed to read check-in status: {}", game.name, err),
            ),
        }
    }

    async fn process(&self) -> TaskOutcome {
        let mut success = true;
        let mut messages = Vec::with_capacity(self.games.len());

        for game in self.games {
            let (game_success, message) = self.process_game(game).await;
            success = success && game_success;
            messages.push(message);
        }

        TaskOutcome {
            success,
            message: messages.join("; "),
        }
    }

    fn build_headers(&self, game: &Game) -> Result<HeaderMap, String> {
        let mut headers = HeaderMap::new();

        headers.insert(
            "Accept",
            HeaderValue::from_static("application/json, text/plain, */*"),
        );
        headers.insert(
            "Accept-Language",
            HeaderValue::from_static("en-US,en;q=0.5"),
        );
        headers.insert(
            "Origin",
            HeaderValue::from_static("https://act.hoyolab.com"),
        );
        headers.insert(
            "Referer",
            HeaderValue::from_static("https://act.hoyolab.com"),
        );
        headers.insert(
            "Content-Type",
            HeaderValue::from_static("application/json;charset=utf-8"),
        );
        headers.insert("User-Agent", HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/116.0.0.0 Safari/537.36"));
        headers.insert("x-rpc-app_version", HeaderValue::from_static("2.34.1"));
        headers.insert("x-rpc-client_type", HeaderValue::from_static("4"));

        if let Some(rpc_sign_game) = game.rpc_sign_game {
            headers.insert(
                "x-rpc-signgame",
                HeaderValue::from_str(rpc_sign_game).map_err(|err| err.to_string())?,
            );
        }

        let cookie = format!(
            "ltoken_v2={}; ltuid_v2={}; ltmid_v2={}",
            self.config.ltoken_v2, self.config.ltuid_v2, self.config.ltmid_v2
        );
        headers.insert(
            "Cookie",
            HeaderValue::from_str(&cookie).map_err(|err| err.to_string())?,
        );

        Ok(headers)
    }
}
