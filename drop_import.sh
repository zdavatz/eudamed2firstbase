#!/bin/bash
# Import hand-downloaded EUDAMED records from the drop folder (20.11.2026).
# Polled from cron every 10 minutes.
#
#   ~/eudamed2firstbase/drop/detail/<uuid>.json   (udiDiData)
#   ~/eudamed2firstbase/drop/basic/<uuid>.json    (basicUdiData)
#   ~/eudamed2firstbase/drop/url_down/<SRN>_page<N>.json  (listing pages)
#
# A file is taken once it has not changed for 2 minutes (download finished).
# Records (non-empty, JSON object) move to import/<date>_drop_detail|basic/,
# anything else to import/<date>_drop_rejected/. A device whose detail AND
# Basic UDI-DI are both imported is copied into eudamed_json/detail|basic, at
# most 1'500 devices per day (GS1 limit) — the nightly `check` pushes them
# (only devices in listing_cache). Released UUIDs: log/drop_released.txt.
# Log: log/drop_import.log
DATA="${DATA:-$HOME/eudamed2firstbase}"
DROP="$DATA/drop"
REPO="${REPO:-$(cd "$(dirname "$0")" && pwd)}"
MAX_PER_DAY="${MAX_PER_DAY:-1500}"
DAY=$(date +%F)
LOG="$DATA/log/drop_import.log"
RELEASED="$DATA/log/drop_released.txt"
mkdir -p "$DROP/detail" "$DROP/basic" "$DROP/url_down" "$DATA/log"
exec 9>"$DATA/log/drop_import.lock"
flock -n 9 || exit 0
log() { echo "$(date '+%F %T') $*" >> "$LOG"; }

# 1. Move finished files from the drop folder into import/.
for kind in detail basic; do
  moved=0; bad=0
  while IFS= read -r -d '' f; do
    name=$(basename "$f")
    if [ -s "$f" ] && [ "$(head -c 64 "$f" | tr -d ' \t\r\n' | head -c 1)" = "{" ]; then
      mkdir -p "$DATA/import/${DAY}_drop_$kind"
      mv -f "$f" "$DATA/import/${DAY}_drop_$kind/$name"; moved=$((moved+1))
    else
      mkdir -p "$DATA/import/${DAY}_drop_rejected/$kind"
      mv -f "$f" "$DATA/import/${DAY}_drop_rejected/$kind/$name"; bad=$((bad+1))
    fi
  done < <(find "$DROP/$kind" -maxdepth 1 -type f -name '*.json' -mmin +2 -print0)
  [ $((moved+bad)) -gt 0 ] && log "$kind: $moved imported, $bad rejected (not a JSON record)"
done

# 1b. Listing pages (<SRN>_page<N>.json) from drop/url_down: move them to
# import/<date>_drop_listing/ and compare. Missing pages -> log/handoff/urls.txt;
# listing complete -> new devices into listing_cache, log/handoff/uuids.txt.
lm=0
while IFS= read -r -d '' f; do
  mkdir -p "$DATA/import/${DAY}_drop_listing"
  mv -f "$f" "$DATA/import/${DAY}_drop_listing/"; lm=$((lm+1))
done < <(find "$DROP/url_down" -maxdepth 1 -type f -name '*_page*.json' -mmin +2 -print0)
if [ "$lm" -gt 0 ]; then
  log "listing: $lm page(s) moved to import/${DAY}_drop_listing"
  "$REPO/target/debug/eudamed2firstbase" handoff "$REPO/srns_sheet.txt" \
    --collect "$DATA/import/${DAY}_drop_listing" >> "$LOG" 2>&1 \
    || log "listing compare failed (rc=$?)"
fi

# 2. Release complete devices (detail + Basic UDI-DI) into eudamed_json/.
touch "$RELEASED"
today=$(grep -c "	$DAY\$" "$RELEASED")
room=$((MAX_PER_DAY - today))
[ "$room" -le 0 ] && exit 0
tmp=$(mktemp -d)
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
    && printf '%s\t%s\n' "$u" "$DAY" >> "$RELEASED" && n=$((n+1))
done < "$tmp/ready"
left=$(( $(wc -l < "$tmp/ready") - n ))
[ "$n" -gt 0 ] && log "released $n device(s) to eudamed_json/ for the nightly push ($left waiting for the next day)"
rm -rf "$tmp"
