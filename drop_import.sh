#!/bin/bash
# Import hand-downloaded EUDAMED records from the drop folder (20.11.2026).
# Polled from cron every 10 minutes.
#
#   ~/eudamed2firstbase/drop/detail/<uuid>.json   (udiDiData)
#   ~/eudamed2firstbase/drop/basic/<uuid>.json    (basicUdiData)
#   ~/eudamed2firstbase/drop/url_down/<SRN>_page<N>.json  (listing pages)
#   ~/eudamed2firstbase/drop/gtin/<GTIN>.json             (GTIN lookups, gtin_urls.txt)
#
# A file is taken once it has not changed for 2 minutes (download finished).
# Records (non-empty, JSON object) move to import/<date>_drop_detail|basic/,
# anything else to import/<date>_drop_rejected/. A device whose detail AND
# Basic UDI-DI are both imported is copied into eudamed_json/detail|basic and
# pushed to GS1 Production right away (repush-srn, devices in listing_cache
# only; never next to the nightly or another push). Released UUIDs:
# log/drop_released.txt; queue: log/drop_push_pending.txt.
# Log: log/drop_import.log
DATA="${DATA:-$HOME/eudamed2firstbase}"
DROP="$DATA/drop"
REPO="${REPO:-$(cd "$(dirname "$0")" && pwd)}"
DAY=$(date +%F)
LOG="$DATA/log/drop_import.log"
RELEASED="$DATA/log/drop_released.txt"
mkdir -p "$DROP/detail" "$DROP/basic" "$DROP/url_down" "$DROP/gtin" "$DATA/log"
exec 9>"$DATA/log/drop_import.lock"
flock -n 9 || exit 0
log() { echo "$(date '+%F %T') $*" >> "$LOG"; }
# Every file taken from the drop folder is recorded in the DB (drop_received,
# 21.11.2026): "<kind>\t<outcome>\t<path>" lines, stored after step 1c.
RECV=$(mktemp)

# 1. Move finished files from the drop folder into import/.
for kind in detail basic; do
  moved=0; bad=0
  while IFS= read -r -d '' f; do
    name=$(basename "$f")
    if [ -s "$f" ] && [ "$(head -c 64 "$f" | tr -d ' \t\r\n' | head -c 1)" = "{" ]; then
      mkdir -p "$DATA/import/${DAY}_drop_$kind"
      mv -f "$f" "$DATA/import/${DAY}_drop_$kind/$name"; moved=$((moved+1))
      printf '%s\timported\t%s\n' "$kind" "$DATA/import/${DAY}_drop_$kind/$name" >> "$RECV"
    else
      mkdir -p "$DATA/import/${DAY}_drop_rejected/$kind"
      mv -f "$f" "$DATA/import/${DAY}_drop_rejected/$kind/$name"; bad=$((bad+1))
      printf '%s\trejected\t%s\n' "$kind" "$DATA/import/${DAY}_drop_rejected/$kind/$name" >> "$RECV"
    fi
  done < <(find "$DROP/$kind" -maxdepth 1 -type f -name '*.json' -mmin +2 -print0)
  [ $((moved+bad)) -gt 0 ] && log "$kind: $moved imported, $bad rejected (not a JSON record)"
done

# Records imported -> uuids.txt lists only what is still missing (21.11.2026:
# it stayed at 374 after the 374 details came in, until the nightly).
if grep -qE '^(detail|basic)	' "$RECV" 2>/dev/null; then
  "$REPO/target/debug/eudamed2firstbase" handoff "$REPO/srns_sheet.txt" --uuids >> "$LOG" 2>&1 \
    || log "uuids.txt not rewritten (rc=$?)"
fi

# 1b. Listing pages (<SRN>_page<N>.json) from drop/url_down: move them to
# import/<date>_drop_listing/ and compare. Missing pages -> log/handoff/urls.txt;
# listing complete -> new devices into listing_cache, log/handoff/uuids.txt.
lm=0
while IFS= read -r -d '' f; do
  mkdir -p "$DATA/import/${DAY}_drop_listing"
  mv -f "$f" "$DATA/import/${DAY}_drop_listing/"; lm=$((lm+1))
  printf 'listing\timported\t%s\n' "$DATA/import/${DAY}_drop_listing/$(basename "$f")" >> "$RECV"
done < <(find "$DROP/url_down" -maxdepth 1 -type f -name '*_page*.json' -mmin +2 -print0)
if [ "$lm" -gt 0 ]; then
  log "listing: $lm page(s) moved to import/${DAY}_drop_listing"
  "$REPO/target/debug/eudamed2firstbase" handoff "$REPO/srns_sheet.txt" \
    --collect "$DATA/import/${DAY}_drop_listing" >> "$LOG" 2>&1 \
    || log "listing compare failed (rc=$?)"
fi

# 1c. GTIN lookups (<GTIN>.json, answers to log/handoff/gtin_urls.txt) from
# drop/gtin: move them to import/<date>_drop_gtin/ and read them. Found devices
# -> listing_cache + log/handoff/uuids.txt; answered GTINs leave gtin_urls.txt.
gm=0
while IFS= read -r -d '' f; do
  mkdir -p "$DATA/import/${DAY}_drop_gtin"
  mv -f "$f" "$DATA/import/${DAY}_drop_gtin/"; gm=$((gm+1))
  printf 'gtin\timported\t%s\n' "$DATA/import/${DAY}_drop_gtin/$(basename "$f")" >> "$RECV"
done < <(find "$DROP/gtin" -maxdepth 1 -type f -name '*.json' -mmin +2 -print0)
if [ "$gm" -gt 0 ]; then
  log "gtin: $gm answer(s) moved to import/${DAY}_drop_gtin"
  "$REPO/target/debug/eudamed2firstbase" handoff "$REPO/srns_sheet.txt" \
    --collect-gtins "$DATA/import/${DAY}_drop_gtin" >> "$LOG" 2>&1 \
    || log "gtin collect failed (rc=$?)"
fi

if [ -s "$RECV" ]; then
  if "$REPO/target/debug/eudamed2firstbase" handoff --received "$RECV" >> "$LOG" 2>&1; then
    rm -f "$RECV"
  else
    log "drop_received not recorded — list kept in $RECV"
  fi
else
  rm -f "$RECV"
fi

# 2. Release complete devices (detail + Basic UDI-DI) into eudamed_json/ and
# queue them for the push (log/drop_push_pending.txt). MAX_PER_DAY (default:
# no limit) caps the releases per day.
PENDING="$DATA/log/drop_push_pending.txt"
touch "$RELEASED" "$PENDING"
today=$(grep -c "	$DAY\$" "$RELEASED")
room=$(( ${MAX_PER_DAY:-0} > 0 ? MAX_PER_DAY - today : 1000000000 ))
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
if [ "$room" -gt 0 ]; then
  for kind in detail basic; do
    find "$DATA"/import/*_drop_"$kind" -maxdepth 1 -type f -name '*.json' -printf '%f\n' 2>/dev/null \
      | sed 's/\.json$//' | sort -u > "$tmp/$kind"
  done
  cut -f1 "$RELEASED" | sort -u > "$tmp/done"
  comm -12 "$tmp/detail" "$tmp/basic" | comm -23 - "$tmp/done" > "$tmp/ready"
  n=0
  while read -r u; do
    [ "$n" -ge "$room" ] && break
    d=$(ls -t "$DATA"/import/*_drop_detail/"$u.json" | head -1)
    b=$(ls -t "$DATA"/import/*_drop_basic/"$u.json" | head -1)
    cp -f "$d" "$DATA/eudamed_json/detail/$u.json" && cp -f "$b" "$DATA/eudamed_json/basic/$u.json" \
      && printf '%s\t%s\n' "$u" "$DAY" >> "$RELEASED" && echo "$u" >> "$PENDING" && n=$((n+1))
  done < "$tmp/ready"
  left=$(( $(wc -l < "$tmp/ready") - n ))
  [ "$n" -gt 0 ] && log "released $n device(s) to eudamed_json/ ($left waiting for the next day)"
fi

# 3. Push the queued devices to GS1 Production right away (repush-srn sends the
# GS1 report). Guard: never next to the nightly or another push — then the
# queue waits for the next poll. The push holds the nightly lock, so a nightly
# starting meanwhile is skipped (and caught up by the next night).
[ -s "$PENDING" ] || exit 0
# Release the poll lock before pushing (21.11.2026): a push takes hours, and
# holding the lock meanwhile blocked every import (1'929 GTIN answers waited
# during the 20.11. push). Two pushes at once are still prevented by the
# nightly lock below and the pgrep guard.
exec 9>&-
exec 8>"$DATA/log/nightly_check.lock"
if ! flock -n 8 || pgrep -f "eudamed2firstbase (check|deep-scan|repush-srn) " >/dev/null; then
  log "push: $(sort -u "$PENDING" | wc -l) device(s) queued, nightly or another push running — retry next poll"
  exit 0
fi
sort -u "$PENDING" > "$tmp/push_uuids"
# Which queued UUIDs are in listing_cache, and their SRNs: asked in Rust
# (handoff --known) — thousands of UUIDs in one sqlite3 argument fail with
# "Argument list too long" (20.11.2026, 6'752).
"$REPO/target/debug/eudamed2firstbase" handoff --known "$tmp/push_uuids" > "$tmp/known" \
  || { log "push: listing_cache lookup failed (rc=$?) — queue kept"; exit 0; }
cut -f2 "$tmp/known" | sort -u > "$tmp/push_srns"
known=$(wc -l < "$tmp/known")
total=$(wc -l < "$tmp/push_uuids")
if [ ! -s "$tmp/push_srns" ]; then
  log "push: none of the $total queued device(s) is in listing_cache — compare the listing first (queue kept)"
  exit 0
fi
[ "$known" -lt "$total" ] && log "push: $((total - known)) of $total queued device(s) not in listing_cache — they stay queued"
[ -f "$HOME/.fb_prod_env" ] && { set -a; . "$HOME/.fb_prod_env"; set +a; }
export FIRSTBASE_ENV=Production
PLOG="$DATA/log/drop_push_$(date +%F_%H%M%S).log"
log "push: $known device(s) of $(wc -l < "$tmp/push_srns") SRN(s) -> GS1 Production, log $PLOG"
( cd "$REPO" && ./target/debug/eudamed2firstbase repush-srn --reconvert \
    --file "$tmp/push_srns" --uuid-file "$tmp/push_uuids" ) > "$PLOG" 2>&1
rc=$?
log "push: rc=$rc — $(grep -E '^Done:' "$PLOG" | tail -1)"
# repush-srn exits 0 even when the push never reached GS1 (21.11.2026: token
# 503, queue of 80 emptied) — only a "Done:" line means the batches went out.
if [ "$rc" -eq 0 ] && grep -q '^Done:' "$PLOG"; then
  # Drop the pushed devices from the queue; keep the ones not in listing_cache
  # yet and those a poll queued while this push ran (the lock is released).
  cut -f1 "$tmp/known" | sort -u > "$tmp/pushed"
  sort -u "$PENDING" | comm -23 - "$tmp/pushed" > "$tmp/left" && cat "$tmp/left" > "$PENDING"
else
  log "push: did not reach GS1 — $(sort -u "$PENDING" | wc -l) device(s) stay queued"
fi
