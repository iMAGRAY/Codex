#!/usr/bin/env bash
# Chaos validation loop for Stellar resilience services.
# Usage: ./resilience_loop.sh [duration_seconds] [sleep_seconds]
# Defaults: duration=600s (10 minutes), sleep=15s between iterations.
set -euo pipefail

DURATION="${1:-600}"
SLEEP_INTERVAL="${2:-15}"
LOG_DIR="${CHAOS_LOG_DIR:-target/chaos}"
mkdir -p "$LOG_DIR"
LOG_FILE="${LOG_DIR}/resilience_chaos_$(date +%Y%m%dT%H%M%S).log"

START_EPOCH=$(date +%s)
END_EPOCH=$((START_EPOCH + DURATION))
ITERATION=0
PASSES=0
FAILS=0

printf "Starting resilience chaos loop (duration=%ss, interval=%ss)\n" "$DURATION" "$SLEEP_INTERVAL" | tee -a "$LOG_FILE"
while :; do
    NOW=$(date +%s)
    [ "$NOW" -ge "$END_EPOCH" ] && break
    ITERATION=$((ITERATION + 1))
    TIMESTAMP=$(date --iso-8601=seconds)
    printf "[%s] Iteration %d: running chaos tests...\n" "$TIMESTAMP" "$ITERATION" | tee -a "$LOG_FILE"
    if cargo test -p codex-core --test resilience_chaos -- --nocapture >>"$LOG_FILE" 2>&1; then
        PASSES=$((PASSES + 1))
        printf "[%s] Iteration %d: PASS\n" "$TIMESTAMP" "$ITERATION" | tee -a "$LOG_FILE"
    else
        FAILS=$((FAILS + 1))
        printf "[%s] Iteration %d: FAIL (see log)\n" "$TIMESTAMP" "$ITERATION" | tee -a "$LOG_FILE"
    fi
    NOW=$(date +%s)
    REMAIN=$((END_EPOCH - NOW))
    [ "$REMAIN" -le 0 ] && break
    sleep "$SLEEP_INTERVAL"
done
TOTAL_TIME=$(( $(date +%s) - START_EPOCH ))
printf "Chaos loop complete: %d passes, %d failures, total runtime %ss.\nLog file: %s\n" "$PASSES" "$FAILS" "$TOTAL_TIME" "$LOG_FILE" | tee -a "$LOG_FILE"
if [ "$FAILS" -gt 0 ]; then
    exit 1
fi
