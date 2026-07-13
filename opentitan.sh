#!/usr/bin/env bash

set -Eeuo pipefail

########################################
# Configuration
########################################

MARIA_DIR="$HOME/maria"
OPENTITAN_DIR="/home/whale-d/maria/opentitan"

BIN="$MARIA_DIR/target/debug/maria"

LOG_DIR="$MARIA_DIR/logs"
PASS_DIR="$LOG_DIR/pass"
FAIL_DIR="$LOG_DIR/fail"

TIMEOUT=120

########################################

mkdir -p "$PASS_DIR"
mkdir -p "$FAIL_DIR"

echo "======================================="
echo "Building Maria..."
echo "======================================="

cd "$MARIA_DIR"

cargo build

if [[ ! -f "$BIN" ]]; then
    echo "Maria binary not found!"
    exit 1
fi

echo
echo "Searching SystemVerilog files..."

mapfile -t FILES < <(
find "$OPENTITAN_DIR" \
    -type f \
    \( -name "*.sv" -o -name "*.svh" \) \
| sort
)

TOTAL=${#FILES[@]}

echo "Found $TOTAL files."
echo

COUNT=1

for FILE in "${FILES[@]}"
do

    NAME=$(basename "$FILE")
    SAFE_NAME=$(echo "$FILE" | sed 's/\//_/g')

    echo
    echo "======================================="
    echo "[$COUNT/$TOTAL]"
    echo "$FILE"
    echo "======================================="

    START=$(date +%s)

    if timeout "${TIMEOUT}s" \
        "$BIN" "$FILE" \
        > "$PASS_DIR/${SAFE_NAME}.log" \
        2>&1
    then

        END=$(date +%s)
        DUR=$((END-START))

        echo "[PASS] ${DUR}s"

    else

        RET=$?

        END=$(date +%s)
        DUR=$((END-START))

        mv "$PASS_DIR/${SAFE_NAME}.log" \
           "$FAIL_DIR/${SAFE_NAME}.log" \
           2>/dev/null || true

        echo "[FAIL] exit=$RET time=${DUR}s"

    fi

    COUNT=$((COUNT+1))

done

echo
echo "======================================="
echo "Finished."
echo "======================================="