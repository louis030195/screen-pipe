// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use super::{Table, TABLES};
use sqlx::{Row, SqliteConnection};

pub(crate) async fn bootstrap(conn: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    sqlx::raw_sql("CREATE TABLE _bulk_files(id INTEGER PRIMARY KEY,table_name TEXT NOT NULL,path TEXT NOT NULL UNIQUE,checksum TEXT,state TEXT NOT NULL CHECK(state IN ('encoding','published','dirty','retired')),rows INTEGER NOT NULL); CREATE TABLE _bulk_legacy_sql(kind TEXT NOT NULL,name TEXT PRIMARY KEY,sql TEXT NOT NULL);").execute(&mut *conn).await?;
    for (table_index, table) in TABLES.iter().enumerate() {
        tracing::info!(table = table.name, "building bulk storage schema");
        if table.name == "elements" {
            super::elements::bootstrap(conn).await?;
            continue;
        }
        // Column-aware triggers below own logical changes. Physical sealing
        // updates archive locators without invalidating complete read results.
        for event in ["INSERT", "UPDATE", "DELETE"] {
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                "DROP TRIGGER hybrid_revision_{}_{event}",
                table.name
            )))
            .execute(&mut *conn)
            .await?;
        }
        let original_columns: Vec<String> =
            sqlx::query_scalar("SELECT name FROM pragma_table_info(?) ORDER BY cid")
                .bind(table.name)
                .fetch_all(&mut *conn)
                .await?;
        let legacy=sqlx::query("SELECT type,name,sql FROM sqlite_master WHERE (tbl_name=? AND type='trigger' AND instr(sql,?)>0) OR (type='table' AND name=?) OR (?='audio_transcriptions' AND name IN ('idx_audio_transcription_chunk_text','idx_audio_transcriptions_transcription'))")
            .bind(table.name).bind(format!("{}_fts",table.name)).bind(format!("{}_fts",table.name)).bind(table.name).fetch_all(&mut *conn).await?;
        for row in legacy {
            let kind: &str = row.try_get("type")?;
            let name: &str = row.try_get("name")?;
            sqlx::query("INSERT INTO _bulk_legacy_sql VALUES(?,?,?)")
                .bind(kind)
                .bind(name)
                .bind(row.try_get::<&str, _>("sql")?)
                .execute(&mut *conn)
                .await?;
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                "DROP {} \"{}\"",
                kind,
                name.replace('"', "\"\"")
            )))
            .execute(&mut *conn)
            .await?;
        }
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("ALTER TABLE {t} ADD COLUMN _archive_file INTEGER REFERENCES _bulk_files(id); ALTER TABLE {t} ADD COLUMN _archive_mask INTEGER NOT NULL DEFAULT {mask}; ALTER TABLE {t} ADD COLUMN _archive_generation INTEGER NOT NULL DEFAULT 1; CREATE INDEX _bulk_{t}_files ON {t}(_archive_file) WHERE _archive_file IS NOT NULL; CREATE INDEX _bulk_{t}_pending ON {t}(id) WHERE _archive_mask!=0;",t=table.name,mask=table.mask()))).execute(&mut *conn).await?;
        if table.name == "audio_transcriptions" {
            sqlx::raw_sql("ALTER TABLE audio_transcriptions ADD COLUMN _archive_digest BLOB; CREATE UNIQUE INDEX _bulk_audio_identity ON audio_transcriptions(audio_chunk_id,CASE WHEN (_archive_mask&1)!=0 THEN screenpipe_payload_sha256(transcription) ELSE _archive_digest END);").execute(&mut *conn).await?;
        }
        let columns=original_columns.iter().map(|name|{
            if let Some(index)=table.columns.iter().position(|c|c.name==name){
                format!("CASE WHEN (e._archive_mask & {})!=0 THEN e.{name} ELSE screenpipe_bulk(p.path,p.checksum,{table_index},e.id,{index}) END AS {name}",1<<index)
            } else {format!("e.\"{name}\"")}
        }).collect::<Vec<_>>().join(",");
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE VIEW {} AS SELECT {columns} FROM {} e LEFT JOIN _bulk_files p ON p.id=e._archive_file",table.view(),table.name))).execute(&mut *conn).await?;
        if !table.fts.is_empty() {
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE VIRTUAL TABLE {t}_fts USING fts5({schema},content='',contentless_delete=1,tokenize='unicode61'); INSERT INTO {t}_fts(rowid,{columns}) SELECT id,{columns} FROM {t} WHERE {condition};",t=table.name,schema=table.fts.join(","),columns=table.fts_columns(),condition=table.fts_condition))).execute(&mut *conn).await?;
        }
        sqlx::query(sqlx::AssertSqlSafe(format!("UPDATE storage_metadata SET staging_bytes=staging_bytes+COALESCE((SELECT SUM({}) FROM {}),0)",table.local_bytes(""),table.name))).execute(&mut *conn).await?;
        sqlx::raw_sql(sqlx::AssertSqlSafe(triggers(table, &original_columns)))
            .execute(&mut *conn)
            .await?;
        crate::storage::schema::construction_checkpoint(conn).await?;
    }
    sqlx::query("INSERT INTO _hybrid_migrations VALUES(2,?)")
        .bind(super::manifest_checksum())
        .execute(&mut *conn)
        .await?;
    Ok(())
}

fn reindex(table: &Table) -> String {
    if table.fts.is_empty() {
        return String::new();
    }
    format!("DELETE FROM {t}_fts WHERE rowid=NEW.id; INSERT INTO {t}_fts(rowid,{columns}) SELECT id,{columns} FROM {view} WHERE id=NEW.id AND ({condition});",t=table.name,columns=table.fts_columns(),view=table.view(),condition=table.fts_condition)
}

fn triggers(table: &Table, original_columns: &[String]) -> String {
    let t = table.name;
    let new_bytes = table.all_bytes("NEW.");
    let old_bytes = table.local_bytes("OLD.");
    let check=format!("SELECT CASE WHEN ({new_bytes})>(SELECT record_limit FROM storage_metadata) THEN RAISE(ABORT,'bulk record budget exceeded') END;");
    let staging_check = "SELECT CASE WHEN (SELECT staging_bytes>staging_limit FROM storage_metadata) THEN RAISE(ABORT,'storage staging budget reached') END;";
    let fts = reindex(table);
    let fts_delete = if table.fts.is_empty() {
        String::new()
    } else {
        format!("DELETE FROM {t}_fts WHERE rowid=OLD.id;")
    };
    let mut sql=format!("CREATE TRIGGER hybrid_bulk_{t}_id BEFORE UPDATE OF id ON {t} WHEN OLD._archive_file IS NOT NULL AND NEW.id!=OLD.id BEGIN SELECT RAISE(ABORT,'archived record IDs are immutable'); END; CREATE TRIGGER hybrid_bulk_{t}_insert AFTER INSERT ON {t} WHEN (SELECT maintenance=0 FROM storage_metadata) BEGIN {check} UPDATE storage_metadata SET staging_bytes=staging_bytes+({new_bytes}),revision=revision+1; {staging_check} {fts} END; CREATE TRIGGER hybrid_bulk_{t}_delete BEFORE DELETE ON {t} BEGIN {fts_delete} UPDATE _bulk_files SET state='dirty' WHERE id=OLD._archive_file; UPDATE storage_metadata SET staging_bytes=staging_bytes-({old_bytes}),revision=revision+1; END;");
    sql.push_str(&format!("CREATE TRIGGER hybrid_bulk_{t}_revision AFTER UPDATE OF {} ON {t} WHEN (SELECT maintenance=0 FROM storage_metadata) BEGIN UPDATE storage_metadata SET revision=revision+1; END;", original_columns.join(",")));
    for (index, column) in table.columns.iter().enumerate() {
        let c = column.name;
        let bit = 1 << index;
        sql.push_str(&format!("CREATE TRIGGER hybrid_bulk_{t}_{c} AFTER UPDATE OF {c} ON {t} WHEN (SELECT maintenance=0 FROM storage_metadata) BEGIN {check} UPDATE {t} SET _archive_mask=_archive_mask|{bit},_archive_generation=_archive_generation+1 WHERE id=NEW.id; UPDATE _bulk_files SET state='dirty' WHERE id=OLD._archive_file; UPDATE storage_metadata SET staging_bytes=staging_bytes+COALESCE(length(CAST(NEW.{c} AS BLOB)),0)-CASE WHEN (OLD._archive_mask & {bit})!=0 THEN COALESCE(length(CAST(OLD.{c} AS BLOB)),0) ELSE 0 END; {staging_check} {fts} END;"));
    }
    let metadata = table
        .fts
        .iter()
        .map(|c| c.split_whitespace().next().unwrap())
        .filter(|name| !table.columns.iter().any(|c| c.name == *name))
        .collect::<Vec<_>>();
    if !metadata.is_empty() {
        sql.push_str(&format!("CREATE TRIGGER hybrid_bulk_{t}_fts_metadata AFTER UPDATE OF {} ON {t} WHEN (SELECT maintenance=0 FROM storage_metadata) BEGIN {fts} END;",metadata.join(",")));
    }
    sql
}
