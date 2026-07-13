//! Reusable tamper-evident hash-chain helpers.
//!
//! Both the audit log and each repo's event log are append-only SHA-256 hash
//! chains with a persisted **head checkpoint** (tip hash + length) so tail
//! truncation / rollback to an earlier prefix is detectable. That checkpoint
//! logic was independently forgotten on two chains during review, so it lives
//! here once: any chain that appends via [`write_head`] and verifies via
//! [`verify`] gets truncation detection for free. Only the per-row hash
//! recomputation stays table-specific (passed as a closure to [`verify`]).

use anyhow::{Context, Result};
use rusqlite::{Connection, OptionalExtension, Params, Row, ToSql};

/// Locates one of the two chain head-checkpoint rows. The SQL identifiers are
/// encoded in the variants instead of accepted as strings, so callers can only
/// provide the bound key value and can never influence query structure.
pub enum HeadRef<'a> {
    Audit(&'a dyn ToSql),
    Event(&'a dyn ToSql),
}

/// Read the chain head (tip hash + length), or the genesis default `("", 0)`.
pub fn read_head(conn: &Connection, head: &HeadRef) -> Result<(String, i64)> {
    let (sql, key) = match head {
        HeadRef::Audit(key) => (
            "SELECT head_hash, entry_count FROM audit_head WHERE id = ?1",
            *key,
        ),
        HeadRef::Event(key) => (
            "SELECT head_hash, entry_count FROM event_head WHERE repo_id = ?1",
            *key,
        ),
    };
    let row = conn
        .query_row(sql, rusqlite::params![key], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
        })
        .optional()
        .context("failed to read chain head")?;
    Ok(row.unwrap_or_else(|| (String::new(), 0)))
}

/// Upsert the chain head checkpoint.
pub fn write_head(
    conn: &Connection,
    head: &HeadRef,
    head_hash: &str,
    entry_count: i64,
) -> Result<()> {
    let (sql, key) = match head {
        HeadRef::Audit(key) => (
            "INSERT INTO audit_head (id, head_hash, entry_count) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET head_hash = excluded.head_hash,
                                           entry_count = excluded.entry_count",
            *key,
        ),
        HeadRef::Event(key) => (
            "INSERT INTO event_head (repo_id, head_hash, entry_count) VALUES (?1, ?2, ?3)
             ON CONFLICT(repo_id) DO UPDATE SET head_hash = excluded.head_hash,
                                                entry_count = excluded.entry_count",
            *key,
        ),
    };
    conn.execute(sql, rusqlite::params![key, head_hash, entry_count])
        .context("failed to update chain head")?;
    Ok(())
}

/// Walk an ordered set of chain rows, verifying prev-linkage and per-row hash,
/// then compare the walked length/tip against the persisted head checkpoint.
///
/// `recompute` maps each row to `(prev_hash, stored_entry_hash, recomputed_hash)`.
/// `base` seeds the walk with `(prev_hash, count)` from a sealed checkpoint — a
/// pruned prefix's tip hash and how many entries it covered. Pass `("", 0)` to
/// verify a full chain from genesis. The first retained row must link to the
/// checkpoint and `base_count + retained == head length`, so truncation stays
/// detectable across a seal.
/// Returns `false` on any break in the chain (bad link, hash mismatch, or a
/// head/length mismatch indicating truncation).
pub fn verify<P, F>(
    conn: &Connection,
    rows_sql: &str,
    rows_params: P,
    head: &HeadRef,
    base: (&str, i64),
    recompute: F,
) -> Result<bool>
where
    P: Params,
    F: Fn(&Row) -> Result<(String, String, String)>,
{
    let (base_prev, base_count) = base;
    let mut stmt = conn.prepare(rows_sql)?;
    let mut rows = stmt.query(rows_params)?;
    let mut expected_prev = base_prev.to_string();
    let mut counted: i64 = base_count;
    while let Some(row) = rows.next()? {
        let (prev_hash, stored_hash, recomputed) = recompute(row)?;
        if prev_hash != expected_prev || recomputed != stored_hash {
            return Ok(false);
        }
        expected_prev = stored_hash;
        counted += 1;
    }
    let (head_hash, entry_count) = read_head(conn, head)?;
    Ok(counted == entry_count && expected_prev == head_hash)
}
