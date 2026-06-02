mod api;
mod auth;
mod db;
mod hoyoverse;
mod models;
mod ncore;
mod scheduler;
mod state;
mod timeutil;
mod torrent;

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser, Debug)]
#[command(name = "lantern", about = "Lantern account task scheduler")]
struct Cli {
    #[arg(
        long,
        env = "LANTERN_DATABASE_URL",
        default_value = "sqlite://lantern.db"
    )]
    database_url: String,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    Serve(ServeArgs),
    User(UserArgs),
    Invite(InviteArgs),
}

#[derive(Args, Debug)]
struct ServeArgs {
    #[arg(long, env = "LANTERN_BIND", default_value = "0.0.0.0:3000")]
    bind: String,

    #[arg(long, env = "LANTERN_STATIC_DIR", default_value = "frontend/dist")]
    static_dir: String,

    #[arg(long, env = "LANTERN_TORRENT_DIR", default_value = "/data/torrents")]
    torrent_dir: String,
}

#[derive(Args, Debug)]
struct UserArgs {
    #[command(subcommand)]
    command: UserCommand,
}

#[derive(Subcommand, Debug)]
enum UserCommand {
    Create {
        #[arg(long)]
        username: String,

        #[arg(long)]
        password: String,
    },
}

#[derive(Args, Debug)]
struct InviteArgs {
    #[command(subcommand)]
    command: InviteCommand,
}

#[derive(Subcommand, Debug)]
enum InviteCommand {
    Create {
        #[arg(long)]
        username: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("lantern=info,tower_http=info")),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();
    let command = cli.command.unwrap_or(Command::Serve(ServeArgs {
        bind: "0.0.0.0:3000".to_string(),
        static_dir: "frontend/dist".to_string(),
        torrent_dir: "/data/torrents".to_string(),
    }));

    let pool = db::connect(&cli.database_url)
        .await
        .with_context(|| format!("failed to connect to {}", cli.database_url))?;
    db::migrate(&pool).await?;

    match command {
        Command::Serve(args) => api::serve(pool, args.bind, args.static_dir, args.torrent_dir).await,
        Command::User(args) => match args.command {
            UserCommand::Create { username, password } => {
                let user = auth::create_user(&pool, &username, &password).await?;
                println!("Created user '{}' with id {}", user.username, user.id);
                Ok(())
            }
        },
        Command::Invite(args) => match args.command {
            InviteCommand::Create { username } => {
                let user = auth::find_user_by_username(&pool, &username)
                    .await?
                    .with_context(|| format!("user '{}' does not exist", username))?;
                let code = auth::create_invite_code(&pool, user.id, true).await?;
                println!("{}", code);
                Ok(())
            }
        },
    }
}
