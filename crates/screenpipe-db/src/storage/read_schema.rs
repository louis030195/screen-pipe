// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

//! Resident read indexes and response revocation, installed through the
//! existing writer during startup. Older hybrid generations upgrade in place.

use super::{storage_error, HybridStorage};
use sqlx::{Connection, Row, SqliteConnection};

const REVOCATION: &str = r#"
CREATE TABLE IF NOT EXISTS _storage_revocation(id INTEGER PRIMARY KEY CHECK(id=1),revision INTEGER NOT NULL);
INSERT OR IGNORE INTO _storage_revocation VALUES(1,0);
CREATE TRIGGER IF NOT EXISTS hybrid_read_revoke_policy AFTER UPDATE OF policy,required_surfaces ON storage_metadata
WHEN NEW.policy IS NOT OLD.policy OR NEW.required_surfaces IS NOT OLD.required_surfaces
BEGIN UPDATE _storage_revocation SET revision=revision+1; END;
CREATE TRIGGER IF NOT EXISTS hybrid_read_revoke_payload AFTER UPDATE OF generation ON frame_payloads
WHEN (SELECT maintenance FROM storage_metadata)=1 AND NEW.generation!=OLD.generation
AND (NEW.completed_surfaces!=0 OR NEW.policy IS NOT OLD.policy)
BEGIN UPDATE _storage_revocation SET revision=revision+1; END;
"#;

const LOOKUP: &str = r#"
CREATE TABLE IF NOT EXISTS _bulk_element_lookup(id INTEGER PRIMARY KEY,frame_id INTEGER NOT NULL,sort_order INTEGER,kind_id INTEGER NOT NULL,on_screen INTEGER);
CREATE VIEW IF NOT EXISTS _bulk_element_search AS SELECT e.id,e.frame_id,e.sort_order,e.on_screen,k.source,k.role FROM _bulk_element_lookup e JOIN _bulk_element_kinds k ON k.id=e.kind_id;
CREATE TABLE IF NOT EXISTS _storage_read_upgrade(id INTEGER PRIMARY KEY CHECK(id=1),last_id INTEGER);
INSERT OR IGNORE INTO _storage_read_upgrade VALUES(1,NULL);
CREATE TRIGGER IF NOT EXISTS hybrid_read_revoke_element AFTER INSERT ON _bulk_element_rows
WHEN NEW._archive_generation>1
BEGIN UPDATE _storage_revocation SET revision=revision+1; END;
"#;

pub(crate) async fn upgrade(
    conn: &mut SqliteConnection,
    storage: &HybridStorage,
) -> Result<(), sqlx::Error> {
    use sha2::{Digest, Sha256};
    let checksum = format!(
        "{:x}",
        Sha256::digest(format!(
            "{REVOCATION}{LOOKUP}:resident-delete-and-redacted-update-v1"
        ))
    );
    let installed: Option<String> =
        sqlx::query_scalar("SELECT checksum FROM _hybrid_migrations WHERE version=3")
            .fetch_optional(&mut *conn)
            .await?;
    if let Some(installed) = installed {
        return if installed == checksum {
            Ok(())
        } else {
            Err(storage_error("read schema checksum mismatch"))
        };
    }
    let mut tx = conn.begin().await?;
    sqlx::raw_sql(REVOCATION).execute(&mut *tx).await?;
    // Every resident logical deletion revokes a prepared response. Privacy
    // updates also revoke when the redaction marker was already populated.
    let tables: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_list WHERE schema='main' AND type='table' AND substr(name,1,1)!='_' AND name NOT LIKE 'sqlite_%' AND name NOT IN ('sqlite_sequence','storage_metadata','payload_files','frame_payloads','upload_bindings')")
        .fetch_all(&mut *tx).await?;
    for table in tables {
        let columns: Vec<String> = sqlx::query_scalar("SELECT name FROM pragma_table_info(?)")
            .bind(&table)
            .fetch_all(&mut *tx)
            .await?;
        let quoted = format!("\"{}\"", table.replace('"', "\"\""));
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "CREATE TRIGGER IF NOT EXISTS \"hybrid_read_revoke_delete_{table}\" AFTER DELETE ON {quoted} BEGIN UPDATE _storage_revocation SET revision=revision+1; END;"
        ))).execute(&mut *tx).await?;
        let redacted = columns
            .iter()
            .filter(|c| c.ends_with("redacted_at"))
            .map(|c| format!("OLD.\"{c}\" IS NOT NULL OR NEW.\"{c}\" IS NOT NULL"))
            .collect::<Vec<_>>()
            .join(" OR ");
        if !redacted.is_empty() {
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                "CREATE TRIGGER IF NOT EXISTS \"hybrid_read_revoke_redaction_{table}\" AFTER UPDATE ON {quoted} WHEN (SELECT maintenance=0 FROM storage_metadata) AND ({redacted}) BEGIN UPDATE _storage_revocation SET revision=revision+1; END;"
            ))).execute(&mut *tx).await?;
        }
    }
    if storage.has_bulk() {
        sqlx::raw_sql(LOOKUP).execute(&mut *tx).await?;
        for event in ["INSERT", "UPDATE"] {
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                "CREATE TRIGGER IF NOT EXISTS hybrid_read_element_lookup_{event} AFTER {event} ON _bulk_element_rows BEGIN
                 DELETE FROM _bulk_element_lookup WHERE id=NEW.id;
                 INSERT INTO _bulk_element_lookup SELECT NEW.id,NEW.frame_id,NEW.sort_order,k.id,NEW.on_screen FROM _bulk_element_kinds k WHERE k.source=NEW.source AND k.role=NEW.role AND NEW._archive_deleted=0;
                 END;"
            ))).execute(&mut *tx).await?;
        }
    }
    tx.commit().await?;
    if storage.has_bulk() {
        loop {
            let after: Option<i64> =
                sqlx::query_scalar("SELECT last_id FROM _storage_read_upgrade WHERE id=1")
                    .fetch_one(&mut *conn)
                    .await?;
            // Only the compact metadata columns are decoded, never text or
            // geometry. Primary-key windows bound memory and WAL on large DBs.
            let lower = after.map_or_else(|| "1".to_owned(), |id| format!("e.id>{id}"));
            let rows = sqlx::query(sqlx::AssertSqlSafe(format!(
                "SELECT e.id,e.frame_id,e.sort_order,k.id AS kind_id,e.on_screen FROM elements e CROSS JOIN _bulk_element_kinds k WHERE {lower} AND k.source=e.source AND k.role=e.role ORDER BY e.id LIMIT 4096"
            ))).fetch_all(&mut *conn).await?;
            if rows.is_empty() {
                break;
            }
            let last: i64 = rows.last().unwrap().try_get("id")?;
            let mut tx = conn.begin().await?;
            let mut insert = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
                "INSERT OR REPLACE INTO _bulk_element_lookup ",
            );
            insert.push_values(&rows, |mut row, value| {
                row.push_bind(value.get::<i64, _>("id"))
                    .push_bind(value.get::<i64, _>("frame_id"))
                    .push_bind(value.get::<Option<i64>, _>("sort_order"))
                    .push_bind(value.get::<i64, _>("kind_id"))
                    .push_bind(value.get::<Option<i64>, _>("on_screen"));
            });
            insert.build().execute(&mut *tx).await?;
            sqlx::query("UPDATE _storage_read_upgrade SET last_id=? WHERE id=1")
                .bind(last)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            super::schema::construction_checkpoint(conn).await?;
            tracing::info!(
                last_element_id = last,
                "building Parquet element lookup index"
            );
        }
    }
    let mut tx = conn.begin().await?;
    sqlx::query("INSERT INTO _hybrid_migrations VALUES(3,?)")
        .bind(checksum)
        .execute(&mut *tx)
        .await?;
    if storage.has_bulk() {
        sqlx::query("DROP TABLE _storage_read_upgrade")
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(())
}
