// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

//! Element history uses ordered Parquet ranges. SQLite holds staged changes,
//! per-frame range references, relationship counts, and the live FTS index.

mod lifecycle;
mod vtab;

use super::{integer, real, text, Column, Kind, Table};
use sqlx::{Row, SqliteConnection};

pub(super) use lifecycle::{export, reclaim, seal, verify};
pub(crate) use vtab::register;

pub(super) const TABLE: Table = Table {
    name: "elements",
    columns: &[
        integer("frame_id"), text("source"), text("role"), text("text"),
        integer("parent_id"), integer("depth"), real("left_bound"),
        real("top_bound"), real("width_bound"), real("height_bound"),
        real("confidence"), integer("sort_order"), text("properties"),
        integer("on_screen"), integer("redacted_at"),
    ],
    eligible: "redacted_at IS NOT NULL OR (SELECT required_surfaces=0 FROM storage_metadata) OR (COALESCE(text,'')='' AND properties IS NULL)",
    fts: &["text", "role", "frame_id UNINDEXED"],
    fts_condition: "text IS NOT NULL AND text!=''",
};

pub(super) fn names() -> String {
    TABLE
        .columns
        .iter()
        .map(|c| c.name)
        .collect::<Vec<_>>()
        .join(",")
}

pub(super) fn declaration() -> String {
    TABLE
        .columns
        .iter()
        .map(|Column { name, kind, .. }| {
            let ty = match kind {
                Kind::Text => "TEXT",
                Kind::Integer => "INTEGER",
                Kind::Real => "REAL",
            };
            let default = if matches!(*name, "depth" | "sort_order") {
                " DEFAULT 0"
            } else {
                ""
            };
            format!("{name} {ty}{default}")
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Register an offline batch whose complete records are already staged. Parent
/// references may point into a later batch; their original identities remain
/// intact across those batch boundaries.
pub(crate) async fn import_batch(
    conn: &mut SqliteConnection,
    first: i64,
    last: i64,
) -> Result<(), sqlx::Error> {
    let range = "id BETWEEN ?1 AND ?2";
    for statement in [
        format!("INSERT OR IGNORE INTO _bulk_element_kinds(source,role) SELECT DISTINCT source,role FROM _bulk_element_rows WHERE {range}"),
        format!("INSERT OR REPLACE INTO _bulk_element_lookup SELECT e.id,e.frame_id,e.sort_order,k.id,e.on_screen FROM _bulk_element_rows e JOIN _bulk_element_kinds k ON k.source=e.source AND k.role=e.role WHERE e.{range} AND e._archive_deleted=0"),
        format!("INSERT INTO _bulk_element_groups SELECT e.frame_id,k.id,COALESCE(e.on_screen,0),e.on_screen IS NULL,count(*) FROM _bulk_element_rows e JOIN _bulk_element_kinds k ON k.source=e.source AND k.role=e.role WHERE e.{range} GROUP BY e.frame_id,k.id,COALESCE(e.on_screen,0),e.on_screen IS NULL ON CONFLICT(frame_id,kind_id,visibility,is_null) DO UPDATE SET rows=rows+excluded.rows"),
        format!("INSERT INTO _bulk_element_parent_refs SELECT parent_id,count(*) FROM _bulk_element_rows WHERE {range} AND parent_id IS NOT NULL GROUP BY parent_id ON CONFLICT(parent_id) DO UPDATE SET rows=rows+excluded.rows"),
        format!("INSERT INTO elements_fts(rowid,text,role,frame_id) SELECT id,text,role,frame_id FROM _bulk_element_rows WHERE {range} AND text IS NOT NULL AND text!=''"),
        format!("UPDATE storage_metadata SET staging_bytes=staging_bytes+COALESCE((SELECT SUM({}) FROM _bulk_element_rows WHERE {range}),0),revision=revision+1", TABLE.all_bytes("")),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(statement)).bind(first).bind(last)
            .execute(&mut *conn).await?;
    }
    sqlx::query("UPDATE _bulk_element_state SET version=version+1")
        .execute(&mut *conn)
        .await?;
    Ok(())
}

pub(super) async fn bootstrap(conn: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    sqlx::raw_sql("CREATE TABLE _bulk_element_kinds(id INTEGER PRIMARY KEY,source TEXT NOT NULL,role TEXT NOT NULL,UNIQUE(source,role));").execute(&mut *conn).await?;
    let seq: Option<(i64, i64)> =
        sqlx::query_as("SELECT rowid,seq FROM sqlite_sequence WHERE name='elements'")
            .fetch_optional(&mut *conn)
            .await?;
    let legacy = sqlx::query("SELECT type,name,sql FROM sqlite_master WHERE (tbl_name='elements' AND type IN ('table','index','trigger') AND sql IS NOT NULL) OR name='elements_fts'").fetch_all(&mut *conn).await?;
    for row in &legacy {
        let name: &str = row.try_get("name")?;
        if name.starts_with("hybrid_revision_") {
            continue;
        }
        sqlx::query("INSERT INTO _bulk_legacy_sql VALUES(?,?,?)")
            .bind(row.try_get::<&str, _>("type")?)
            .bind(name)
            .bind(row.try_get::<&str, _>("sql")?)
            .execute(&mut *conn)
            .await?;
        crate::storage::schema::construction_checkpoint(conn).await?;
    }
    // Triggers and indexes are rebuilt over the staged/archived row source.
    for row in &legacy {
        let kind: &str = row.try_get("type")?;
        let name: &str = row.try_get("name")?;
        if kind != "table" {
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                "DROP {kind} \"{}\"",
                name.replace('"', "\"\"")
            )))
            .execute(&mut *conn)
            .await?;
            crate::storage::schema::construction_checkpoint(conn).await?;
        }
    }
    crate::storage::schema::construction_sql(
        conn,
        "DROP TABLE elements_fts; ALTER TABLE elements RENAME TO _bulk_elements_source;",
    )
    .await?;
    let columns = names();
    crate::storage::schema::construction_sql(conn, &format!(
        "CREATE TABLE _bulk_element_rows(id INTEGER PRIMARY KEY,{},_archive_generation INTEGER NOT NULL,_archive_deleted INTEGER NOT NULL DEFAULT 0,_archive_file INTEGER REFERENCES _bulk_files(id));",declaration())).await?;
    crate::storage::schema::construction_rows(conn, &format!(
        "INSERT INTO _bulk_element_rows(id,{columns},_archive_generation) SELECT id,{columns},1 FROM _bulk_elements_source"), "_bulk_elements_source", super::FILE_ROWS).await?;
    crate::storage::faults::checkpoint("migration_elements_copied");
    // The unpublished staged copy owns every record. Removing its temporary
    // predecessor bypasses per-row self-FK deletion scans; complete relationship
    // verification checks the constructed candidate before activation.
    crate::storage::schema::construction_sql(
        conn,
        "PRAGMA foreign_keys=OFF; DROP TABLE _bulk_elements_source; PRAGMA foreign_keys=ON;",
    )
    .await?;
    crate::storage::schema::construction_sql(conn, &format!(
        "CREATE INDEX _bulk_element_staged_frame ON _bulk_element_rows(frame_id);
         CREATE INDEX _bulk_element_staged_file ON _bulk_element_rows(_archive_file) WHERE _archive_file IS NOT NULL;
         CREATE TABLE _bulk_element_ranges(first_id INTEGER PRIMARY KEY,last_id INTEGER NOT NULL,file_id INTEGER NOT NULL UNIQUE REFERENCES _bulk_files(id));
         CREATE TABLE _bulk_element_frames(frame_id INTEGER NOT NULL,file_id INTEGER NOT NULL REFERENCES _bulk_files(id),PRIMARY KEY(frame_id,file_id));
         CREATE INDEX _bulk_element_frames_file ON _bulk_element_frames(file_id);
         INSERT INTO _bulk_element_kinds(source,role) SELECT DISTINCT source,role FROM _bulk_element_rows;
         CREATE TABLE _bulk_element_groups(frame_id INTEGER NOT NULL REFERENCES frames(id),kind_id INTEGER NOT NULL REFERENCES _bulk_element_kinds(id),visibility INTEGER NOT NULL,is_null INTEGER NOT NULL,rows INTEGER NOT NULL,PRIMARY KEY(frame_id,kind_id,visibility,is_null));
         INSERT INTO _bulk_element_groups SELECT e.frame_id,k.id,COALESCE(e.on_screen,0),e.on_screen IS NULL,count(*) FROM _bulk_element_rows e JOIN _bulk_element_kinds k ON k.source=e.source AND k.role=e.role GROUP BY e.frame_id,k.id,COALESCE(e.on_screen,0),e.on_screen IS NULL;
         CREATE VIEW _bulk_element_counts AS SELECT g.frame_id,k.source,k.role,CASE WHEN g.is_null=1 THEN NULL ELSE g.visibility END AS on_screen,g.rows FROM _bulk_element_groups g JOIN _bulk_element_kinds k ON k.id=g.kind_id;
         CREATE TABLE _bulk_element_checks(parent_id INTEGER PRIMARY KEY);
         CREATE TABLE _bulk_element_parent_refs(parent_id INTEGER PRIMARY KEY,rows INTEGER NOT NULL);
         INSERT INTO _bulk_element_parent_refs SELECT parent_id,count(*) FROM _bulk_element_rows WHERE parent_id IS NOT NULL GROUP BY parent_id;
         CREATE TABLE _bulk_element_state(id INTEGER PRIMARY KEY CHECK(id=1),version INTEGER NOT NULL);
         INSERT INTO _bulk_element_state VALUES(1,1);
         CREATE VIRTUAL TABLE elements USING screenpipe_elements;
         CREATE VIEW _bulk_logical_elements AS SELECT * FROM elements;
         CREATE VIEW _bulk_element_search_content AS SELECT id,text,role,frame_id FROM elements WHERE text IS NOT NULL AND text!='';
         CREATE VIRTUAL TABLE elements_fts USING fts5(text,role,frame_id UNINDEXED,content='_bulk_element_search_content',content_rowid='id',columnsize=0,tokenize='unicode61');
         INSERT INTO elements_fts(rowid,text,role,frame_id) SELECT id,text,role,frame_id FROM _bulk_element_rows WHERE text IS NOT NULL AND text!='';
         UPDATE storage_metadata SET staging_bytes=staging_bytes+COALESCE((SELECT SUM({bytes}) FROM _bulk_element_rows),0);",
        bytes=TABLE.all_bytes(""))).await?;
    if let Some((rowid, seq)) = seq {
        sqlx::query("INSERT INTO sqlite_sequence(rowid,name,seq) VALUES(?,'elements',?)")
            .bind(rowid)
            .bind(seq)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}
