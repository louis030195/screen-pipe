// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use super::{storage_error, StorageDescriptor};
use sha2::{Digest, Sha256};
use sqlx::{Connection, Row, SqliteConnection};

/// Execute trusted construction statements while the offline lifecycle owns
/// the writer permit. Callers supply simple DDL/DML, without trigger bodies or
/// semicolons inside literals. Each completed statement releases its WAL pages
/// before the next operation; normal capture keeps its shared checkpoint policy.
pub(super) async fn construction_sql(
    conn: &mut SqliteConnection,
    sql: &str,
) -> Result<(), sqlx::Error> {
    for statement in sql.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        sqlx::raw_sql(sqlx::AssertSqlSafe(statement))
            .execute(&mut *conn)
            .await?;
        construction_checkpoint(conn).await?;
        super::faults::checkpoint("migration_schema_step");
    }
    Ok(())
}

pub(super) async fn construction_checkpoint(
    conn: &mut SqliteConnection,
) -> Result<(), sqlx::Error> {
    // In-place schema steps commit their DDL and resume marker together. Their
    // caller checkpoints after commit, never inside the schema transaction.
    let mut handle = conn.lock_handle().await?;
    let in_transaction =
        unsafe { libsqlite3_sys::sqlite3_get_autocommit(handle.as_raw_handle().as_ptr()) == 0 };
    drop(handle);
    if in_transaction {
        return Ok(());
    }
    // Construction keeps pools open between batches. Copy committed frames
    // without requesting a WAL restart under those connections.
    let row = sqlx::query("PRAGMA wal_checkpoint(PASSIVE)")
        .fetch_one(&mut *conn)
        .await?;
    if row.try_get::<i64, _>(0)? != 0 || row.try_get::<i64, _>(1)? != row.try_get::<i64, _>(2)? {
        return Err(storage_error("offline construction has an active reader"));
    }
    Ok(())
}

pub(super) async fn converted_step(
    conn: &mut SqliteConnection,
    step: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM _storage_conversion_steps WHERE step=?)")
        .bind(step)
        .fetch_one(conn)
        .await
}

pub(super) async fn finish_step(
    conn: &mut SqliteConnection,
    step: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO _storage_conversion_steps(step) VALUES(?)")
        .bind(step)
        .execute(conn)
        .await?;
    Ok(())
}

/// Install only the catalog/trigger definitions. Historical payloads are
/// registered and sealed one batch at a time by the offline conversion loop.
pub(super) async fn bootstrap_in_place(
    conn: &mut SqliteConnection,
    descriptor: &StorageDescriptor,
) -> Result<(), sqlx::Error> {
    if !converted_step(conn, "frames-schema").await? {
        let mut tx = conn.begin().await?;
        for statement in CATALOG.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            if statement.starts_with("INSERT INTO frames_fts")
                || statement.starts_with("UPDATE frames SET")
            {
                continue;
            }
            sqlx::raw_sql(sqlx::AssertSqlSafe(statement))
                .execute(&mut *tx)
                .await?;
        }
        sqlx::query("INSERT INTO storage_metadata(singleton,descriptor,staging_limit,record_limit,policy,required_surfaces,writer_version) VALUES(1,?,?,?,?,?,?)")
            .bind(serde_json::to_string(descriptor).map_err(storage_error)?)
            .bind(descriptor.budget.staging_bytes as i64).bind(descriptor.budget.record_bytes as i64)
            .bind(&descriptor.privacy.identity).bind(descriptor.privacy.required_surfaces as i64)
            .bind(env!("CARGO_PKG_VERSION")).execute(&mut *tx).await?;
        sqlx::raw_sql(sqlx::AssertSqlSafe(triggers()))
            .execute(&mut *tx)
            .await?;
        let tables: Vec<String> = sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '%_fts%' AND substr(name,1,1)!='_' AND name NOT IN ('frames','frame_payloads','payload_files','storage_metadata','upload_bindings') AND sql NOT LIKE 'CREATE VIRTUAL TABLE%'").fetch_all(&mut *tx).await?;
        for table in tables {
            for event in ["INSERT", "UPDATE", "DELETE"] {
                let name = table.replace('"', "\"\"");
                sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE TRIGGER \"hybrid_revision_{name}_{event}\" AFTER {event} ON \"{name}\" BEGIN UPDATE storage_metadata SET revision=revision+1; END;"))).execute(&mut *tx).await?;
            }
        }
        sqlx::query("INSERT INTO _hybrid_migrations VALUES(1,?)")
            .bind(format!(
                "{:x}",
                Sha256::digest(format!("{CATALOG}{}", triggers()))
            ))
            .execute(&mut *tx)
            .await?;
        finish_step(&mut tx, "frames-schema").await?;
        tx.commit().await?;
        construction_checkpoint(conn).await?;
        super::faults::checkpoint("migration_schema_step");
    }
    super::bulk::bootstrap_in_place(conn).await
}

pub(super) async fn stage_frames(
    conn: &mut SqliteConnection,
    first: i64,
    last: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE storage_metadata SET maintenance=1")
        .execute(&mut *conn)
        .await?;
    let flags = CATALOG
        .rsplit_once("UPDATE frames SET ")
        .unwrap()
        .1
        .trim()
        .trim_end_matches(';');
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE frames SET {flags} WHERE id BETWEEN ? AND ?"
    )))
    .bind(first)
    .bind(last)
    .execute(&mut *conn)
    .await?;
    sqlx::query(sqlx::AssertSqlSafe(format!("INSERT INTO frame_payloads(frame_id,generation,state,bytes,policy,completed_surfaces) SELECT NEW.id,1,'staged',({BYTES}),?,{EMPTY_SURFACES} FROM frames NEW WHERE NEW.id BETWEEN ? AND ?")))
        .bind(sqlx::query_scalar::<_, String>("SELECT policy FROM storage_metadata").fetch_one(&mut *conn).await?)
        .bind(first).bind(last).execute(&mut *conn).await?;
    sqlx::query("INSERT INTO frames_fts(rowid,full_text,app_name,window_name,browser_url) SELECT id,full_text,COALESCE(app_name,''),COALESCE(window_name,''),COALESCE(browser_url,'') FROM frames WHERE id BETWEEN ? AND ? AND full_text IS NOT NULL AND full_text!=''")
        .bind(first).bind(last).execute(&mut *conn).await?;
    sqlx::query("UPDATE storage_metadata SET maintenance=0,staging_bytes=staging_bytes+COALESCE((SELECT sum(bytes) FROM frame_payloads WHERE frame_id BETWEEN ? AND ?),0)").bind(first).bind(last).execute(conn).await?;
    Ok(())
}

/// Backfill by primary-key windows so construction WAL is reused between
/// batches. Both statements are static SQL supplied by schema construction.
pub(super) async fn construction_rows(
    conn: &mut SqliteConnection,
    statement: &str,
    source: &str,
    batch_rows: usize,
) -> Result<(), sqlx::Error> {
    let mut after = None;
    loop {
        let lower = after.map_or_else(|| "1".to_owned(), |id: i64| format!("id>{id}"));
        let last: Option<i64> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT max(id) FROM (SELECT id FROM {source} WHERE {lower} ORDER BY id LIMIT {batch_rows})"
        )))
        .fetch_one(&mut *conn)
        .await?;
        let Some(last) = last else { break };
        construction_sql(conn, &format!("{statement} WHERE {lower} AND id<={last}")).await?;
        after = Some(last);
    }
    Ok(())
}

pub(super) const LEGACY_FTS: &str =
    include_str!("../migrations/20260415000000_frames_fts_external_content.sql");

pub(super) fn hybrid_fts_schema() -> &'static str {
    let start = CATALOG.find("CREATE VIRTUAL TABLE frames_fts").unwrap();
    &CATALOG[start..start + CATALOG[start..].find(';').unwrap()]
}

const CATALOG: &str = r#"
CREATE TABLE _hybrid_migrations(version INTEGER PRIMARY KEY, checksum TEXT NOT NULL);
CREATE TABLE storage_metadata (
    singleton INTEGER PRIMARY KEY CHECK(singleton=1), descriptor TEXT NOT NULL,
    revision INTEGER NOT NULL DEFAULT 0, maintenance INTEGER NOT NULL DEFAULT 0,
    staging_bytes INTEGER NOT NULL DEFAULT 0, staging_limit INTEGER NOT NULL,
    record_limit INTEGER NOT NULL, policy TEXT NOT NULL, required_surfaces INTEGER NOT NULL,
    writer_version TEXT NOT NULL
);
CREATE TABLE upload_bindings (
    destination TEXT PRIMARY KEY, checkpoint TEXT NOT NULL
);
CREATE TABLE payload_files (
    id TEXT PRIMARY KEY, search_path TEXT NOT NULL UNIQUE, detail_path TEXT NOT NULL UNIQUE,
    search_checksum TEXT, detail_checksum TEXT, schema_version INTEGER NOT NULL,
    state TEXT NOT NULL CHECK(state IN ('encoding','published','dirty','retired')),
    row_count INTEGER NOT NULL, created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE TABLE frame_payloads (
    frame_id INTEGER PRIMARY KEY REFERENCES frames(id) ON DELETE CASCADE,
    generation INTEGER NOT NULL CHECK(generation > 0),
    state TEXT NOT NULL CHECK(state IN ('staged','sealed')),
    file_id TEXT REFERENCES payload_files(id), bytes INTEGER NOT NULL,
    policy TEXT NOT NULL, completed_surfaces INTEGER NOT NULL DEFAULT 0,
    capture_version TEXT, archive_writer_version TEXT,
    attempts INTEGER NOT NULL DEFAULT 0, retry_at INTEGER, last_error TEXT,
    CHECK((state='staged' AND file_id IS NULL) OR (state='sealed' AND file_id IS NOT NULL))
);
CREATE INDEX frame_payloads_file ON frame_payloads(file_id);
CREATE INDEX frame_payloads_staging ON frame_payloads(state,frame_id);
ALTER TABLE frames ADD COLUMN payload_full_text_length INTEGER NOT NULL DEFAULT 0;
ALTER TABLE frames ADD COLUMN payload_accessibility_length INTEGER NOT NULL DEFAULT 0;
ALTER TABLE frames ADD COLUMN payload_full_text_present INTEGER NOT NULL DEFAULT 0;
ALTER TABLE frames ADD COLUMN payload_accessibility_present INTEGER NOT NULL DEFAULT 0;
ALTER TABLE frames ADD COLUMN payload_detail_present INTEGER NOT NULL DEFAULT 0;
DROP TRIGGER IF EXISTS frames_ai;
DROP TRIGGER IF EXISTS frames_au;
DROP TRIGGER IF EXISTS frames_ad;
DROP TABLE frames_fts;
CREATE VIRTUAL TABLE frames_fts USING fts5(full_text,app_name,window_name,browser_url,
    content='',contentless_delete=1,tokenize='unicode61');
INSERT INTO frames_fts(rowid,full_text,app_name,window_name,browser_url)
SELECT id,full_text,COALESCE(app_name,''),COALESCE(window_name,''),COALESCE(browser_url,'')
FROM frames WHERE full_text IS NOT NULL AND full_text != '';
UPDATE frames SET payload_full_text_length=length(COALESCE(full_text,'')),
    payload_accessibility_length=length(COALESCE(accessibility_text,'')),
    payload_full_text_present=full_text IS NOT NULL,
    payload_accessibility_present=accessibility_text IS NOT NULL,
    payload_detail_present=accessibility_tree_json IS NOT NULL OR text_json IS NOT NULL;
"#;

const BYTES: &str = "COALESCE(length(CAST(NEW.full_text AS BLOB)),0)+COALESCE(length(CAST(NEW.accessibility_text AS BLOB)),0)+COALESCE(length(CAST(NEW.accessibility_tree_json AS BLOB)),0)+COALESCE(length(CAST(NEW.text_json AS BLOB)),0)";
const EMPTY_SURFACES: &str = "(CASE WHEN COALESCE(NEW.full_text,'')='' THEN 1 ELSE 0 END | CASE WHEN COALESCE(NEW.accessibility_text,'')='' THEN 2 ELSE 0 END | CASE WHEN COALESCE(NEW.accessibility_tree_json,'')='' THEN 4 ELSE 0 END | CASE WHEN COALESCE(NEW.text_json,'')='' THEN 8 ELSE 0 END | CASE WHEN COALESCE(NEW.window_name,'')='' THEN 16 ELSE 0 END | CASE WHEN COALESCE(NEW.browser_url,'')='' THEN 32 ELSE 0 END)";

fn triggers() -> String {
    format!(
        r#"
CREATE TRIGGER hybrid_frame_insert AFTER INSERT ON frames
WHEN (SELECT maintenance=0 FROM storage_metadata WHERE singleton=1)
BEGIN
    SELECT CASE WHEN ({BYTES}) > (SELECT record_limit FROM storage_metadata)
        THEN RAISE(ABORT,'frame storage: record budget exceeded') END;
    SELECT CASE WHEN (SELECT staging_bytes+({BYTES})>staging_limit FROM storage_metadata)
        THEN RAISE(ABORT,'frame storage: staging budget reached; capture admission paused') END;
    INSERT INTO frame_payloads(frame_id,generation,state,bytes,policy,completed_surfaces,capture_version)
    SELECT NEW.id,1,'staged',({BYTES}),policy,{EMPTY_SURFACES},writer_version FROM storage_metadata;
    UPDATE storage_metadata SET staging_bytes=staging_bytes+({BYTES}),revision=revision+1;
    UPDATE frames SET payload_full_text_length=length(COALESCE(NEW.full_text,'')),
        payload_accessibility_length=length(COALESCE(NEW.accessibility_text,'')),
        payload_full_text_present=NEW.full_text IS NOT NULL,
        payload_accessibility_present=NEW.accessibility_text IS NOT NULL,
        payload_detail_present=NEW.accessibility_tree_json IS NOT NULL OR NEW.text_json IS NOT NULL
    WHERE id=NEW.id;
    INSERT INTO frames_fts(rowid,full_text,app_name,window_name,browser_url)
    SELECT NEW.id,NEW.full_text,COALESCE(NEW.app_name,''),COALESCE(NEW.window_name,''),COALESCE(NEW.browser_url,'')
    WHERE NEW.full_text IS NOT NULL AND NEW.full_text != '';
END;
CREATE TRIGGER hybrid_frame_sealed_guard BEFORE UPDATE OF full_text,accessibility_text,accessibility_tree_json,text_json,app_name,window_name,browser_url ON frames
WHEN (SELECT maintenance=0 FROM storage_metadata) AND
     (SELECT state='sealed' FROM frame_payloads WHERE frame_id=OLD.id)
BEGIN SELECT RAISE(ABORT,'frame storage: sealed frame requires coordinated payload replacement'); END;
CREATE TRIGGER hybrid_frame_update AFTER UPDATE OF full_text,accessibility_text,accessibility_tree_json,text_json,app_name,window_name,browser_url ON frames
WHEN (SELECT maintenance=0 FROM storage_metadata)
BEGIN
    SELECT CASE WHEN ({BYTES}) > (SELECT record_limit FROM storage_metadata)
        THEN RAISE(ABORT,'frame storage: record budget exceeded') END;
    UPDATE storage_metadata SET revision=revision+1,
        staging_bytes=staging_bytes+({BYTES})-(SELECT bytes FROM frame_payloads WHERE frame_id=NEW.id);
    SELECT CASE WHEN (SELECT staging_bytes>staging_limit FROM storage_metadata)
        THEN RAISE(ABORT,'frame storage: staging budget reached; capture admission paused') END;
    UPDATE frame_payloads SET generation=generation+1,bytes=({BYTES}),
        policy=(SELECT policy FROM storage_metadata),completed_surfaces={EMPTY_SURFACES},attempts=0,retry_at=NULL,last_error=NULL
    WHERE frame_id=NEW.id;
    UPDATE frames SET payload_full_text_length=length(COALESCE(NEW.full_text,'')),
        payload_accessibility_length=length(COALESCE(NEW.accessibility_text,'')),
        payload_full_text_present=NEW.full_text IS NOT NULL,
        payload_accessibility_present=NEW.accessibility_text IS NOT NULL,
        payload_detail_present=NEW.accessibility_tree_json IS NOT NULL OR NEW.text_json IS NOT NULL
    WHERE id=NEW.id;
    DELETE FROM frames_fts WHERE rowid=OLD.id;
    INSERT INTO frames_fts(rowid,full_text,app_name,window_name,browser_url)
    SELECT NEW.id,NEW.full_text,COALESCE(NEW.app_name,''),COALESCE(NEW.window_name,''),COALESCE(NEW.browser_url,'')
    WHERE NEW.full_text IS NOT NULL AND NEW.full_text != '';
END;
CREATE TRIGGER hybrid_frame_metadata AFTER UPDATE OF timestamp,offset_index,device_name,focused,video_chunk_id,frame_name ON frames
WHEN (SELECT maintenance=0 FROM storage_metadata)
BEGIN UPDATE storage_metadata SET revision=revision+1; END;
CREATE TRIGGER hybrid_frame_delete BEFORE DELETE ON frames
BEGIN
    DELETE FROM frames_fts WHERE rowid=OLD.id;
    UPDATE payload_files SET state='dirty' WHERE id=(SELECT file_id FROM frame_payloads WHERE frame_id=OLD.id);
    UPDATE storage_metadata SET revision=revision+1,staging_bytes=staging_bytes-
        COALESCE((SELECT bytes FROM frame_payloads WHERE frame_id=OLD.id AND state='staged'),0);
    DELETE FROM frame_payloads WHERE frame_id=OLD.id;
END;
"#
    )
}

pub(crate) async fn bootstrap(
    conn: &mut SqliteConnection,
    descriptor: &StorageDescriptor,
) -> Result<(), sqlx::Error> {
    let (catalog, flags) = CATALOG.rsplit_once("UPDATE frames SET ").unwrap();
    construction_sql(conn, catalog).await?;
    construction_rows(
        conn,
        &format!("UPDATE frames SET {}", flags.trim().trim_end_matches(';')),
        "frames",
        128,
    )
    .await?;
    sqlx::query("INSERT INTO storage_metadata(singleton,descriptor,staging_limit,record_limit,policy,required_surfaces,writer_version) VALUES(1,?,?,?,?,?,?)")
        .bind(serde_json::to_string(descriptor).map_err(storage_error)?)
        .bind(descriptor.budget.staging_bytes as i64).bind(descriptor.budget.record_bytes as i64)
        .bind(&descriptor.privacy.identity).bind(descriptor.privacy.required_surfaces as i64)
        .bind(env!("CARGO_PKG_VERSION")).execute(&mut *conn).await?;
    let seed = format!("INSERT INTO frame_payloads(frame_id,generation,state,bytes,policy,completed_surfaces) SELECT NEW.id,1,'staged',({BYTES}),?,{EMPTY_SURFACES} FROM frames NEW");
    sqlx::query(sqlx::AssertSqlSafe(seed))
        .bind(&descriptor.privacy.identity)
        .execute(&mut *conn)
        .await?;
    sqlx::query("UPDATE storage_metadata SET staging_bytes=COALESCE((SELECT SUM(bytes) FROM frame_payloads),0)")
        .execute(&mut *conn).await?;
    sqlx::raw_sql(sqlx::AssertSqlSafe(triggers()))
        .execute(&mut *conn)
        .await?;
    // Resident text and relationships participate in the same read revision.
    let tables: Vec<String> = sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '%_fts%' AND substr(name,1,1)!='_' AND name NOT IN ('frames','frame_payloads','payload_files','storage_metadata','upload_bindings') AND sql NOT LIKE 'CREATE VIRTUAL TABLE%'")
        .fetch_all(&mut *conn).await?;
    for table in tables {
        for event in ["INSERT", "UPDATE", "DELETE"] {
            let sql = format!("CREATE TRIGGER hybrid_revision_{table}_{event} AFTER {event} ON {table} BEGIN UPDATE storage_metadata SET revision=revision+1; END;");
            sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
                .execute(&mut *conn)
                .await?;
        }
    }
    let checksum = format!("{:x}", Sha256::digest(format!("{CATALOG}{}", triggers())));
    sqlx::query("INSERT INTO _hybrid_migrations VALUES(1,?)")
        .bind(checksum)
        .execute(&mut *conn)
        .await?;
    if descriptor
        .capabilities
        .iter()
        .any(|s| s == super::bulk::CAPABILITY)
    {
        super::bulk::bootstrap(conn).await?;
    }
    Ok(())
}

pub(crate) async fn verify(
    conn: &mut SqliteConnection,
    descriptor: &StorageDescriptor,
) -> Result<(), sqlx::Error> {
    let row = sqlx::query("SELECT descriptor,maintenance FROM storage_metadata WHERE singleton=1")
        .fetch_one(&mut *conn)
        .await?;
    let stored: StorageDescriptor =
        serde_json::from_str(row.try_get("descriptor")?).map_err(storage_error)?;
    if &stored != descriptor || row.try_get::<i64, _>("maintenance")? != 0 {
        return Err(storage_error("descriptor/catalog identity mismatch"));
    }
    let checksum: String =
        sqlx::query_scalar("SELECT checksum FROM _hybrid_migrations WHERE version=1")
            .fetch_one(&mut *conn)
            .await?;
    if checksum != format!("{:x}", Sha256::digest(format!("{CATALOG}{}", triggers()))) {
        return Err(storage_error("hybrid schema checksum mismatch"));
    }
    if descriptor
        .capabilities
        .iter()
        .any(|s| s == super::bulk::CAPABILITY)
    {
        let checksum: String =
            sqlx::query_scalar("SELECT checksum FROM _hybrid_migrations WHERE version=2")
                .fetch_one(&mut *conn)
                .await?;
        if checksum != super::bulk::manifest_checksum() {
            return Err(storage_error("bulk schema checksum mismatch"));
        }
    }
    Ok(())
}
