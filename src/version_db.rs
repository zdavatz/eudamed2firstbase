use anyhow::{Context, Result};
use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use std::path::Path;

/// Default database path
pub const VERSION_DB_PATH: &str = "db/version_tracking.db";

/// Version record for one UDI-DI (keyed by UUID)
#[derive(Debug, Clone, Default)]
pub struct VersionRecord {
    pub uuid: String,
    pub gtin: String,
    /// SHA256 hash of the full Detail API JSON response
    pub detail_hash: String,
    // UDI-DI root (from Detail API)
    pub udi_version: Option<u32>,
    pub udi_date: Option<String>,
    // Basic UDI-DI (from BUDI API)
    pub budi_version: Option<u32>,
    pub budi_date: Option<String>,
    // Manufacturer (from BUDI API -> manufacturer)
    pub mfr_version: Option<u32>,
    pub mfr_date: Option<String>,
    // Authorised Representative (from BUDI API -> authorisedRepresentative)
    pub ar_version: Option<u32>,
    pub ar_date: Option<String>,
    // Certificates (from BUDI API -> deviceCertificateInfoList)
    // Stored as JSON array of version numbers, e.g. "[1,2,1]"
    pub cert_versions: Option<String>,
    // Package / containedItem (from Detail API)
    pub pkg_version: Option<u32>,
    pub pkg_date: Option<String>,
    // MarketInfo (from Detail API -> marketInfoLink)
    pub market_version: Option<u32>,
    pub market_date: Option<String>,
    // Device status (from Detail API -> deviceStatus)
    pub device_status: Option<String>,
    pub status_date: Option<String>,
    // Product designer (from Detail API -> productDesigner)
    pub designer_version: Option<u32>,
    pub designer_date: Option<String>,
    // Tracking metadata
    pub last_synced: Option<String>,
}

/// Which sections changed between old and new version
#[derive(Debug, Default)]
pub struct ChangeSet {
    pub is_new: bool,
    pub udi_changed: bool,
    pub budi_changed: bool,
    pub mfr_changed: bool,
    pub ar_changed: bool,
    pub cert_changed: bool,
    pub pkg_changed: bool,
    pub market_changed: bool,
    pub status_changed: bool,
    pub designer_changed: bool,
}

impl ChangeSet {
    pub fn has_any_change(&self) -> bool {
        self.is_new
            || self.udi_changed
            || self.budi_changed
            || self.mfr_changed
            || self.ar_changed
            || self.cert_changed
            || self.pkg_changed
            || self.market_changed
            || self.status_changed
            || self.designer_changed
    }

    pub fn summary(&self) -> String {
        if self.is_new {
            return "NEW".to_string();
        }
        let mut parts = Vec::new();
        if self.udi_changed { parts.push("UDI"); }
        if self.budi_changed { parts.push("BUDI"); }
        if self.mfr_changed { parts.push("MFR"); }
        if self.ar_changed { parts.push("AR"); }
        if self.cert_changed { parts.push("CERT"); }
        if self.pkg_changed { parts.push("PKG"); }
        if self.market_changed { parts.push("MARKET"); }
        if self.status_changed { parts.push("STATUS"); }
        if self.designer_changed { parts.push("DESIGNER"); }
        if parts.is_empty() {
            "UNCHANGED".to_string()
        } else {
            parts.join("+")
        }
    }
}

/// Open (or create) the version tracking database
pub fn open_db(path: &Path) -> Result<Connection> {
    let conn = Connection::open(path)
        .with_context(|| format!("Failed to open version DB at {}", path.display()))?;

    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS udi_versions (
            uuid TEXT PRIMARY KEY,
            gtin TEXT NOT NULL DEFAULT '',
            detail_hash TEXT NOT NULL DEFAULT '',
            udi_version INTEGER,
            udi_date TEXT,
            budi_version INTEGER,
            budi_date TEXT,
            mfr_version INTEGER,
            mfr_date TEXT,
            ar_version INTEGER,
            ar_date TEXT,
            cert_versions TEXT,
            pkg_version INTEGER,
            pkg_date TEXT,
            market_version INTEGER,
            market_date TEXT,
            device_status TEXT,
            status_date TEXT,
            designer_version INTEGER,
            designer_date TEXT,
            last_synced TEXT NOT NULL DEFAULT ''
        );
        CREATE INDEX IF NOT EXISTS idx_gtin ON udi_versions(gtin);
        CREATE INDEX IF NOT EXISTS idx_last_synced ON udi_versions(last_synced);

        CREATE TABLE IF NOT EXISTS push_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            uuid TEXT NOT NULL,
            gtin TEXT NOT NULL DEFAULT '',
            pushed_at TEXT NOT NULL,
            request_id TEXT,
            status TEXT NOT NULL,
            error_code TEXT,
            error_msg TEXT,
            publish_gln TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_push_log_uuid ON push_log(uuid);
        CREATE INDEX IF NOT EXISTS idx_push_log_status ON push_log(status);
        CREATE INDEX IF NOT EXISTS idx_push_log_pushed_at ON push_log(pushed_at);

        CREATE TABLE IF NOT EXISTS request_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            request_id TEXT NOT NULL UNIQUE,
            request_type TEXT NOT NULL,
            submitted_at TEXT NOT NULL,
            status TEXT NOT NULL DEFAULT 'SUBMITTED',
            polled_at TEXT,
            item_count INTEGER DEFAULT 0,
            accepted INTEGER DEFAULT 0,
            rejected INTEGER DEFAULT 0,
            publish_gln TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_request_log_status ON request_log(status);

        CREATE TABLE IF NOT EXISTS swissdamed_push_log (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            uuid TEXT NOT NULL,
            correlation_id TEXT,
            pushed_at TEXT NOT NULL,
            endpoint TEXT NOT NULL,
            status TEXT NOT NULL,
            error_code TEXT,
            error_msg TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_swissdamed_uuid ON swissdamed_push_log(uuid);
        CREATE INDEX IF NOT EXISTS idx_swissdamed_status ON swissdamed_push_log(status);",
    )?;

    Ok(conn)
}

/// Compute SHA256 hash of a JSON string
pub fn hash_json(json: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(json.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// Look up existing version record by UUID. Returns None if not found.
pub fn get_version(conn: &Connection, uuid: &str) -> Result<Option<VersionRecord>> {
    let mut stmt = conn.prepare_cached(
        "SELECT uuid, gtin, detail_hash,
                udi_version, udi_date, budi_version, budi_date,
                mfr_version, mfr_date, ar_version, ar_date,
                cert_versions, pkg_version, pkg_date,
                market_version, market_date, device_status, status_date,
                designer_version, designer_date, last_synced
         FROM udi_versions WHERE uuid = ?1",
    )?;

    let result = stmt
        .query_row(params![uuid], |row| {
            Ok(VersionRecord {
                uuid: row.get(0)?,
                gtin: row.get(1)?,
                detail_hash: row.get(2)?,
                udi_version: row.get(3)?,
                udi_date: row.get(4)?,
                budi_version: row.get(5)?,
                budi_date: row.get(6)?,
                mfr_version: row.get(7)?,
                mfr_date: row.get(8)?,
                ar_version: row.get(9)?,
                ar_date: row.get(10)?,
                cert_versions: row.get(11)?,
                pkg_version: row.get(12)?,
                pkg_date: row.get(13)?,
                market_version: row.get(14)?,
                market_date: row.get(15)?,
                device_status: row.get(16)?,
                status_date: row.get(17)?,
                designer_version: row.get(18)?,
                designer_date: row.get(19)?,
                last_synced: row.get(20)?,
            })
        })
        .ok();

    Ok(result)
}

/// Insert or update a version record
pub fn upsert_version(conn: &Connection, rec: &VersionRecord) -> Result<()> {
    conn.execute(
        "INSERT INTO udi_versions (
            uuid, gtin, detail_hash,
            udi_version, udi_date, budi_version, budi_date,
            mfr_version, mfr_date, ar_version, ar_date,
            cert_versions, pkg_version, pkg_date,
            market_version, market_date, device_status, status_date,
            designer_version, designer_date, last_synced
        ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21)
        ON CONFLICT(uuid) DO UPDATE SET
            gtin=excluded.gtin, detail_hash=excluded.detail_hash,
            udi_version=excluded.udi_version, udi_date=excluded.udi_date,
            budi_version=excluded.budi_version, budi_date=excluded.budi_date,
            mfr_version=excluded.mfr_version, mfr_date=excluded.mfr_date,
            ar_version=excluded.ar_version, ar_date=excluded.ar_date,
            cert_versions=excluded.cert_versions,
            pkg_version=excluded.pkg_version, pkg_date=excluded.pkg_date,
            market_version=excluded.market_version, market_date=excluded.market_date,
            device_status=excluded.device_status, status_date=excluded.status_date,
            designer_version=excluded.designer_version, designer_date=excluded.designer_date,
            last_synced=excluded.last_synced",
        params![
            rec.uuid, rec.gtin, rec.detail_hash,
            rec.udi_version, rec.udi_date, rec.budi_version, rec.budi_date,
            rec.mfr_version, rec.mfr_date, rec.ar_version, rec.ar_date,
            rec.cert_versions, rec.pkg_version, rec.pkg_date,
            rec.market_version, rec.market_date, rec.device_status, rec.status_date,
            rec.designer_version, rec.designer_date, rec.last_synced,
        ],
    )?;
    Ok(())
}

/// Compare a new version record against the stored one and return what changed.
/// Fast path: if detail_hash matches, nothing changed.
pub fn detect_changes(
    conn: &Connection,
    new_rec: &VersionRecord,
) -> Result<ChangeSet> {
    let old = match get_version(conn, &new_rec.uuid)? {
        Some(old) => old,
        None => {
            return Ok(ChangeSet {
                is_new: true,
                ..Default::default()
            });
        }
    };

    // Fast path: hash unchanged → skip detailed comparison
    if old.detail_hash == new_rec.detail_hash {
        return Ok(ChangeSet::default());
    }

    Ok(ChangeSet {
        is_new: false,
        udi_changed: old.udi_version != new_rec.udi_version
            || old.udi_date != new_rec.udi_date,
        budi_changed: old.budi_version != new_rec.budi_version
            || old.budi_date != new_rec.budi_date,
        mfr_changed: old.mfr_version != new_rec.mfr_version
            || old.mfr_date != new_rec.mfr_date,
        ar_changed: old.ar_version != new_rec.ar_version
            || old.ar_date != new_rec.ar_date,
        cert_changed: old.cert_versions != new_rec.cert_versions,
        pkg_changed: old.pkg_version != new_rec.pkg_version
            || old.pkg_date != new_rec.pkg_date,
        market_changed: old.market_version != new_rec.market_version
            || old.market_date != new_rec.market_date,
        status_changed: old.device_status != new_rec.device_status
            || old.status_date != new_rec.status_date,
        designer_changed: old.designer_version != new_rec.designer_version
            || old.designer_date != new_rec.designer_date,
    })
}

/// Extract version info from raw Detail API JSON (serde_json::Value).
/// This parses the version fields from each sub-section without needing
/// the full typed struct.
pub fn extract_detail_versions(json_str: &str) -> VersionRecord {
    let mut rec = VersionRecord::default();
    rec.detail_hash = hash_json(json_str);

    let val: serde_json::Value = match serde_json::from_str(json_str) {
        Ok(v) => v,
        Err(_) => return rec,
    };

    // UDI-DI root
    rec.udi_version = val.get("versionNumber").and_then(|v| v.as_u64()).map(|v| v as u32);
    rec.udi_date = val.get("versionDate").and_then(|v| v.as_str()).map(|s| s.to_string());

    // UUID
    rec.uuid = val.get("uuid").and_then(|v| v.as_str()).unwrap_or("").to_string();

    // GTIN (from primaryDi.code)
    rec.gtin = val.get("primaryDi")
        .and_then(|di| di.get("code"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // MarketInfo
    if let Some(mi) = val.get("marketInfoLink") {
        rec.market_version = mi.get("versionNumber").and_then(|v| v.as_u64()).map(|v| v as u32);
        rec.market_date = mi.get("versionDate").and_then(|v| v.as_str()).map(|s| s.to_string());
    }

    // Package (containedItem)
    if let Some(ci) = val.get("containedItem") {
        rec.pkg_version = ci.get("versionNumber").and_then(|v| v.as_u64()).map(|v| v as u32);
        rec.pkg_date = ci.get("versionDate").and_then(|v| v.as_str()).map(|s| s.to_string());
    }

    // Device status
    if let Some(ds) = val.get("deviceStatus") {
        rec.device_status = ds.get("type")
            .and_then(|t| t.get("code"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        rec.status_date = ds.get("statusDate").and_then(|v| v.as_str()).map(|s| s.to_string());
    }

    // Product designer
    if let Some(pd) = val.get("productDesigner") {
        rec.designer_version = pd.get("versionNumber").and_then(|v| v.as_u64()).map(|v| v as u32);
        rec.designer_date = pd.get("versionDate").and_then(|v| v.as_str()).map(|s| s.to_string());
    }

    rec
}

/// Extract BUDI-related version info from raw Basic UDI-DI JSON.
/// Merges into an existing VersionRecord (adds budi, mfr, ar, cert fields).
pub fn merge_budi_versions(rec: &mut VersionRecord, budi_json: &str) {
    let val: serde_json::Value = match serde_json::from_str(budi_json) {
        Ok(v) => v,
        Err(_) => return,
    };

    // Basic UDI-DI root
    rec.budi_version = val.get("versionNumber").and_then(|v| v.as_u64()).map(|v| v as u32);
    rec.budi_date = val.get("versionDate").and_then(|v| v.as_str()).map(|s| s.to_string());

    // Manufacturer
    if let Some(mfr) = val.get("manufacturer") {
        rec.mfr_version = mfr.get("versionNumber").and_then(|v| v.as_u64()).map(|v| v as u32);
        rec.mfr_date = mfr.get("lastUpdateDate").and_then(|v| v.as_str()).map(|s| s.to_string());
    }

    // Authorised Representative
    if let Some(ar) = val.get("authorisedRepresentative") {
        rec.ar_version = ar.get("versionNumber").and_then(|v| v.as_u64()).map(|v| v as u32);
        rec.ar_date = ar.get("lastUpdateDate").and_then(|v| v.as_str()).map(|s| s.to_string());
    }

    // Certificates — collect version numbers as JSON array
    if let Some(certs) = val.get("deviceCertificateInfoListForDisplay").and_then(|v| v.as_array()) {
        let versions: Vec<u64> = certs
            .iter()
            .filter_map(|c| c.get("versionNumber").and_then(|v| v.as_u64()))
            .collect();
        if !versions.is_empty() {
            rec.cert_versions = Some(serde_json::to_string(&versions).unwrap_or_default());
        }
    }
}

/// Get total count of tracked UDI-DIs
pub fn count_records(conn: &Connection) -> Result<u64> {
    let count: u64 = conn.query_row("SELECT COUNT(*) FROM udi_versions", [], |row| row.get(0))?;
    Ok(count)
}

/// Get summary statistics
#[allow(dead_code)]
pub fn stats(conn: &Connection) -> Result<(u64, u64, u64)> {
    let total: u64 = conn.query_row("SELECT COUNT(*) FROM udi_versions", [], |row| row.get(0))?;
    let with_gtin: u64 = conn.query_row(
        "SELECT COUNT(*) FROM udi_versions WHERE gtin != ''",
        [],
        |row| row.get(0),
    )?;
    let with_budi: u64 = conn.query_row(
        "SELECT COUNT(*) FROM udi_versions WHERE budi_version IS NOT NULL",
        [],
        |row| row.get(0),
    )?;
    Ok((total, with_gtin, with_budi))
}
