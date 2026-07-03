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

fn fetch_page(agent: &ureq::Agent, limiter: &RateLimiter, page: u32) -> Result<ActorPage> {
    let url = format!(
        "{}?page={}&pageSize={}&iso2Code=en&languageIso2Code=en",
        ACTOR_BASE_URL, page, ACTOR_PAGE_SIZE
    );
    let body = eudamed_get(agent, limiter, &url, 6)
        .map_err(|e| anyhow::anyhow!("actor page {page}: {e}"))?;
    serde_json::from_str(&body).with_context(|| format!("parsing actor page {page}"))
}

/// Upsert every actor on a page; returns how many rows were written. The DB
/// write is serialized behind the shared mutex — but page *fetches* (the slow
/// part: 2–15 s each due to deep-offset pagination) happen outside the lock, in
/// parallel — so the mutex is held only microseconds per page.
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

/// Default parallel fetch width. EUDAMED's `/eos` pages get progressively slower
/// as the offset grows (deep-offset pagination: ~2 s early, ~12–15 s past
/// offset 20 k). Single-threaded that collapses throughput to ~5 pages/min. The
/// shared [`RateLimiter`] caps the AGGREGATE rate under the ~60/60 s budget, so
/// threads don't raise the request rate — they only overlap the long latencies
/// (~12 s ÷ 1.05 s pacing ≈ 12 requests need to be in-flight to saturate the
/// paced budget). 16 keeps the budget filled with margin.
const DEFAULT_ACTOR_THREADS: usize = 16;

/// Fetch every actor from EUDAMED and upsert into the `actors` table.
///
/// `rate_interval_ms` paces the AGGREGATE request rate (default 1050 ms ≈
/// 57/min, under the shared budget); `threads` overlap the deep-offset latency
/// so the paced budget is actually filled (single-threaded it can't be — each
/// request blocks ~12 s). Re-runnable: `upsert_actor` refreshes existing SRNs
/// and inserts new ones, and **never deletes**, so existing rows are kept and
/// only enriched (e.g. backfilling the v1.0.86 address columns).
/// Returns `(fetched, total_reported)`.
pub fn sync_actors(conn: Connection, rate_interval_ms: u64, threads: usize) -> Result<(u64, u64)> {
    let agent = eudamed_agent();
    let limiter = RateLimiter::new(Duration::from_millis(rate_interval_ms));
    let threads = threads.max(1);

    let first = fetch_page(&agent, &limiter, 0)?;
    let total_pages = first.total_pages.max(1);
    let total_elements = first.total_elements;
    eprintln!(
        "sync-actors: {} actors across {} pages, {} threads, {}ms/req pacing (aggregate ~{}/min)",
        total_elements,
        total_pages,
        threads,
        rate_interval_ms,
        60_000 / rate_interval_ms.max(1),
    );

    let conn = Mutex::new(conn);
    let fetched = AtomicU64::new(0);
    fetched.fetch_add(store_page(&conn, first)?, Ordering::Relaxed);
    let done_pages = AtomicU64::new(1);

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .context("building actor fetch thread pool")?;

    pool.install(|| {
        (1..total_pages).into_par_iter().for_each(|page| {
            match fetch_page(&agent, &limiter, page) {
                Ok(p) => match store_page(&conn, p) {
                    Ok(n) => {
                        fetched.fetch_add(n, Ordering::Relaxed);
                    }
                    Err(e) => eprintln!("sync-actors: WARN store page {page}: {e}"),
                },
                Err(e) => eprintln!("sync-actors: WARN {e} — skipping page {page}"),
            }
            let d = done_pages.fetch_add(1, Ordering::Relaxed) + 1;
            if d % 100 == 0 || d as u32 == total_pages {
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
    Ok((fetched, total_elements))
}
