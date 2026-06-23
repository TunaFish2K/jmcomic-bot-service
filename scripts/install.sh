#!/usr/bin/env bash
set -euo pipefail

SERVICE_NAME="jmcomic-bot-service"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PACKAGE_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
SERVICE_USER="${SERVICE_USER:-jmcomic-bot}"
SERVICE_GROUP="${SERVICE_GROUP:-jmcomic-bot}"
INSTALL_DIR="${INSTALL_DIR:-/usr/local/bin}"
CONFIG_DIR="${CONFIG_DIR:-/etc/jmcomic-bot-service}"
DATA_DIR="${DATA_DIR:-/var/lib/jmcomic-bot-service}"
REPO="${JM_BOT_REPO:-TunaFish2K/jmcomic-bot-service}"
VERSION="${JM_BOT_VERSION:-latest}"
START_SERVICE="${START_SERVICE:-1}"
TMP_DIR=""

log() {
  printf '%s\n' "$*"
}

fail() {
  printf 'ERROR: %s\n' "$*" >&2
  exit 1
}

need_root() {
  if [ "$(id -u)" -ne 0 ]; then
    fail "run as root, for example: sudo bash scripts/install.sh"
  fi
}

need_command() {
  command -v "$1" >/dev/null 2>&1 || fail "missing required command: $1"
}

detect_target() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os:$arch" in
    Linux:x86_64|Linux:amd64) printf 'x86_64-unknown-linux-gnu' ;;
    *) fail "unsupported platform: $os $arch. Build from source or add a release asset for this target." ;;
  esac
}

download_url() {
  local target="$1"
  local file="jmcomic-bot-service-${target}.tar.gz"
  if [ "$VERSION" = "latest" ]; then
    printf 'https://github.com/%s/releases/latest/download/%s' "$REPO" "$file"
  else
    printf 'https://github.com/%s/releases/download/%s/%s' "$REPO" "$VERSION" "$file"
  fi
}

download_release() {
  local target="$1"
  local url file archive release_dir
  file="jmcomic-bot-service-${target}.tar.gz"
  archive="$TMP_DIR/$file"
  url="$(download_url "$target")"
  log "Downloading $url"
  curl -fL "$url" -o "$archive"
  if command -v sha256sum >/dev/null 2>&1 && curl -fsL "$url.sha256" -o "$archive.sha256"; then
    (cd "$TMP_DIR" && sha256sum -c "$file.sha256")
  fi
  tar -xzf "$archive" -C "$TMP_DIR"
  release_dir="$(find "$TMP_DIR" -mindepth 1 -maxdepth 1 -type d -name "jmcomic-bot-service-*" -print -quit)"
  [ -n "$release_dir" ] || fail "release archive did not contain a jmcomic-bot-service directory"
  printf '%s\n' "$release_dir"
}

source_dir() {
  local target="$1"
  if [ -x "./jmcomic-bot-service" ] && [ -f "./config.example.json" ]; then
    pwd
    return
  fi
  if [ -x "$PACKAGE_DIR/jmcomic-bot-service" ] && [ -f "$PACKAGE_DIR/config.example.json" ]; then
    printf '%s\n' "$PACKAGE_DIR"
    return
  fi
  download_release "$target"
}

ensure_user_and_dirs() {
  if ! getent group "$SERVICE_GROUP" >/dev/null; then
    groupadd --system "$SERVICE_GROUP"
  fi
  if ! id "$SERVICE_USER" >/dev/null 2>&1; then
    useradd --system --home "$DATA_DIR" --shell /usr/sbin/nologin --gid "$SERVICE_GROUP" "$SERVICE_USER"
  fi

  install -d -m 0755 "$INSTALL_DIR"
  install -d -m 0755 "$CONFIG_DIR"
  install -d -o "$SERVICE_USER" -g "$SERVICE_GROUP" -m 0755 "$DATA_DIR"
}

install_files() {
  local src="$1"
  install -m 0755 "$src/jmcomic-bot-service" "$INSTALL_DIR/jmcomic-bot-service"
  install -m 0644 "$src/config.schema.json" "$CONFIG_DIR/config.schema.json"
  install -m 0644 "$src/systemd/jmcomic-bot-service.service" "/etc/systemd/system/jmcomic-bot-service.service"

  if [ ! -f "$CONFIG_DIR/config.json" ]; then
    install -m 0644 "$src/config.example.json" "$CONFIG_DIR/config.json"
  fi
}

maybe_write_config() {
  local config="$CONFIG_DIR/config.json"
  if [ -z "${WORKER_BASE_URL:-}" ] && [ -z "${BOT_TOKEN:-}" ] && [ -z "${SIGNING_SECRET:-}" ]; then
    return
  fi

  [ -n "${WORKER_BASE_URL:-}" ] || fail "WORKER_BASE_URL is required when writing config from environment"
  [ -n "${BOT_TOKEN:-}" ] || fail "BOT_TOKEN is required when writing config from environment"
  [ -n "${SIGNING_SECRET:-}" ] || fail "SIGNING_SECRET is required when writing config from environment"

  local public_base_json="null"
  if [ -n "${PUBLIC_BASE_URL:-}" ]; then
    public_base_json="\"${PUBLIC_BASE_URL}\""
  fi

  cat > "$config" <<EOF
{
  "\$schema": "./config.schema.json",
  "bot_tokens": ["${BOT_TOKEN}"],
  "file_signing_secret": "${SIGNING_SECRET}",
  "worker_base_url": "${WORKER_BASE_URL}",
  "public_base_url": ${public_base_json},
  "data_dir": "${DATA_DIR}",
  "database_url": "sqlite://${DATA_DIR}/jm-bot.db",
  "bind_addr": "${BIND_ADDR:-0.0.0.0:3000}",
  "max_concurrent_jobs": ${MAX_CONCURRENT_JOBS:-2},
  "image_concurrency": ${IMAGE_CONCURRENCY:-6},
  "signed_url_ttl_seconds": ${SIGNED_URL_TTL_SECONDS:-3600},
  "artifact_ttl_days": ${ARTIFACT_TTL_DAYS:-30},
  "cache_max_bytes": ${CACHE_MAX_BYTES:-53687091200},
  "max_pages_per_job": ${MAX_PAGES_PER_JOB:-800},
  "jpeg_quality": ${JPEG_QUALITY:-90}
}
EOF
}

config_has_placeholders() {
  grep -q 'change-me\|your-worker.example' "$CONFIG_DIR/config.json"
}

start_service() {
  systemctl daemon-reload
  if [ "$START_SERVICE" != "1" ]; then
    log "Installed. START_SERVICE=$START_SERVICE, service not started."
    return
  fi
  if config_has_placeholders; then
    log "Installed, but config still contains placeholders."
    log "Edit $CONFIG_DIR/config.json, then run:"
    log "  systemctl enable --now jmcomic-bot-service"
    return
  fi
  systemctl enable --now jmcomic-bot-service
  systemctl --no-pager --full status jmcomic-bot-service || true
}

main() {
  need_root
  need_command curl
  need_command tar
  need_command systemctl

  TMP_DIR="$(mktemp -d)"
  trap 'rm -rf "$TMP_DIR"' EXIT

  local target src
  target="${TARGET:-$(detect_target)}"
  src="$(source_dir "$target")"

  ensure_user_and_dirs
  install_files "$src"
  maybe_write_config
  start_service
}

main "$@"
