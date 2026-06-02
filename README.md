# Lantern

Automated daily check-in scheduler for Hoyoverse games (Genshin Impact, Honkai: Star Rail, Zenless Zone Zero) and nCore, with built-in BitTorrent seed management.

## Features

- **Hoyoverse check-in** - Daily HoYoLab rewards for Genshin Impact, Star Rail, and Zenless Zone Zero
- **nCore check-in** - Daily presence check for the Hungarian private tracker
- **nCore HnR tracking** - Monitors Hit-and-Run torrent status, seeds automatically
- **Built-in BitTorrent client** - Powered by [irontide], no external client needed
- **Push notifications** - ntfy.sh alerts on task failures
- **Web dashboard** - React SPA for managing accounts, tasks, invites, and logs
- **Invite-only registration** - Secure user onboarding via invite codes
- **SQLite storage** - Simple single-file database, no external dependencies

## Docker

Pre-built multi-arch images (amd64 & arm64) are available on Docker Hub:

```
docker.io/darktohka/lantern:latest
```

### Quick start

```yaml
# docker-compose.yml
services:
  lantern:
    image: darktohka/lantern:latest
    container_name: lantern
    restart: unless-stopped
    ports:
      - "3000:3000"
    volumes:
      - ./data:/data
    environment:
      - LANTERN_BIND=0.0.0.0:3000
      - LANTERN_STATIC_DIR=/static
      - LANTERN_DATABASE_URL=sqlite:///data/lantern.db
      - LANTERN_TORRENT_DIR=/data/torrents
      - RUST_LOG=lantern=info,tower_http=info
```

```bash
docker compose up -d
```

### Building locally

```bash
docker compose build
# or
docker build --build-arg PROFILE=release-lto -t lantern .
```

## Usage

### Web UI

Navigate to `http://localhost:3000`, register with an invite code, and add your accounts.

### CLI

```bash
# Create a user (bypasses invite system)
lantern user create --username admin --password secret

# Generate an invite code for a user
lantern invite create --username admin

# Start the web server
lantern serve
```

## Configuration

All configuration is via environment variables.

| Variable | Default | Description |
|----------|---------|-------------|
| `LANTERN_DATABASE_URL` | `sqlite://lantern.db` | SQLite database path |
| `LANTERN_BIND` | `0.0.0.0:3000` | HTTP listen address |
| `LANTERN_STATIC_DIR` | `frontend/dist` | Path to built frontend files |
| `LANTERN_TORRENT_DIR` | `/data/torrents` | Torrent download directory |
| `RUST_LOG` | `lantern=info,tower_http=info` | Logging filter |

## Building from source

Prerequisites: Rust nightly, Bun (for frontend).

```bash
# Build everything
cargo build --profile release-lto

# Build frontend separately
cd frontend && bun install && bun run build && cd ..

# Run
LANTERN_STATIC_DIR=frontend/dist ./target/release-lto/lantern serve
```

## API

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/api/health` | Health check |
| POST | `/api/auth/register` | Register with invite code |
| POST | `/api/auth/login` | Login |
| POST | `/api/auth/logout` | Logout |
| GET | `/api/auth/me` | Current user info |
| GET/POST | `/api/accounts` | List / create accounts |
| PUT/DELETE | `/api/accounts/{id}` | Update / delete account |
| GET/POST/DELETE | `/api/invites` | Manage invite codes |
| GET | `/api/tasks` | List scheduled tasks |
| POST | `/api/tasks/{id}/run` | Trigger task manually |
| GET | `/api/task-logs` | Paginated execution logs |
| GET/DELETE | `/api/torrents` | List / remove torrents |
| GET/POST/DELETE | `/api/ntfy-alerts` | Manage ntfy.sh alert targets |
