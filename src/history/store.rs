//! IndexedDB storage layer for analysis history.
//!
//! Uses [`indexed_db`] to interact with the browser's IndexedDB.
//! Database name: `pineapplehub_history`.
//! Object stores: `sessions` (key: `session_id`) and `records` (key: `id`).

use indexed_db::{Database, Factory};
use serde::Serialize;
use serde_wasm_bindgen::Serializer;
use wasm_bindgen::JsValue;

use super::model::{AnalysisRecord, SessionMeta, SessionSummary};

/// Maximum number of records before cache is considered "full".
pub(crate) const MAX_RECORDS: u32 = 5000;
/// Warning thresholds as fractions of [`MAX_RECORDS`].
const WARN_THRESHOLD: f64 = 0.80;
const CRITICAL_THRESHOLD: f64 = 0.90;
/// Target ratio after one-click cleanup.
const CLEANUP_TARGET: f64 = 0.70;

/// Cache capacity status.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CacheWarningLevel {
    /// Below 80% — no warning.
    Ok,
    /// 80–89% — soft non-blocking banner.
    Caution {
        current: u32,
        cleanable_sessions: u32,
    },
    /// 90–99% — persistent banner with cleanup button.
    Warning {
        current: u32,
        cleanable_sessions: u32,
    },
    /// 100% — block new writes.
    Full { current: u32 },
}

const DB_NAME: &str = "pineapplehub_history";
const DB_VERSION: u32 = 1;
const SESSIONS_STORE: &str = "sessions";
const RECORDS_STORE: &str = "records";

/// The user-error type threaded through `indexed_db::Error<E>`.
type StoreError = String;
/// Convenience alias for our DB handle type.
pub(crate) type Db = Database<StoreError>;
/// Convenience alias for our result type.
type StoreResult<T> = Result<T, indexed_db::Error<StoreError>>;

/// Open (or create) the history database.
pub(crate) async fn open_db() -> StoreResult<Db> {
    let factory = Factory::<StoreError>::get()?;
    factory
        .open(DB_NAME, DB_VERSION, |evt| async move {
            let db = evt.database();
            db.build_object_store(SESSIONS_STORE)
                .key_path("session_id")
                .create()?;
            let records_store = db
                .build_object_store(RECORDS_STORE)
                .key_path("id")
                .create()?;
            records_store
                .build_index("session_id", "session_id")
                .create()?;
            records_store
                .build_index("timestamp", "timestamp")
                .create()?;
            Ok(())
        })
        .await
}

/// Helper: serialize a Rust value to `JsValue` via serde.
fn to_js<T: Serialize>(val: &T) -> JsValue {
    val.serialize(&Serializer::json_compatible())
        .expect("serialization should not fail for simple types")
}

/// Helper: deserialize a `JsValue` to a Rust type via serde.
fn from_js<T: serde::de::DeserializeOwned>(val: JsValue) -> Option<T> {
    serde_wasm_bindgen::from_value(val).ok()
}

// ──────────────────────── Write Operations ────────────────────────

/// Save a complete batch session (metadata + all records).
///
/// Uses two transactions: one for session metadata, one for ALL records.
/// With `indexed-db`, multiple `.await` calls within a `.run()` closure
/// are safe — the transaction stays active until the closure returns.
pub(crate) async fn save_session(
    db: &Db,
    meta: &SessionMeta,
    records: &[AnalysisRecord],
) -> StoreResult<()> {
    log::info!(
        "save_session: saving session {} with {} records",
        meta.session_id,
        records.len()
    );

    // Transaction 1: save session meta
    let meta_js = to_js(meta);
    db.transaction(&[SESSIONS_STORE])
        .rw()
        .run(|t| async move {
            t.object_store(SESSIONS_STORE)?.put(&meta_js).await?;
            Ok(())
        })
        .await?;
    log::info!("save_session: session meta committed");

    // Transaction 2: save ALL records in ONE transaction
    let records_js: Vec<JsValue> = records.iter().map(|r| to_js(r)).collect();
    let record_count = records_js.len();
    db.transaction(&[RECORDS_STORE])
        .rw()
        .run(move |t| async move {
            let store = t.object_store(RECORDS_STORE)?;
            for (i, val) in records_js.iter().enumerate() {
                store.put(val).await?;
                if (i + 1) % 50 == 0 || i + 1 == record_count {
                    log::info!("save_session: {}/{} records saved", i + 1, record_count);
                }
            }
            Ok(())
        })
        .await?;

    log::info!("save_session: all {} records committed", record_count);
    Ok(())
}

/// Append new records to an existing session and update its aggregate counts.
///
/// Used by the Camera page "Append to existing session" mode.  
/// If the session does not exist the function returns `Ok(())` silently
/// (the records are still written, so no data is lost).
pub(crate) async fn append_to_session(
    db: &Db,
    session_id: &str,
    new_records: &[AnalysisRecord],
) -> StoreResult<()> {
    if new_records.is_empty() {
        return Ok(());
    }

    log::info!(
        "append_to_session: appending {} records to session {}",
        new_records.len(),
        session_id
    );

    // Transaction 1: write the new records
    let records_js: Vec<JsValue> = new_records.iter().map(|r| to_js(r)).collect();
    let record_count = records_js.len();
    db.transaction(&[RECORDS_STORE])
        .rw()
        .run(move |t| async move {
            let store = t.object_store(RECORDS_STORE)?;
            for (i, val) in records_js.iter().enumerate() {
                store.put(val).await?;
                if (i + 1) % 50 == 0 || i + 1 == record_count {
                    log::info!("append_to_session: {}/{} records written", i + 1, record_count);
                }
            }
            Ok(())
        })
        .await?;

    // Transaction 2: patch the session meta (total_count, success_count, failed_count)
    let sid_new = new_records
        .iter()
        .filter(|r| r.metrics.major_length > 0.0)
        .count() as u32;
    let sid_failed = (new_records.len() as u32).saturating_sub(sid_new);
    let key = JsValue::from_str(session_id);
    db.transaction(&[SESSIONS_STORE])
        .rw()
        .run(move |t| async move {
            let store = t.object_store(SESSIONS_STORE)?;
            if let Some(val) = store.get(&key).await? {
                if let Some(mut meta) = from_js::<SessionMeta>(val) {
                    meta.total_count   += sid_new + sid_failed;
                    meta.success_count += sid_new;
                    meta.failed_count  += sid_failed;
                    store.put(&to_js(&meta)).await?;
                    log::info!(
                        "append_to_session: meta updated — total={}", meta.total_count
                    );
                }
            }
            Ok(())
        })
        .await?;

    Ok(())
}

/// Update a single record (e.g., after editing metrics, toggling suspect, or adding a note).
pub(crate) async fn update_record(
    db: &Db,
    record: &AnalysisRecord,
) -> StoreResult<()> {
    let val = to_js(record);
    db.transaction(&[RECORDS_STORE])
        .rw()
        .run(|t| async move {
            t.object_store(RECORDS_STORE)?.put(&val).await?;
            Ok(())
        })
        .await
}

/// Toggle the starred flag on a session.
pub(crate) async fn toggle_session_star(
    db: &Db,
    session_id: &str,
    starred: bool,
) -> StoreResult<()> {
    let key = JsValue::from_str(session_id);
    db.transaction(&[SESSIONS_STORE])
        .rw()
        .run(move |t| async move {
            let store = t.object_store(SESSIONS_STORE)?;
            if let Some(val) = store.get(&key).await? {
                if let Some(mut meta) = from_js::<SessionMeta>(val) {
                    meta.starred = starred;
                    store.put(&to_js(&meta)).await?;
                }
            }
            Ok(())
        })
        .await
}

/// Rename a session (set a user-assigned display name).
pub(crate) async fn rename_session(
    db: &Db,
    session_id: &str,
    name: Option<String>,
) -> StoreResult<()> {
    let key = JsValue::from_str(session_id);
    db.transaction(&[SESSIONS_STORE])
        .rw()
        .run(move |t| async move {
            let store = t.object_store(SESSIONS_STORE)?;
            if let Some(val) = store.get(&key).await? {
                if let Some(mut meta) = from_js::<SessionMeta>(val) {
                    meta.name = name;
                    store.put(&to_js(&meta)).await?;
                }
            }
            Ok(())
        })
        .await
}

// ──────────────────────── Read Operations ────────────────────────

/// Load all session metadata, ordered by timestamp descending.
pub(crate) async fn load_sessions(db: &Db) -> StoreResult<Vec<SessionMeta>> {
    db.transaction(&[SESSIONS_STORE])
        .run(|t| async move {
            let entries = t.object_store(SESSIONS_STORE)?.get_all(None).await?;
            let mut sessions: Vec<SessionMeta> =
                entries.into_iter().filter_map(from_js).collect();
            // Sort by timestamp descending (newest first)
            sessions.sort_by(|a, b| {
                b.timestamp
                    .partial_cmp(&a.timestamp)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            Ok(sessions)
        })
        .await
}

/// Count suspect records in a particular session.
async fn count_suspects_in_session(
    db: &Db,
    session_id: &str,
) -> StoreResult<u32> {
    let records = load_session_records(db, session_id).await?;
    Ok(records.iter().filter(|r| r.suspect).count() as u32)
}

/// Load all session summaries (metadata + suspect counts).
pub(crate) async fn load_session_summaries(
    db: &Db,
) -> StoreResult<Vec<SessionSummary>> {
    let metas = load_sessions(db).await?;
    let mut summaries = Vec::with_capacity(metas.len());

    for meta in &metas {
        let suspect_count = count_suspects_in_session(db, &meta.session_id).await?;
        summaries.push(SessionSummary::from_meta(meta, suspect_count));
    }

    Ok(summaries)
}

/// Load all records for a single session, using the `session_id` index + cursor.
pub(crate) async fn load_session_records(
    db: &Db,
    session_id: &str,
) -> StoreResult<Vec<AnalysisRecord>> {
    let sid = JsValue::from_str(session_id);
    db.transaction(&[RECORDS_STORE])
        .run(|t| async move {
            let store = t.object_store(RECORDS_STORE)?;
            let index = store.index("session_id")?;
            let mut records = Vec::new();
            let mut cursor = index.cursor().range(sid.clone()..=sid)?.open().await?;
            while let Some(val) = cursor.value() {
                if let Some(r) = from_js::<AnalysisRecord>(val) {
                    records.push(r);
                }
                cursor.advance(1).await?;
            }
            Ok(records)
        })
        .await
}

/// Load records for multiple sessions.
pub(crate) async fn load_records_for_sessions(
    db: &Db,
    session_ids: &[String],
) -> StoreResult<Vec<AnalysisRecord>> {
    let mut all = Vec::new();
    for sid in session_ids {
        let mut records = load_session_records(db, sid).await?;
        all.append(&mut records);
    }
    Ok(all)
}

/// Count total records in the database.
pub(crate) async fn count_records(db: &Db) -> StoreResult<u32> {
    db.transaction(&[RECORDS_STORE])
        .run(|t| async move {
            let count = t.object_store(RECORDS_STORE)?.count().await?;
            Ok(count as u32)
        })
        .await
}

// ──────────────────────── Delete Operations ────────────────────────

/// Result of a cleanup operation.
#[derive(Clone, Debug)]
pub(crate) struct CleanupResult {
    pub sessions_deleted: u32,
    pub records_deleted: u32,
}

/// Delete one or more sessions and all their associated records.
///
/// Each session is deleted atomically: records + session meta in one transaction.
pub(crate) async fn delete_sessions(
    db: &Db,
    session_ids: &[String],
) -> StoreResult<u32> {
    let mut deleted = 0u32;

    for sid in session_ids {
        // First, find all record IDs for this session (read transaction)
        let records = load_session_records(db, sid).await?;
        let record_count = records.len() as u32;

        // Then, delete all records + session meta in one atomic rw transaction
        let record_ids: Vec<String> = records.iter().map(|r| r.id.clone()).collect();
        let sid_owned = sid.clone();
        db.transaction(&[SESSIONS_STORE, RECORDS_STORE])
            .rw()
            .run(|t| async move {
                let store = t.object_store(RECORDS_STORE)?;
                for rid in &record_ids {
                    store.delete(&JsValue::from_str(rid)).await?;
                }
                t.object_store(SESSIONS_STORE)?
                    .delete(&JsValue::from_str(&sid_owned))
                    .await?;
                Ok(())
            })
            .await?;

        deleted += record_count;
    }

    Ok(deleted)
}

// ──────────────────────── Cleanup Operations ────────────────────────

/// One-click cleanup: delete oldest unstarred sessions until record count ≤ 70%.
pub(crate) async fn cleanup_oldest_unstarred(db: &Db) -> StoreResult<CleanupResult> {
    let target = (f64::from(MAX_RECORDS) * CLEANUP_TARGET) as u32;
    let mut current = count_records(db).await?;

    if current <= target {
        return Ok(CleanupResult {
            sessions_deleted: 0,
            records_deleted: 0,
        });
    }

    // Load sessions sorted by timestamp ascending (oldest first)
    let mut sessions = load_sessions(db).await?;
    sessions.sort_by(|a, b| {
        a.timestamp
            .partial_cmp(&b.timestamp)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut total_sessions = 0u32;
    let mut total_records = 0u32;

    for session in &sessions {
        if current <= target {
            break;
        }
        // Skip starred sessions
        if session.starred {
            continue;
        }

        let records = load_session_records(db, &session.session_id).await?;
        let count = records.len() as u32;

        delete_sessions(db, &[session.session_id.clone()]).await?;

        current = current.saturating_sub(count);
        total_sessions += 1;
        total_records += count;
    }

    Ok(CleanupResult {
        sessions_deleted: total_sessions,
        records_deleted: total_records,
    })
}

/// Count unstarred sessions (for cache warning messages).
pub(crate) async fn count_unstarred_sessions(db: &Db) -> StoreResult<u32> {
    let sessions = load_sessions(db).await?;
    Ok(sessions.iter().filter(|s| !s.starred).count() as u32)
}

/// Check the current cache status and return appropriate warning level.
pub(crate) async fn check_cache_status(db: &Db) -> StoreResult<CacheWarningLevel> {
    let current = count_records(db).await?;
    let ratio = f64::from(current) / f64::from(MAX_RECORDS);

    if ratio >= 1.0 {
        Ok(CacheWarningLevel::Full { current })
    } else if ratio >= CRITICAL_THRESHOLD {
        let cleanable = count_unstarred_sessions(db).await?;
        Ok(CacheWarningLevel::Warning {
            current,
            cleanable_sessions: cleanable,
        })
    } else if ratio >= WARN_THRESHOLD {
        let cleanable = count_unstarred_sessions(db).await?;
        Ok(CacheWarningLevel::Caution {
            current,
            cleanable_sessions: cleanable,
        })
    } else {
        Ok(CacheWarningLevel::Ok)
    }
}

/// Generate a simple unique ID (timestamp + random suffix).
pub(crate) fn generate_id() -> String {
    let ts = js_sys::Date::now() as u64;
    let rand = (js_sys::Math::random() * 1_000_000.0) as u64;
    format!("{ts}-{rand}")
}
