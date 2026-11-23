//! `daily-report [--days N]` (23.11.2026): what changed per day, read-only.
//!
//! Operator: the focus is how many new devices are found every day from the
//! hospital GTIN list, and how many devices got updated. Per local day:
//!
//! - GTIN list (`handoff_gtin`, the GTIN hand-off): GTIN lookups answered
//!   "found" (and how many of them had no match before, `handoff_gtin_event`),
//!   list devices new in GS1 Production, list devices delivered again.
//! - All devices: new devices in the hand-downloaded listings
//!   (`log/unknown_uuids_<day>_*.txt`), EUDAMED changes in those listings
//!   (`log/listing_changes_<day>_*.tsv`: version bumps / status changes),
//!   devices new in GS1 Production, devices delivered again (already accepted
//!   before: an EUDAMED update or a correction), devices whose last push of the
//!   day was rejected.
use anyhow::Result;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

#[derive(Default, Clone)]
struct Day {
    glist_found: usize,
    glist_found_late: usize,
    glist_new_gs1: usize,
    glist_again_gs1: usize,
    listed_new: usize,
    listed_changes: usize,
    new_gs1: usize,
    again_gs1: usize,
    rejected: usize,
    total_gtins: usize,
    total_glist: usize,
}

fn norm(g: &str) -> String {
    format!("{:0>14}", g.trim())
}

/// "2026-11-23T00:45:56Z" (UTC) or "2026-11-23 01:45:56" (local) -> local day.
fn local_day(ts: &str) -> Option<String> {
    if ts.ends_with('Z') {
        let t = chrono::DateTime::parse_from_rfc3339(ts).ok()?;
        Some(
            t.with_timezone(&chrono::Local)
                .format("%Y-%m-%d")
                .to_string(),
        )
    } else {
        ts.get(..10).map(|s| s.to_string())
    }
}

fn count_lines(dir: &Path, prefix: &str, suffix: &str, header: bool) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    rd.flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.starts_with(prefix) && n.ends_with(suffix)
        })
        .map(|e| {
            let n = std::fs::read_to_string(e.path())
                .map(|b| b.lines().filter(|l| !l.trim().is_empty()).count())
                .unwrap_or(0);
            if header {
                n.saturating_sub(1)
            } else {
                n
            }
        })
        .sum()
}

pub fn run(data: &Path, days: i64) -> Result<String> {
    let con = rusqlite::Connection::open_with_flags(
        data.join("db").join("version_tracking.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )?;
    con.busy_timeout(std::time::Duration::from_secs(60))?;
    let today = chrono::Local::now().date_naive();
    let day_list: Vec<String> = (0..days)
        .rev()
        .map(|i| {
            (today - chrono::Duration::days(i))
                .format("%Y-%m-%d")
                .to_string()
        })
        .collect();
    let mut rep: BTreeMap<String, Day> = day_list
        .iter()
        .map(|d| (d.clone(), Day::default()))
        .collect();

    let has = |t: &str| -> bool {
        con.query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [t],
            |_| Ok(()),
        )
        .is_ok()
    };

    // GTIN list and its devices.
    let mut glist: HashSet<String> = HashSet::new();
    if has("handoff_gtin") {
        let mut st = con.prepare("SELECT gtin, status, answered_at FROM handoff_gtin")?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })?;
        for (g, s, at) in rows.flatten() {
            glist.insert(norm(&g));
            if s == "found" {
                if let Some(d) = at.as_deref().and_then(local_day) {
                    if let Some(e) = rep.get_mut(&d) {
                        e.glist_found += 1;
                    }
                }
            }
        }
    }
    if has("handoff_gtin_event") {
        let mut st = con.prepare(
            "SELECT at FROM handoff_gtin_event WHERE from_status = 'not_found' AND to_status = 'found'",
        )?;
        for at in st.query_map([], |r| r.get::<_, String>(0))?.flatten() {
            if let Some(e) = local_day(&at).and_then(|d| rep.get_mut(&d)) {
                e.glist_found_late += 1;
            }
        }
    }
    let mut glist_uuids: HashSet<String> = HashSet::new();
    let mut glist_gtin_uuids: HashMap<String, Vec<String>> = HashMap::new();
    {
        let mut st =
            con.prepare("SELECT uuid, primary_di FROM listing_cache WHERE primary_di <> ''")?;
        for (u, g) in st
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .flatten()
        {
            if glist.contains(&norm(&g)) {
                glist_uuids.insert(u.clone());
                glist_gtin_uuids.entry(norm(&g)).or_default().push(u);
            }
        }
    }

    // GS1 Production pushes: first ACCEPTED = new, a later ACCEPTED = again.
    let mut first_acc: HashMap<String, String> = HashMap::new();
    // GTIN -> day GS1 first accepted it (the totals per day).
    let mut first_gtin: HashMap<String, String> = HashMap::new();
    let mut new_by_day: HashMap<String, HashSet<String>> = HashMap::new();
    let mut again_by_day: HashMap<String, HashSet<String>> = HashMap::new();
    let mut last_of_day: HashMap<(String, String), String> = HashMap::new();
    {
        let mut st = con.prepare(
            "SELECT uuid, gtin, pushed_at, status FROM push_log
             WHERE firstbase_env = 'Production' ORDER BY id",
        )?;
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
            ))
        })?;
        for (u, g, at, s) in rows.flatten() {
            let Some(d) = local_day(&at) else { continue };
            if g.as_deref()
                .map(|g| glist.contains(&norm(g)))
                .unwrap_or(false)
            {
                glist_uuids.insert(u.clone());
            }
            last_of_day.insert((d.clone(), u.clone()), s.clone());
            if s != "ACCEPTED" {
                continue;
            }
            if let Some(g) = g.as_deref().filter(|g| !g.trim().is_empty()) {
                first_gtin.entry(norm(g)).or_insert_with(|| d.clone());
            }
            match first_acc.get(&u) {
                None => {
                    first_acc.insert(u.clone(), d.clone());
                    new_by_day.entry(d).or_default().insert(u);
                }
                Some(f) if *f != d => {
                    again_by_day.entry(d).or_default().insert(u);
                }
                _ => {}
            }
        }
    }
    for (d, e) in rep.iter_mut() {
        if let Some(s) = new_by_day.get(d) {
            e.new_gs1 = s.len();
            e.glist_new_gs1 = s.iter().filter(|u| glist_uuids.contains(*u)).count();
        }
        if let Some(s) = again_by_day.get(d) {
            e.again_gs1 = s.len();
            e.glist_again_gs1 = s.iter().filter(|u| glist_uuids.contains(*u)).count();
        }
        e.rejected = last_of_day
            .iter()
            .filter(|((dd, _), s)| dd == d && s.as_str() == "REJECTED")
            .count();
        e.total_gtins = first_gtin
            .values()
            .filter(|f| f.as_str() <= d.as_str())
            .count();
        e.total_glist = glist
            .iter()
            .filter(|g| {
                first_gtin
                    .get(*g)
                    .map(|f| f.as_str() <= d.as_str())
                    .unwrap_or(false)
                    || glist_gtin_uuids.get(*g).map_or(false, |us| {
                        us.iter()
                            .any(|u| first_acc.get(u).map_or(false, |f| f.as_str() <= d.as_str()))
                    })
            })
            .count();
        let log = data.join("log");
        e.listed_new = count_lines(&log, &format!("unknown_uuids_{d}"), ".txt", false);
        e.listed_changes = count_lines(&log, &format!("listing_changes_{d}"), ".tsv", true);
    }

    let mut out = String::new();
    out.push_str(&format!("GTIN list: {} GTINs\n\n", glist.len()));
    out.push_str("day         | list found  (no match before) | list new in GS1 | list again in GS1 || listing: new devices | listing: EUDAMED changes || GS1 new | GS1 again | rejected || GTINs in GS1 (total) | list GTINs in GS1 (total)\n");
    for (d, e) in &rep {
        out.push_str(&format!(
            "{d}  | {:>10}  {:>21} | {:>15} | {:>17} || {:>20} | {:>24} || {:>7} | {:>9} | {:>8} || {:>20} | {:>25}\n",
            e.glist_found,
            e.glist_found_late,
            e.glist_new_gs1,
            e.glist_again_gs1,
            e.listed_new,
            e.listed_changes,
            e.new_gs1,
            e.again_gs1,
            e.rejected,
            e.total_gtins,
            format!("{} ({})", e.total_glist, glist.len())
        ));
    }
    out.push_str(
        "\n\"again\" = a device GS1 had already accepted earlier (EUDAMED update or correction).\n",
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    #[test]
    fn days_and_gtins() {
        assert_eq!(super::norm("7612345000435"), "07612345000435");
        assert_eq!(
            super::local_day("2026-11-23 01:45:56").as_deref(),
            Some("2026-11-23")
        );
        assert!(super::local_day("2026-11-23T00:45:56Z").is_some());
    }
}
