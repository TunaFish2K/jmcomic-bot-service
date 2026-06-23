#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COVERAGE_DIR="$ROOT_DIR/target/coverage"
COVERAGE_TARGET_DIR="$ROOT_DIR/target/coverage-target"
THRESHOLD="${COVERAGE_THRESHOLD:-80}"

rm -rf "$COVERAGE_DIR" "$COVERAGE_TARGET_DIR"
mkdir -p "$COVERAGE_DIR"

export CARGO_TARGET_DIR="$COVERAGE_TARGET_DIR"
export RUSTFLAGS="-Cinstrument-coverage"
export LLVM_PROFILE_FILE="$COVERAGE_DIR/%p-%m.profraw"

cd "$ROOT_DIR"
cargo test

llvm-profdata merge -sparse "$COVERAGE_DIR"/*.profraw -o "$COVERAGE_DIR/coverage.profdata"

mapfile -t OBJECTS < <(
  find "$COVERAGE_TARGET_DIR/debug/deps" \
    -maxdepth 1 \
    -type f \
    -perm -111 \
    ! -name '*.so' \
    ! -name '*.d' \
    | sort
)

if [ "${#OBJECTS[@]}" -eq 0 ]; then
  echo "No coverage objects found." >&2
  exit 1
fi

MAIN_OBJECT="${OBJECTS[0]}"
EXTRA_OBJECT_ARGS=()
for object in "${OBJECTS[@]:1}"; do
  EXTRA_OBJECT_ARGS+=("-object" "$object")
done

IGNORE_REGEX='(\.cargo/registry|\.rustup/toolchains|/rustc/|/tests/|src/main\.rs)'
REPORT="$COVERAGE_DIR/report.txt"

llvm-cov report \
  "$MAIN_OBJECT" \
  -instr-profile="$COVERAGE_DIR/coverage.profdata" \
  "${EXTRA_OBJECT_ARGS[@]}" \
  --ignore-filename-regex="$IGNORE_REGEX" \
  | tee "$REPORT"

LINE_COVERAGE="$(awk '/^TOTAL/ {gsub("%", "", $10); print $10}' "$REPORT")"

if ! awk -v coverage="$LINE_COVERAGE" -v threshold="$THRESHOLD" 'BEGIN { exit !(coverage + 0 >= threshold + 0) }'; then
  echo "Line coverage ${LINE_COVERAGE}% is below required ${THRESHOLD}%." >&2
  exit 1
fi

echo "Line coverage ${LINE_COVERAGE}% meets required ${THRESHOLD}%."
