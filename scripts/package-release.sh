#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
VERSION="${VERSION:-$(grep -m1 '^version =' "$ROOT_DIR/Cargo.toml" | sed -E 's/version = "([^"]+)"/\1/')}"
TARGET="${TARGET:-$(rustc -vV | awk '/host:/ {print $2}')}"
BIN_PATH="${BIN_PATH:-$ROOT_DIR/target/$TARGET/release/jmcomic-bot-service}"
OUT_DIR="${OUT_DIR:-$ROOT_DIR/dist}"
PACKAGE_NAME="jmcomic-bot-service-v${VERSION}-${TARGET}"
ARCHIVE_NAME="jmcomic-bot-service-${TARGET}.tar.gz"
STAGING_DIR="$OUT_DIR/$PACKAGE_NAME"

if [ ! -x "$BIN_PATH" ]; then
  echo "Binary not found or not executable: $BIN_PATH" >&2
  echo "Build it first, for example: cargo build --release --target $TARGET" >&2
  exit 1
fi

rm -rf "$STAGING_DIR"
mkdir -p "$STAGING_DIR/systemd" "$STAGING_DIR/scripts"

install -m 0755 "$BIN_PATH" "$STAGING_DIR/jmcomic-bot-service"
install -m 0644 "$ROOT_DIR/config.example.json" "$STAGING_DIR/config.example.json"
install -m 0644 "$ROOT_DIR/config.schema.json" "$STAGING_DIR/config.schema.json"
install -m 0644 "$ROOT_DIR/systemd/jmcomic-bot-service.service" "$STAGING_DIR/systemd/jmcomic-bot-service.service"
install -m 0755 "$ROOT_DIR/scripts/install.sh" "$STAGING_DIR/scripts/install.sh"
install -m 0644 "$ROOT_DIR/README.md" "$STAGING_DIR/README.md"
install -m 0755 "$ROOT_DIR/scripts/install.sh" "$OUT_DIR/install.sh"

(
  cd "$OUT_DIR"
  tar -czf "$ARCHIVE_NAME" "$PACKAGE_NAME"
  sha256sum "$ARCHIVE_NAME" > "$ARCHIVE_NAME.sha256"
  sha256sum "install.sh" > "install.sh.sha256"
)

echo "$OUT_DIR/$ARCHIVE_NAME"
