# jmcomic-bot-service

Rust VPS backend for a bot: it reuses the existing Cloudflare Worker for JMComic metadata, then acts like the web client for image download, slice restore, JPEG conversion, archive/PDF generation, local caching, and signed file delivery.

## Features

- Bearer-token protected bot API.
- Worker-backed search, album info, chapter photo metadata.
- Local SQLite metadata cache and local disk artifact cache.
- Async download jobs with progress polling.
- ZIP, CBZ, and PDF output.
- Short-lived HMAC-signed file URLs that do not require bearer auth.
- JSON config file with JSON Schema editor completion.
- systemd unit for VPS deployment.

## Configuration

Copy `config.example.json` and edit it. The example includes:

```json
"$schema": "./config.schema.json"
```

Keep that line when editing locally to get editor completion and validation.

Default config path:

```text
/etc/jmcomic-bot-service/config.json
```

You can override it with `--config /path/to/config.json` or `JM_BOT_CONFIG=/path/to/config.json`.

| Field | Default | Required | Description |
| --- | --- | --- | --- |
| `bot_tokens` | none | yes | Bearer tokens accepted by `/api/v1/*`, except signed file URLs. |
| `file_signing_secret` | none | yes | HMAC secret for short-lived file links. |
| `worker_base_url` | none | yes | Existing Worker base URL, for example `https://xxx.workers.dev`. |
| `public_base_url` | none | no | Public service origin used when returning absolute `download_url`s. If omitted, URLs are relative. |
| `data_dir` | `/var/lib/jmcomic-bot-service` | no | Persistent data root. |
| `database_url` | `sqlite://{data_dir}/jm-bot.db` | no | SQLite URL. |
| `bind_addr` | `0.0.0.0:3000` | no | Listen address. |
| `max_concurrent_jobs` | `2` | no | Number of background archive jobs. |
| `image_concurrency` | `6` | no | Parallel image downloads/processes inside one job. |
| `signed_url_ttl_seconds` | `3600` | no | Generated file URL lifetime. |
| `artifact_ttl_days` | `30` | no | Artifact expiry metadata. |
| `cache_max_bytes` | `53687091200` | no | Reserved for cache pruning policy. |
| `max_pages_per_job` | `800` | no | Hard page-count limit per job. |
| `jpeg_quality` | `90` | no | JPEG output quality, 1-100. |

Persistent layout:

- `{data_dir}/jm-bot.db`
- `{data_dir}/artifacts/{artifact_id}.{zip|cbz|pdf}`
- `{data_dir}/artifacts/covers/{album_id}.jpg`
- `{data_dir}/tmp`

## Run

Build and run locally:

```bash
cargo build --release
cp config.example.json ./config.json
./target/release/jmcomic-bot-service --config ./config.json
```

Development:

```bash
cargo run -- --config ./config.json
```

## systemd

Build the binary on the VPS:

```bash
cargo build --release
sudo install -m 0755 target/release/jmcomic-bot-service /usr/local/bin/jmcomic-bot-service
```

Install config and schema:

```bash
sudo install -d -m 0755 /etc/jmcomic-bot-service
sudo install -m 0644 config.example.json /etc/jmcomic-bot-service/config.json
sudo install -m 0644 config.schema.json /etc/jmcomic-bot-service/config.schema.json
sudoedit /etc/jmcomic-bot-service/config.json
```

Create the service user and data directory:

```bash
sudo useradd --system --home /var/lib/jmcomic-bot-service --shell /usr/sbin/nologin jmcomic-bot
sudo install -d -o jmcomic-bot -g jmcomic-bot -m 0755 /var/lib/jmcomic-bot-service
```

Install and start the unit:

```bash
sudo install -m 0644 systemd/jmcomic-bot-service.service /etc/systemd/system/jmcomic-bot-service.service
sudo systemctl daemon-reload
sudo systemctl enable --now jmcomic-bot-service
sudo systemctl status jmcomic-bot-service
```

Logs:

```bash
journalctl -u jmcomic-bot-service -f
```

## Authentication

All endpoints under `/api/v1` require:

```http
Authorization: Bearer <token>
```

Exception: `GET /api/v1/files/{artifact_id}?exp=&sig=` uses only the signed query string.

## API

### `GET /health`

Public health check.

Response:

```json
{ "ok": true }
```

### `GET /api/v1/search`

Proxy search through the Worker.

Query:

| Name | Required | Description |
| --- | --- | --- |
| `q` or `query` | yes | Search keyword. |
| `page` | no | Page number, default `1`. |
| `orderBy` | no | `mr`, `mv`, `mp`, `tf`; default `mr`. |
| `time` | no | `a`, `t`, `w`, `m`; default `a`. |
| `mainTag` | no | `0`-`4`, default `0`. |

Response is the Worker search JSON unchanged.

### `GET /api/v1/albums/{album_id}`

Returns album info plus bot-friendly sorted chapter list.

Response:

```json
{
  "id": "123",
  "name": "Album title",
  "images": ["00001.jpg"],
  "description": "intro",
  "totalViews": "1000",
  "likes": "99",
  "series": [{ "id": "123", "name": "第1話", "sort": "1" }],
  "seriesID": "123",
  "author": ["author"],
  "tags": ["tag"],
  "works": [],
  "actors": [],
  "chapters": [{ "id": "123", "name": "第1話", "order": 1 }]
}
```

### `GET /api/v1/albums/{album_id}/cover`

Downloads the first chapter image, restores slices, converts it to JPEG, stores it under `/data/artifacts/covers`, and returns `image/jpeg`.

### `POST /api/v1/downloads`

Creates or reuses a download job.

Request:

```json
{
  "album_id": "123",
  "photo_ids": ["123", "124"],
  "format": "cbz",
  "force": false
}
```

Fields:

| Name | Required | Description |
| --- | --- | --- |
| `album_id` | yes | Album id used for title/cache grouping. |
| `photo_ids` | no | Chapter ids. If omitted, service fetches the album and uses sorted series chapters, or `album_id` for single-chapter albums. |
| `format` | yes | `zip`, `cbz`, or `pdf`. |
| `force` | no | If `true`, bypasses existing cached artifact and creates a new job. |

Response:

```json
{
  "job_id": "uuid",
  "status": "queued",
  "format": "cbz",
  "album_id": "123",
  "photo_ids": ["123", "124"],
  "stage": "queued",
  "progress_done": 0,
  "progress_total": 0,
  "cached": false,
  "artifact_id": null,
  "download_url": null,
  "error": null,
  "created_at": 1771682400,
  "updated_at": 1771682400
}
```

Statuses: `queued`, `running`, `completed`, `failed`.

Stages: `queued`, `metadata`, `downloading`, `archive`, `completed`, `failed`.

If an artifact cache hit occurs, `status` is `completed`, `cached` is `true`, and `download_url` is already available.

### `GET /api/v1/downloads/{job_id}`

Polls job progress.

Response shape is the same as `POST /api/v1/downloads`.

### `GET /api/v1/artifacts/{artifact_id}`

Returns artifact metadata and a fresh signed URL.

Response:

```json
{
  "artifact_id": "uuid",
  "format": "cbz",
  "title": "Album title",
  "size_bytes": 123456,
  "sha256": "hex",
  "page_count": 120,
  "download_url": "/api/v1/files/uuid?exp=1771686000&sig=...",
  "created_at": 1771682400,
  "last_accessed_at": 1771682400,
  "expires_at": 1774274400
}
```

### `GET /api/v1/files/{artifact_id}?exp=&sig=`

Serves the artifact file. No bearer token is required; the HMAC signature and expiry are mandatory.

Responses:

- `200`: file stream with `Content-Disposition: attachment`.
- `401`: invalid or expired signature.
- `404`: artifact row missing, expired, or file missing.

## Bot Flow

1. Search: `GET /api/v1/search?q=...`
2. Show details: `GET /api/v1/albums/{album_id}`
3. Start archive: `POST /api/v1/downloads`
4. Poll: `GET /api/v1/downloads/{job_id}`
5. Send file: use `download_url` when status is `completed`

Example:

```bash
curl -H "Authorization: Bearer dev" \
  "http://127.0.0.1:3000/api/v1/search?q=test"

curl -X POST "http://127.0.0.1:3000/api/v1/downloads" \
  -H "Authorization: Bearer dev" \
  -H "Content-Type: application/json" \
  -d '{"album_id":"123","format":"cbz"}'
```

## Verification

```bash
cargo fmt
cargo test
cargo check
scripts/coverage.sh
```

The integration test uses a mock Worker and mock CDN, then runs the real service path: metadata fetch, image HTTP download, slice/JPEG processing, CBZ packaging, SQLite artifact record creation.

The real upstream test is ignored by default because it requires a reachable Worker/CDN path:

```bash
JM_REAL_WORKER_BASE_URL=http://127.0.0.1:8787 \
JM_REAL_ALBUM_ID=1446932 \
cargo test --test upstream_real -- --ignored
```
