//! `listing-compare <dir>`: compare public listing pages that the operator
//! downloaded by hand (`<SRN>_page<N>.json`, size=300) with `listing_cache`.
//!
//! Since 05.11.2026 the public listing answers every automated client with 429
//! and the datalake `/udi` has been down since 13.11.2026, so the only way to
//! see new devices, version bumps and status changes is a browser download of
//! the listing URLs. This command checks the pages, lists the pages still
//! missing (an SRN that grew since the URL list was made), and writes the
//! unknown UUIDs one per line for the detail / Basic UDI-DI download.
//! Read-only: it writes nothing to the database.

use anyhow::{Context, Result};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

pub const PAGE_SIZE: u64 = 300;

/// One device row of a listing page (only the fields we compare).
#[derive(Debug, Clone, PartialEq)]
pub struct ListedDevice {
    pub uuid: String,
    pub srn: String,
    pub primary_di: String,
    pub status: String,
    pub version: Option<i64>,
}

/// One SRN's downloaded pages.
#[derive(Debug, Default)]
pub struct SrnPages {
    pub total: Option<u64>,
    pub pages: BTreeSet<u64>,
    pub devices: Vec<ListedDevice>,
}

/// What `listing_cache` / `udi_versions` know about a device.
#[derive(Debug, Clone)]
pub struct KnownDevice {
    pub srn: String,
    pub primary_di: String,
    pub status: String,
    pub version: Option<i64>,
}

#[derive(Debug, Default)]
pub struct Comparison {
    pub new: Vec<ListedDevice>,
    pub new_known_gtin: usize,
    /// (device, known version)
    pub version_bumps: Vec<(ListedDevice, i64)>,
    /// (device, known status)
    pub status_changes: Vec<(ListedDevice, String)>,
    /// Known devices of a complete SRN that are not listed any more.
    pub no_longer_listed: Vec<(String, KnownDevice)>,
    /// (srn, page) that totalElements requires but that is not in the dir.
    pub missing_pages: Vec<(String, u64)>,
    pub listed: usize,
    pub total: u64,
}

fn code_of(v: &serde_json::Value) -> String {
    v.get("code")
        .and_then(|c| c.as_str())
        .or_else(|| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// Parse one page body. Errors on anything that is not a listing page.
pub fn parse_page(srn: &str, body: &str) -> Result<(Option<u64>, Vec<ListedDevice>)> {
    let v: serde_json::Value = serde_json::from_str(body).context("not JSON")?;
    let content = v
        .get("content")
        .and_then(|c| c.as_array())
        .context("no content array")?;
    let total = v.get("totalElements").and_then(|t| t.as_u64());
    let mut out = Vec::with_capacity(content.len());
    for c in content {
        let Some(uuid) = c.get("uuid").and_then(|u| u.as_str()) else {
            continue;
        };
        let version = c.get("versionNumber").and_then(|n| {
            n.as_i64()
                .or_else(|| n.as_str().and_then(|s| s.parse().ok()))
        });
        out.push(ListedDevice {
            uuid: uuid.to_string(),
            srn: srn.to_string(),
            primary_di: c
                .get("primaryDi")
                .and_then(|p| p.as_str())
                .unwrap_or("")
                .to_string(),
            status: c.get("deviceStatusType").map(code_of).unwrap_or_default(),
            version,
        });
    }
    Ok((total, out))
}

/// `<SRN>_page<N>.json` → (SRN, N)
pub fn parse_file_name(name: &str) -> Option<(String, u64)> {
    let stem = name.strip_suffix(".json")?;
    let (srn, page) = stem.rsplit_once("_page")?;
    Some((srn.to_string(), page.parse().ok()?))
}

/// Read every page in `dir`. Returns the pages per SRN and the unusable files.
pub fn read_dir(dir: &Path) -> Result<(BTreeMap<String, SrnPages>, Vec<String>)> {
    let mut srns: BTreeMap<String, SrnPages> = BTreeMap::new();
    let mut bad = Vec::new();
    for e in std::fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let path = e?.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let Some((srn, page)) = parse_file_name(&name) else {
            continue;
        };
        let body = std::fs::read_to_string(&path).unwrap_or_default();
        match parse_page(&srn, &body) {
            Ok((total, devices)) => {
                let p = srns.entry(srn).or_default();
                if p.total.is_none() || page == 0 {
                    p.total = total.or(p.total);
                }
                p.pages.insert(page);
                p.devices.extend(devices);
            }
            Err(_) => bad.push(name),
        }
    }
    bad.sort();
    Ok((srns, bad))
}

/// Compare the pages with the known devices. An SRN with missing pages gets no
/// "no longer listed" entries (its absent devices may sit on a missing page).
pub fn compare(
    srns: &BTreeMap<String, SrnPages>,
    known: &HashMap<String, KnownDevice>,
) -> Comparison {
    let mut c = Comparison::default();
    let known_gtins: BTreeSet<&str> = known.values().map(|k| k.primary_di.as_str()).collect();
    let mut listed: BTreeSet<&str> = BTreeSet::new();
    let mut complete: BTreeSet<&str> = BTreeSet::new();
    for (srn, p) in srns {
        let total = p.total.unwrap_or(0);
        c.total += total;
        let needed = total.div_ceil(PAGE_SIZE);
        let missing: Vec<u64> = (0..needed).filter(|n| !p.pages.contains(n)).collect();
        if missing.is_empty() {
            complete.insert(srn.as_str());
        }
        c.missing_pages
            .extend(missing.into_iter().map(|n| (srn.clone(), n)));
        for d in &p.devices {
            if !listed.insert(d.uuid.as_str()) {
                continue;
            }
            match known.get(&d.uuid) {
                None => {
                    if !d.primary_di.is_empty() && known_gtins.contains(d.primary_di.as_str()) {
                        c.new_known_gtin += 1;
                    }
                    c.new.push(d.clone());
                }
                Some(k) => {
                    if let (Some(new), Some(old)) = (d.version, k.version) {
                        if new > old {
                            c.version_bumps.push((d.clone(), old));
                        }
                    }
                    if !k.status.is_empty() && !d.status.is_empty() && k.status != d.status {
                        c.status_changes.push((d.clone(), k.status.clone()));
                    }
                }
            }
        }
    }
    c.listed = listed.len();
    let mut gone: Vec<(String, KnownDevice)> = known
        .iter()
        .filter(|(u, k)| complete.contains(k.srn.as_str()) && !listed.contains(u.as_str()))
        .map(|(u, k)| (u.clone(), k.clone()))
        .collect();
    gone.sort_by(|a, b| (&a.1.srn, &a.0).cmp(&(&b.1.srn, &b.0)));
    c.no_longer_listed = gone;
    c
}

/// Known devices from `listing_cache`, with the detail version from
/// `udi_versions` where the device was indexed (the version GS1 has).
pub fn load_known(db: &Path) -> Result<HashMap<String, KnownDevice>> {
    let conn = rusqlite::Connection::open_with_flags(
        db,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .with_context(|| format!("open {}", db.display()))?;
    let mut udi: HashMap<String, i64> = HashMap::new();
    if let Ok(mut st) =
        conn.prepare("SELECT uuid, udi_version FROM udi_versions WHERE udi_version IS NOT NULL")
    {
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))?;
        for (u, v) in rows.flatten() {
            udi.insert(u, v);
        }
    }
    let mut st = conn.prepare(
        "SELECT uuid, srn, primary_di, device_status, version_number FROM listing_cache",
    )?;
    let rows = st.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            KnownDevice {
                srn: r.get(1)?,
                primary_di: r.get(2)?,
                status: r.get(3)?,
                version: r.get(4)?,
            },
        ))
    })?;
    let mut out = HashMap::new();
    for (u, mut k) in rows.flatten() {
        if let Some(v) = udi.get(&u) {
            k.version = Some(*v);
        }
        out.insert(u, k);
    }
    Ok(out)
}

pub fn listing_url(srn: &str, page: u64) -> String {
    format!(
        "https://ec.europa.eu/tools/eudamed/api/devices/udiDiData?page={page}&size={PAGE_SIZE}&srn={srn}&iso2Code=en&languageIso2Code=en"
    )
}

fn short(code: &str) -> &str {
    code.rsplit('.').next().unwrap_or(code)
}

/// Run the command: print the summary and write the files to `out_dir`
/// (`unknown_uuids_<tag>.txt/.tsv`, `listing_urls_<tag>_missing.txt`,
/// `listing_changes_<tag>.tsv`, `no_longer_listed_<tag>.tsv`).
pub fn run(dir: &Path, db: &Path, out_dir: &Path, tag: &str) -> Result<Comparison> {
    let (srns, bad) = read_dir(dir)?;
    let known = load_known(db)?;
    let c = compare(&srns, &known);
    let pages: usize = srns.values().map(|p| p.pages.len()).sum();
    println!(
        "listing-compare: {} page(s) of {} SRN(s) in {}, {} unusable",
        pages,
        srns.len(),
        dir.display(),
        bad.len()
    );
    for b in &bad {
        println!("  unusable: {b}");
    }
    println!(
        "devices: {} listed of {} (totalElements), {} known in listing_cache",
        c.listed,
        c.total,
        known.len()
    );
    let mut by_srn: BTreeMap<&str, usize> = BTreeMap::new();
    for (s, _) in &c.missing_pages {
        *by_srn.entry(s.as_str()).or_default() += 1;
    }
    println!(
        "missing pages: {}{}",
        c.missing_pages.len(),
        if by_srn.is_empty() {
            String::new()
        } else {
            format!(
                " ({})",
                by_srn
                    .iter()
                    .map(|(s, n)| format!("{s} {n}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        }
    );
    let mut st: BTreeMap<&str, usize> = BTreeMap::new();
    let mut ns: BTreeMap<&str, usize> = BTreeMap::new();
    for d in &c.new {
        *st.entry(short(&d.status)).or_default() += 1;
        *ns.entry(d.srn.as_str()).or_default() += 1;
    }
    let mut top: Vec<(&str, usize)> = ns.into_iter().collect();
    top.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    println!(
        "new: {} ({} with a GTIN known under another UUID; {})",
        c.new.len(),
        c.new_known_gtin,
        st.iter()
            .map(|(s, n)| format!("{s} {n}"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    for (s, n) in top.iter().take(10) {
        println!("  {s}: {n}");
    }
    println!("version bumps: {}", c.version_bumps.len());
    println!("status changes: {}", c.status_changes.len());
    println!(
        "no longer listed (complete SRNs only): {}",
        c.no_longer_listed.len()
    );

    std::fs::create_dir_all(out_dir)?;
    let w = |name: String, body: String| -> Result<()> {
        let p = out_dir.join(&name);
        std::fs::write(&p, body).with_context(|| format!("write {}", p.display()))?;
        println!("  wrote {}", p.display());
        Ok(())
    };
    w(
        format!("unknown_uuids_{tag}.txt"),
        c.new.iter().map(|d| format!("{}\n", d.uuid)).collect(),
    )?;
    w(
        format!("unknown_uuids_{tag}.tsv"),
        std::iter::once("uuid\tsrn\tprimary_di\tstatus\n".to_string())
            .chain(c.new.iter().map(|d| {
                format!(
                    "{}\t{}\t{}\t{}\n",
                    d.uuid,
                    d.srn,
                    d.primary_di,
                    short(&d.status)
                )
            }))
            .collect(),
    )?;
    w(
        format!("listing_urls_{tag}_missing.txt"),
        c.missing_pages
            .iter()
            .map(|(s, n)| format!("{}\n", listing_url(s, *n)))
            .collect(),
    )?;
    let mut changes = String::from("uuid\tsrn\tprimary_di\tkind\told\tnew\n");
    for (d, old) in &c.version_bumps {
        changes.push_str(&format!(
            "{}\t{}\t{}\tversion\t{}\t{}\n",
            d.uuid,
            d.srn,
            d.primary_di,
            old,
            d.version.unwrap_or_default()
        ));
    }
    for (d, old) in &c.status_changes {
        changes.push_str(&format!(
            "{}\t{}\t{}\tstatus\t{}\t{}\n",
            d.uuid,
            d.srn,
            d.primary_di,
            short(old),
            short(&d.status)
        ));
    }
    w(format!("listing_changes_{tag}.tsv"), changes)?;
    w(
        format!("no_longer_listed_{tag}.tsv"),
        std::iter::once("uuid\tsrn\tprimary_di\n".to_string())
            .chain(
                c.no_longer_listed
                    .iter()
                    .map(|(u, k)| format!("{}\t{}\t{}\n", u, k.srn, k.primary_di)),
            )
            .collect(),
    )?;
    Ok(c)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(total: u64, rows: &[(&str, &str, &str, i64)]) -> String {
        let content: Vec<serde_json::Value> = rows
            .iter()
            .map(|(u, g, s, v)| {
                serde_json::json!({
                    "uuid": u, "primaryDi": g, "versionNumber": v,
                    "deviceStatusType": {"code": format!("refdata.device-model-status.{s}")}
                })
            })
            .collect();
        serde_json::json!({"content": content, "totalElements": total}).to_string()
    }

    fn known(srn: &str, g: &str, s: &str, v: i64) -> KnownDevice {
        KnownDevice {
            srn: srn.into(),
            primary_di: g.into(),
            status: format!("refdata.device-model-status.{s}"),
            version: Some(v),
        }
    }

    #[test]
    fn file_names() {
        assert_eq!(
            parse_file_name("US-MF-000008255_page32.json"),
            Some(("US-MF-000008255".into(), 32))
        );
        assert_eq!(parse_file_name("notes.txt"), None);
    }

    #[test]
    fn rejects_non_listing_bodies() {
        assert!(parse_page("X", "<html>busy</html>").is_err());
        assert!(parse_page("X", "{\"message\":\"429\"}").is_err());
    }

    #[test]
    fn finds_new_bumps_status_gone_and_missing_pages() {
        let mut srns = BTreeMap::new();
        // A: complete (1 page, 3 devices)
        let (t, d) = parse_page(
            "A",
            &page(
                3,
                &[
                    ("a1", "g1", "on-the-market", 2),
                    ("a2", "g2", "no-longer-on-the-market", 1),
                    ("a3", "g9", "on-the-market", 1),
                ],
            ),
        )
        .unwrap();
        srns.insert(
            "A".to_string(),
            SrnPages {
                total: t,
                pages: [0].into(),
                devices: d,
            },
        );
        // B: 301 devices but only page 0 → page 1 missing, no "gone" for B
        let (t, d) = parse_page("B", &page(301, &[("b1", "h1", "on-the-market", 1)])).unwrap();
        srns.insert(
            "B".to_string(),
            SrnPages {
                total: t,
                pages: [0].into(),
                devices: d,
            },
        );
        let mut k = HashMap::new();
        k.insert("a1".to_string(), known("A", "g1", "on-the-market", 1)); // bump 1→2
        k.insert("a2".to_string(), known("A", "g2", "on-the-market", 1)); // status change
        k.insert("a4".to_string(), known("A", "g4", "on-the-market", 1)); // gone
        k.insert("b2".to_string(), known("B", "h2", "on-the-market", 1)); // B incomplete: not gone
        k.insert("x".to_string(), known("C", "g9", "on-the-market", 1)); // g9 known elsewhere
        let c = compare(&srns, &k);
        assert_eq!(
            c.new.iter().map(|d| d.uuid.as_str()).collect::<Vec<_>>(),
            ["a3", "b1"]
        );
        assert_eq!(c.new_known_gtin, 1);
        assert_eq!(c.version_bumps.len(), 1);
        assert_eq!(c.status_changes.len(), 1);
        assert_eq!(
            c.no_longer_listed
                .iter()
                .map(|g| g.0.as_str())
                .collect::<Vec<_>>(),
            ["a4"]
        );
        assert_eq!(c.missing_pages, vec![("B".to_string(), 1)]);
        assert_eq!(c.total, 304);
    }
}
