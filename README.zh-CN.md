# jmcomic-bot-service

[English](README.md)

用于机器人的 Rust VPS 后端。它复用现有 Cloudflare Worker 查询 JMComic 元数据，然后在 VPS 上扮演 Web 客户端完成图片下载、分块还原、JPEG 转换、ZIP/CBZ/PDF 打包、本地缓存和签名文件分发。

## 功能

- 使用 Bearer token 保护机器人 API。
- 通过现有 Worker 查询搜索结果、本子信息、章节图片元数据。
- 本地 SQLite 元数据缓存和本地磁盘文件缓存。
- 异步下载任务，支持进度轮询。
- 输出 ZIP、CBZ、PDF。
- 短期有效的 HMAC 签名文件 URL，下载文件时不需要 Bearer token。
- JSON 配置文件，并提供 JSON Schema 用于编辑器补全。
- 提供 systemd unit，适合 VPS 部署。

## 配置

复制 `config.example.json` 后编辑。示例里包含：

```json
"$schema": "./config.schema.json"
```

本地编辑配置时建议保留这一行，编辑器可以获得补全和校验。

默认配置路径：

```text
/etc/jmcomic-bot-service/config.json
```

也可以用 `--config /path/to/config.json` 或 `JM_BOT_CONFIG=/path/to/config.json` 覆盖。

| 字段 | 默认值 | 必填 | 说明 |
| --- | --- | --- | --- |
| `bot_tokens` | 无 | 是 | `/api/v1/*` 接受的 Bearer token，签名文件 URL 除外。 |
| `file_signing_secret` | 无 | 是 | 短期文件链接的 HMAC 密钥。 |
| `worker_base_url` | 无 | 是 | 现有 Worker 地址，例如 `https://xxx.workers.dev`。 |
| `public_base_url` | 无 | 否 | 返回绝对 `download_url` 时使用的公网服务地址。不填则返回相对 URL。 |
| `data_dir` | `/var/lib/jmcomic-bot-service` | 否 | 持久化数据目录。 |
| `database_url` | `sqlite://{data_dir}/jm-bot.db` | 否 | SQLite URL。 |
| `bind_addr` | `0.0.0.0:3000` | 否 | 监听地址。 |
| `max_concurrent_jobs` | `2` | 否 | 后台归档任务并发数。 |
| `image_concurrency` | `6` | 否 | 单个任务内图片下载和处理并发数。 |
| `signed_url_ttl_seconds` | `3600` | 否 | 生成的文件 URL 有效期。 |
| `artifact_ttl_days` | `30` | 否 | 归档文件过期时间元数据。 |
| `cache_max_bytes` | `53687091200` | 否 | 预留给缓存清理策略的容量上限。 |
| `max_pages_per_job` | `800` | 否 | 单个任务允许的最大页数。 |
| `jpeg_quality` | `90` | 否 | JPEG 输出质量，范围 1-100。 |

持久化目录结构：

- `{data_dir}/jm-bot.db`
- `{data_dir}/artifacts/{artifact_id}.{zip|cbz|pdf}`
- `{data_dir}/artifacts/covers/{album_id}.jpg`
- `{data_dir}/tmp`

## 二进制安装

VPS 安装路径不需要 Rust 或 Cargo。推送 release tag 后，GitHub Actions 会构建 Linux x86_64 二进制并发布这个资源：

```text
jmcomic-bot-service-x86_64-unknown-linux-gnu.tar.gz
```

安装最新 release：

```bash
curl -fsSL https://github.com/TunaFish2K/jmcomic-bot-service/releases/latest/download/install.sh | sudo bash
```

注意：这个 URL 只有在第一次 `v*` tag release 发布后才存在。只推送 `main` 不会生成 `latest` release；手动 `workflow_dispatch` 只会生成 Actions artifact，不会发布 GitHub Release。

安装脚本会安装：

- `/usr/local/bin/jmcomic-bot-service`
- `/etc/jmcomic-bot-service/config.json`
- `/etc/jmcomic-bot-service/config.schema.json`
- `/etc/systemd/system/jmcomic-bot-service.service`
- `/var/lib/jmcomic-bot-service`

如果安装后的配置仍包含占位符，安装脚本会保持服务停止。编辑配置后再启动：

```bash
sudoedit /etc/jmcomic-bot-service/config.json
sudo systemctl enable --now jmcomic-bot-service
journalctl -u jmcomic-bot-service -f
```

安装指定 release：

```bash
curl -fsSL https://github.com/TunaFish2K/jmcomic-bot-service/releases/download/v0.1.1/install.sh -o /tmp/install-jmcomic-bot-service.sh
sudo env JM_BOT_VERSION=v0.1.1 bash /tmp/install-jmcomic-bot-service.sh
```

安装时直接写入必要配置：

```bash
curl -fsSL https://github.com/TunaFish2K/jmcomic-bot-service/releases/latest/download/install.sh -o /tmp/install-jmcomic-bot-service.sh
sudo env \
  WORKER_BASE_URL="https://your-worker.example.workers.dev" \
  BOT_TOKEN="change-me-bot-token" \
  SIGNING_SECRET="change-me-signing-secret" \
  PUBLIC_BASE_URL="https://bot-backend.example.com" \
  bash /tmp/install-jmcomic-bot-service.sh
```

手动下载 tarball 后离线安装：

```bash
curl -fLO https://github.com/TunaFish2K/jmcomic-bot-service/releases/download/v0.1.1/jmcomic-bot-service-x86_64-unknown-linux-gnu.tar.gz
tar -xzf jmcomic-bot-service-x86_64-unknown-linux-gnu.tar.gz
cd jmcomic-bot-service-v0.1.1-x86_64-unknown-linux-gnu
sudo bash scripts/install-offline.sh
```

tarball 下载并解压后，`scripts/install-offline.sh` 只使用本地包内文件安装，不会访问 GitHub。

安装脚本环境变量：

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `JM_BOT_VERSION` | `latest` | Release tag，例如 `v0.1.1`。 |
| `JM_BOT_REPO` | `TunaFish2K/jmcomic-bot-service` | 下载 release 的 GitHub 仓库。 |
| `START_SERVICE` | `1` | 设为 `0` 时只安装，不启动 systemd 服务。 |
| `TARGET` | 自动检测 | release target。目前发布 `x86_64-unknown-linux-gnu`。 |
| `WORKER_BASE_URL` | 无 | 通过环境变量写配置时必填。 |
| `BOT_TOKEN` | 无 | 通过环境变量写配置时必填。 |
| `SIGNING_SECRET` | 无 | 通过环境变量写配置时必填。 |
| `PUBLIC_BASE_URL` | `null` | 可选，返回绝对文件 URL 时使用的公网地址。 |

环境变量写配置的逻辑刻意保持简单；token 和 URL 请使用不包含引号或换行的普通字符串。

发布新的二进制 release：

```bash
git tag v0.1.1
git push origin v0.1.1
```

tag 应该和 `Cargo.toml` 里的 package version 保持一致。tag workflow 会构建并发布 `install.sh`、tarball 和 sha256 校验文件。

## 开发

本地构建并运行：

```bash
cargo build --release
cp config.example.json ./config.json
./target/release/jmcomic-bot-service --config ./config.json
```

开发运行：

```bash
cargo run -- --config ./config.json
```

## 鉴权

`/api/v1` 下的接口都需要：

```http
Authorization: Bearer <token>
```

例外：`GET /api/v1/files/{artifact_id}?exp=&sig=` 只使用查询参数里的签名，不需要 Bearer token。

## API

### `GET /health`

公开健康检查。

响应：

```json
{ "ok": true }
```

### `GET /api/v1/search`

通过 Worker 代理搜索。

查询参数：

| 参数 | 必填 | 说明 |
| --- | --- | --- |
| `q` 或 `query` | 是 | 搜索关键词。 |
| `page` | 否 | 页码，默认 `1`。 |
| `orderBy` | 否 | `mr`、`mv`、`mp`、`tf`，默认 `mr`。 |
| `time` | 否 | `a`、`t`、`w`、`m`，默认 `a`。 |
| `mainTag` | 否 | `0`-`4`，默认 `0`。 |

响应会原样返回 Worker 的搜索 JSON。

### `GET /api/v1/albums/{album_id}`

返回本子信息，并附带适合机器人展示的排序章节列表。

响应示例：

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

下载第一个章节的第一张图片，还原切片，转换为 JPEG，存到 `/data/artifacts/covers`，并返回 `image/jpeg`。

### `POST /api/v1/downloads`

创建或复用下载任务。

请求：

```json
{
  "album_id": "123",
  "photo_ids": ["123", "124"],
  "format": "cbz",
  "force": false
}
```

字段：

| 字段 | 必填 | 说明 |
| --- | --- | --- |
| `album_id` | 是 | 本子 id，用于标题和缓存分组。 |
| `photo_ids` | 否 | 章节 id。不填时服务会获取本子信息，使用排序后的系列章节；单章节本子会使用 `album_id`。 |
| `format` | 是 | `zip`、`cbz` 或 `pdf`。 |
| `force` | 否 | 为 `true` 时跳过已有缓存，创建新任务。 |

响应：

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

状态：`queued`、`running`、`completed`、`failed`。

阶段：`queued`、`metadata`、`downloading`、`archive`、`completed`、`failed`。

如果命中归档缓存，`status` 为 `completed`，`cached` 为 `true`，并且会直接返回 `download_url`。

### `GET /api/v1/downloads/{job_id}`

轮询任务进度。

响应结构和 `POST /api/v1/downloads` 相同。

### `GET /api/v1/artifacts/{artifact_id}`

返回归档文件元数据，并生成一个新的签名下载 URL。

响应：

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

返回归档文件流。这个接口不需要 Bearer token，但必须携带 HMAC 签名和过期时间。

响应：

- `200`：文件流，包含 `Content-Disposition: attachment`。
- `401`：签名无效或已过期。
- `404`：归档记录不存在、已过期或文件缺失。

## 机器人流程

1. 搜索：`GET /api/v1/search?q=...`
2. 展示详情：`GET /api/v1/albums/{album_id}`
3. 开始归档：`POST /api/v1/downloads`
4. 轮询进度：`GET /api/v1/downloads/{job_id}`
5. 发送文件：当状态为 `completed` 时使用 `download_url`

示例：

```bash
curl -H "Authorization: Bearer dev" \
  "http://127.0.0.1:3000/api/v1/search?q=test"

curl -X POST "http://127.0.0.1:3000/api/v1/downloads" \
  -H "Authorization: Bearer dev" \
  -H "Content-Type: application/json" \
  -d '{"album_id":"123","format":"cbz"}'
```

## 验证

```bash
cargo fmt
cargo test
cargo check
scripts/coverage.sh
cargo build --release --target "$(rustc -vV | awk '/host:/ {print $2}')"
scripts/package-release.sh
```

集成测试使用 mock Worker 和 mock CDN，覆盖真实服务路径：元数据获取、图片 HTTP 下载、切片/JPEG 处理、CBZ 打包、SQLite 归档记录创建。

真实 upstream 测试默认忽略，因为它需要可访问的 Worker/CDN 路径：

```bash
JM_REAL_WORKER_BASE_URL=http://127.0.0.1:8787 \
JM_REAL_ALBUM_ID=1446932 \
cargo test --test upstream_real -- --ignored
```
