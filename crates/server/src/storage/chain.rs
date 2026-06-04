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

/// Locates a chain's single head-checkpoint row: the table and its key
/// column/value (`audit_head` keyed by `id=1`; `event_head` keyed by `repo_id`).
pub struct HeadRef<'a> {
    pub table: &'a str,
    pub key_col: &'a str,
    pub key: &'a dyn ToSql,
}

/// Guard against accidental SQL identifier injection: table/column names are
/// always internal constants, never user input.
fn assert_ident(head: &HeadRef) {
    debug_assert!(
        matches!(head.table, "audit_head" | "event_head"),
        "unexpected head table"
    );
    debug_assert!(
        matches!(head.key_col, "id" | "repo_id"),
        "unexpected head key column"
    );
}

/// Read the chain head (tip hash + length), or the genesis default `("", 0)`.
pub fn read_head(conn: &Connection, head: &HeadRef) -> Result<(String, i64)> {
    assert_ident(head);
    let sql = format!(
        "SELECT head_hash, entry_count FROM {} WHERE {} = ?1",
        head.table, head.key_col
    );
    let row = conn
        .query_row(&sql, rusqlite::params![head.key], |r| {
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
    assert_ident(head);
    let sql = format!(
        "INSERT INTO {0} ({1}, head_hash, entry_count) VALUES (?1, ?2, ?3)
         ON CONFLICT({1}) DO UPDATE SET head_hash = excluded.head_hash,
                                        entry_count = excluded.entry_count",
        head.table, head.key_col
    );
    conn.execute(&sql, rusqlite::params![head.key, head_hash, entry_count])
        .context("failed to update chain head")?;
    Ok(())
}

/// Walk an ordered set of chain rows, verifying prev-linkage and per-row hash,
/// then compare the walked length/tip against the persisted head checkpoint.
///
/// `recompute` maps each row to `(prev_hash, stored_entry_hash, recomputed_hash)`.
/// Returns `false` on any break in the chain (bad link, hash mismatch, or a
/// head/length mismatch indicating truncation).
pub fn verify<P, F>(
    conn: &Connection,
    rows_sql: &str,
    rows_params: P,
    head: &HeadRef,
    recompute: F,
) -> Result<bool>
where
    P: Params,
    F: Fn(&Row) -> Result<(String, String, String)>,
{
    let mut stmt = conn.prepare(rows_sql)?;
    let mut rows = stmt.query(rows_params)?;
    let mut expected_prev = String::new();
    let mut counted: i64 = 0;
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
