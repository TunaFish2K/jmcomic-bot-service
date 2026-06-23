#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

if [ ! -x "$SCRIPT_DIR/install.sh" ]; then
  printf 'ERROR: install.sh not found next to install-offline.sh\n' >&2
  exit 1
fi

export JM_BOT_OFFLINE=1
exec "$SCRIPT_DIR/install.sh" "$@"
