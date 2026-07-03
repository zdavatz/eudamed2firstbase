//! EUDAMED actor / economic-operator registry sync.
//!
//! The `/api/eos` listing returns every registered actor (manufacturer, AR,
//! importer, …) with its SRN and name — independent of any device. This module
//! paces through all pages via the shared [`RateLimiter`] + [`eudamed_get`]
//! choke-point (same rate-limit survival as the device download: honors 429
//! `Retry-After`, stays under EUDAMED's ~60/60 s budget) and upserts each actor
//! into the `actors` table, keyed by SRN. Devices link via
//! `actors.srn = listing_cache.srn`.
//!
//! CLI: `cargo run sync-actors [--rate-ms N]` — full refresh; re-runnable
//! (upsert, so existing rows are updated in place, new actors added).

use anyhow::{Context, Result};
use rayon::prelude::*;
use rusqlite::Connection;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use crate::download::{eudamed_agent, eudamed_get, RateLimiter};
use crate::mappings::ACTOR_COUNTRY_CODES;
use crate::version_db::{count_actors, upsert_actor, ActorRecord};

const ACTOR_BASE_URL: &str = "https://ec.europa.eu/tools/eudamed/api/eos";
/// `pageSize` is capped to 20 server-side regardless of what we ask.
const ACTOR_PAGE_SIZE: usize = 20;

#[derive(serde::Deserialize)]
struct ActorPage {
    #[serde(default)]
    content: Vec<ActorItem>,
    #[serde(rename = "totalPages", default)]
    total_pages: u32,
    #[serde(rename = "totalElements", default)]
    total_elements: u64,
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct CodeObj {
    code: Option<String>,
}

impl CodeObj {
    /// `{"code":"refdata.actor-status.active"}` -> `"active"`.
    fn tail(&self) -> String {
        self.code
            .as_deref()
            .map(|c| c.rsplit('.').next().unwrap_or(c).to_string())
            .unwrap_or_default()
    }
}

#[derive(serde::Deserialize, Default)]
#[serde(default)]
struct ActorItem {
    srn: Option<String>,
    name: Option<String>,
    #[serde(rename = "roleName")]
    role_name: Option<String>,
    #[serde(rename = "actorStatus")]
    actor_status: CodeObj,
    #[serde(rename = "countryIso2Code")]
    country_iso2: Option<String>,
    #[serde(rename = "countryName")]
    country_name: Option<String>,
    #[serde(rename = "eudamedIdentifier")]
    eudamed_identifier: Option<String>,
    #[serde(rename = "electronicMail")]
    email: Option<String>,
    telephone: Option<String>,
    #[serde(rename = "streetName")]
    street_name: Option<String>,
    #[serde(rename = "buildingNumber")]
    building_number: Option<String>,
    #[serde(rename = "postalZone")]
    postal_zone: Option<String>,
    #[serde(rename = "cityName")]
    city_name: Option<String>,
    #[serde(rename = "geographicalAddress")]
    geographical_address: Option<String>,
    #[serde(rename = "countryType")]
    country_type: Option<String>,
    #[serde(rename = "abbreviatedName")]
    abbreviated_name: Option<String>,
    #[serde(rename = "dateOfRegistration")]
    date_of_registration: Option<String>,
    uuid: Option<String>,
}

impl ActorItem {
    fn into_record(self) -> Option<ActorRecord> {
        let srn = self.srn.filter(|s| !s.trim().is_empty())?;
        Some(ActorRecord {
            srn,
            name: self.name.unwrap_or_default(),
            role_name: self.role_name.unwrap_or_default(),
            actor_status: self.actor_status.tail(),
            country_iso2: self.country_iso2.unwrap_or_default(),
            country_name: self.country_name.unwrap_or_default(),
            eudamed_identifier: self.eudamed_identifier.unwrap_or_default(),
            email: self.email.unwrap_or_default(),
            telephone: self.telephone.unwrap_or_default(),
            street_name: self.street_name.unwrap_or_default(),
            building_number: self.building_number.unwrap_or_default(),
            postal_zone: self.postal_zone.unwrap_or_default(),
            city_name: self.city_name.unwrap_or_default(),
            geographical_address: self.geographical_address.unwrap_or_default(),
            country_type: self.country_type.unwrap_or_default(),
            abbreviated_name: self.abbreviated_name.unwrap_or_default(),
            date_of_registration: self.date_of_registration.unwrap_or_default(),
            uuid: self.uuid.unwrap_or_default(),
        })
    }
}

/// Fetch one page of the `/eos` listing filtered to `country` (ISO alpha-2).
/// Filtering per country keeps the offset shallow — the unfiltered listing's
/// deep-offset pagination (page latency 2 s → 15 s past offset ~20 k) is the
/// whole reason this sync partitions by country.
fn fetch_page(
    agent: &ureq::Agent,
    limiter: &RateLimiter,
    country: &str,
    page: u32,
) -> Result<ActorPage> {
    let url = format!(
        "{}?page={}&pageSize={}&iso2Code=en&languageIso2Code=en&countryIso2Code={}",
        ACTOR_BASE_URL, page, ACTOR_PAGE_SIZE, country
    );
    let body = eudamed_get(agent, limiter, &url, 6)
        .map_err(|e| anyhow::anyhow!("actor {country} page {page}: {e}"))?;
    serde_json::from_str(&body).with_context(|| format!("parsing actor {country} page {page}"))
}

/// Upsert every actor on a page; returns how many rows were written. The DB
/// write is serialized behind the shared mutex — but page *fetches* happen
/// outside the lock, in parallel — so the mutex is held only microseconds.
fn store_page(conn: &Mutex<Connection>, page: ActorPage) -> Result<u64> {
    let c = conn.lock().unwrap_or_else(|e| e.into_inner());
    let mut n = 0u64;
    for item in page.content {
        if let Some(rec) = item.into_record() {
            upsert_actor(&c, &rec)?;
            n += 1;
        }
    }
    Ok(n)
}

/// Fetch every actor from EUDAMED and upsert into the `actors` table, **paginated
/// per country** to sidestep deep-offset latency.
///
/// EUDAMED's unfiltered `/eos` listing uses deep-offset pagination — `page=N`
/// scans+discards `N×20` rows, so latency climbs from ~2 s (early) to ~15 s (past
/// offset ~20 k). A full unfiltered walk crawls at ~5 pages/min and any attempt to
/// parallelize *through* it just trips `/eos`'s ~40/min 429 budget into a 60 s
/// `Retry-After` lockstep. Partitioning by `countryIso2Code` keeps every partition
/// small, so **no page reaches a deep offset** — pages stay ~2 s and throughput
/// becomes rate-bound (not latency-bound). Two phases:
///   1. probe each of [`ACTOR_COUNTRY_CODES`] with page 0 (also stores that page);
///   2. fan out the remaining `(country, page)` work across a small pool.
///
/// `/eos` tolerates only ~30/min sustained; ANY multi-thread burst eventually
/// trips a 429 → 60 s `Retry-After` and (with >1 thread) synchronizes into a
/// lockstep that collapses throughput to ~5/min. So the default is **1 thread**
/// (`--threads N` to override, at your own 429 risk): a single thread self-limits
/// via page latency, can't lockstep (a lone 429 is just absorbed), and with fast
/// per-country pages sustains ~30/min — the `/eos` ceiling anyway. The shared
/// [`RateLimiter`] (default 2000 ms ≈ 30/min) caps even the sub-second empty-country
/// probes so the sustained rate never exceeds budget. Re-runnable: `upsert_actor`
/// refreshes existing SRNs and **never deletes**, so existing rows are kept and
/// enriched.
/// Returns `(fetched, total_reported)`.
pub fn sync_actors(conn: Connection, rate_interval_ms: u64, threads: usize) -> Result<(u64, u64)> {
    let agent = eudamed_agent();
    let limiter = RateLimiter::new(Duration::from_millis(rate_interval_ms));
    let threads = threads.max(1);
    let conn = Mutex::new(conn);
    let fetched = AtomicU64::new(0);

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .context("building actor fetch thread pool")?;

    eprintln!(
        "sync-actors: phase 1 — probing {} countries, {} threads, {}ms/req (~{}/min)",
        ACTOR_COUNTRY_CODES.len(),
        threads,
        rate_interval_ms,
        60_000 / rate_interval_ms.max(1),
    );

    // Phase 1: page 0 per country → (country, remaining_pages, total_elements).
    // Also stores page 0's actors. Countries with 0 actors are dropped.
    let mut total_reported: u64 = 0;
    let country_pages: Vec<(&'static str, u32)> = pool.install(|| {
        ACTOR_COUNTRY_CODES
            .par_iter()
            .filter_map(|&cc| match fetch_page(&agent, &limiter, cc, 0) {
                Ok(p) => {
                    let total = p.total_elements;
                    let total_pages = p.total_pages;
                    match store_page(&conn, p) {
                        Ok(n) => {
                            fetched.fetch_add(n, Ordering::Relaxed);
                        }
                        Err(e) => eprintln!("sync-actors: WARN store {cc} page 0: {e}"),
                    }
                    if total > 0 {
                        Some((cc, total, total_pages))
                    } else {
                        None
                    }
                }
                Err(e) => {
                    eprintln!("sync-actors: WARN {e} — skipping country {cc}");
                    None
                }
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|(cc, total, total_pages)| {
                total_reported += total;
                (cc, total_pages)
            })
            .collect()
    });

    // Phase 2: flat (country, page) work-list for pages 1..total_pages.
    let work: Vec<(&'static str, u32)> = country_pages
        .iter()
        .flat_map(|&(cc, tp)| (1..tp).map(move |pg| (cc, pg)))
        .collect();
    let total_pages = work.len();
    eprintln!(
        "sync-actors: phase 2 — {} countries with actors ({} total), {} further pages",
        country_pages.len(),
        total_reported,
        total_pages,
    );

    let done = AtomicU64::new(0);
    pool.install(|| {
        work.par_iter().for_each(|&(cc, pg)| {
            match fetch_page(&agent, &limiter, cc, pg) {
                Ok(p) => match store_page(&conn, p) {
                    Ok(n) => {
                        fetched.fetch_add(n, Ordering::Relaxed);
                    }
                    Err(e) => eprintln!("sync-actors: WARN store {cc} page {pg}: {e}"),
                },
                Err(e) => eprintln!("sync-actors: WARN {e} — skipping {cc} page {pg}"),
            }
            let d = done.fetch_add(1, Ordering::Relaxed) + 1;
            if d % 100 == 0 || d as usize == total_pages {
                eprintln!(
                    "sync-actors: {}/{} pages — {} actors upserted",
                    d,
                    total_pages,
                    fetched.load(Ordering::Relaxed)
                );
            }
        });
    });

    let conn = conn.into_inner().unwrap_or_else(|e| e.into_inner());
    let in_db = count_actors(&conn)?;
    let fetched = fetched.load(Ordering::Relaxed);
    eprintln!(
        "sync-actors: done — {} fetched this run, {} total in actors table",
        fetched, in_db
    );
    Ok((fetched, total_reported))
}
