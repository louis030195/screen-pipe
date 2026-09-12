// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

//! Ordered element records and typed payload columns share bounded Parquet
//! files. SQLite owns pending writes, search postings, and relational lookups.

mod cache;
mod codec;
mod connection;
mod element_index;
pub(super) mod elements;
mod lifecycle;
mod schema;
#[cfg(test)]
mod tests;

use super::{storage_error, HybridStorage};
pub(crate) use connection::pool_options;
pub(crate) use connection::register_hash;
pub(super) use lifecycle::export;
pub(super) use schema::bootstrap;
use std::sync::{atomic::AtomicUsize, Arc, Mutex};

pub const CAPABILITY: &str = "parquet-bulk-v1";
pub const FILE_ROWS: usize = 32_768;
pub const GROUP_ROWS: usize = 8_192;

pub(super) fn is_bulk_table(name: &str) -> bool {
    TABLES.iter().any(|t| t.name == name)
}

#[derive(Clone, Copy, Debug)]
pub(super) enum Kind {
    Text,
    Integer,
    Real,
}

pub(super) struct Column {
    pub name: &'static str,
    pub kind: Kind,
    pub empty: &'static str,
}

impl Column {
    fn bytes(&self, prefix: &str) -> String {
        match self.kind {
            Kind::Text => format!("COALESCE(length(CAST({prefix}{} AS BLOB)),0)", self.name),
            Kind::Integer | Kind::Real => {
                format!("CASE WHEN {prefix}{} IS NULL THEN 0 ELSE 8 END", self.name)
            }
        }
    }
}

const fn text(name: &'static str) -> Column {
    Column {
        name,
        kind: Kind::Text,
        empty: "NULL",
    }
}
const fn required(name: &'static str, empty: &'static str) -> Column {
    Column {
        name,
        kind: Kind::Text,
        empty,
    }
}
const fn integer(name: &'static str) -> Column {
    Column {
        name,
        kind: Kind::Integer,
        empty: "NULL",
    }
}
const fn real(name: &'static str) -> Column {
    Column {
        name,
        kind: Kind::Real,
        empty: "NULL",
    }
}

#[derive(Clone, Copy)]
pub(super) struct Table {
    pub name: &'static str,
    pub columns: &'static [Column],
    pub eligible: &'static str,
    pub fts: &'static [&'static str],
    pub fts_condition: &'static str,
}

pub(super) static TABLES: &[Table] = &[
    elements::TABLE,
    Table { name: "audio_transcriptions", columns: &[required("transcription", "''")],
      eligible: "(redacted_at IS NOT NULL OR (SELECT required_surfaces=0 FROM storage_metadata) OR transcription='')",
      fts: &["transcription", "device", "speaker_id"], fts_condition: "transcription!=''" },
    Table { name: "ui_events", columns: &[text("text_content"),text("element_value"),text("element_description"),text("element_ancestors"),text("element_bounds"),text("window_title"),text("browser_url"),text("element_name"),text("element_automation_id")],
      eligible: "(redacted_at IS NOT NULL OR (SELECT required_surfaces=0 FROM storage_metadata))",
      fts: &["text_content","app_name","window_title","element_name"], fts_condition: "1" },
    Table { name: "semantic_items", columns: &[text("body"),required("metadata_json", "'{}'")],
      eligible: "1", fts: &["title","body","actor","status","metadata_json"], fts_condition: "1" },
    Table { name: "pipe_executions", columns: &[text("stdout"),text("stderr"),text("error_message"),text("trigger_event")],
      eligible: "finished_at IS NOT NULL", fts: &[], fts_condition: "1" },
    Table { name: "meeting_transcript_segments", columns: &[required("transcript", "''")],
      eligible: "1", fts: &[], fts_condition: "1" },
    Table { name: "outputs", columns: &[text("preview"),text("metadata")],
      eligible: "1", fts: &[], fts_condition: "1" },
];

impl Table {
    fn mask(&self) -> i64 {
        (1 << self.columns.len()) - 1
    }
    fn view(&self) -> String {
        format!("_bulk_logical_{}", self.name)
    }
    fn fts_columns(&self) -> String {
        self.fts
            .iter()
            .map(|s| s.split_whitespace().next().unwrap())
            .collect::<Vec<_>>()
            .join(",")
    }
    fn local_bytes(&self, prefix: &str) -> String {
        self.columns
            .iter()
            .enumerate()
            .map(|(index, c)| {
                format!(
                    "CASE WHEN ({prefix}_archive_mask & {})!=0 THEN {} ELSE 0 END",
                    1 << index,
                    c.bytes(prefix)
                )
            })
            .collect::<Vec<_>>()
            .join("+")
    }
    fn all_bytes(&self, prefix: &str) -> String {
        self.columns
            .iter()
            .map(|c| c.bytes(prefix))
            .collect::<Vec<_>>()
            .join("+")
    }
}

pub(super) fn manifest_checksum() -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"bulk-v1:element-ranges:frame-group-counts:fts-columnsize-0:column-overrides:audio-sha256-key");
    for table in TABLES {
        hash.update(format!(
            "{}:{}:{}:{:?}",
            table.name, table.eligible, table.fts_condition, table.fts
        ));
        for column in table.columns {
            hash.update(format!(
                "{}:{:?}:{}",
                column.name, column.kind, column.empty
            ));
        }
    }
    format!("{:x}", hash.finalize())
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Value {
    Null,
    Text(String),
    Integer(i64),
    Real(f64),
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct Record {
    pub id: i64,
    pub generation: i64,
    pub values: Vec<Value>,
}

impl Record {
    fn bytes(&self) -> usize {
        self.values
            .iter()
            .map(|v| match v {
                Value::Null => 0,
                Value::Text(s) => s.len(),
                _ => 8,
            })
            .sum()
    }
}

#[derive(Default)]
pub(crate) struct Runtime {
    cache: cache::Cache,
    statements: AtomicUsize,
    element_frames: Mutex<Option<(String, Arc<std::collections::HashMap<i64, Vec<usize>>>)>>,
    #[cfg(test)]
    decode_hook: Mutex<Option<Arc<dyn Fn(&str) + Send + Sync>>>,
}

#[derive(Clone)]
enum Cached {
    Records(Arc<Vec<Record>>),
    ElementIndex(Arc<element_index::ElementIndex>),
}

impl HybridStorage {
    fn element_frame_rows(
        &self,
        key: String,
        rows: &[element_index::Entry],
    ) -> Result<Arc<std::collections::HashMap<i64, Vec<usize>>>, sqlx::Error> {
        {
            let cache = self.bulk.element_frames.lock().map_err(storage_error)?;
            if let Some((existing, frames)) = &*cache {
                if existing == &key {
                    return Ok(frames.clone());
                }
            }
        }
        let mut frames: std::collections::HashMap<i64, Vec<usize>> = Default::default();
        for (index, row) in rows.iter().enumerate() {
            frames.entry(row.frame).or_default().push(index);
        }
        let frames = Arc::new(frames);
        *self.bulk.element_frames.lock().map_err(storage_error)? = Some((key, frames.clone()));
        Ok(frames)
    }
    pub(crate) fn has_bulk(&self) -> bool {
        self.descriptor.capabilities.iter().any(|s| s == CAPABILITY)
    }

    fn element_index(
        &self,
        path: &str,
        hash: &str,
    ) -> Result<Arc<element_index::ElementIndex>, sqlx::Error> {
        let key = format!("index:{path}:{hash}");
        let value = self.bulk.cache.get_or_load(self, key, || {
            let index = element_index::read(
                &self.payload_path(std::path::Path::new(path))?,
                hash,
                &self.descriptor.budget,
            )?;
            Ok(Cached::ElementIndex(Arc::new(index)))
        })?;
        let Cached::ElementIndex(index) = value else {
            unreachable!()
        };
        Ok(index)
    }

    fn bulk_records(
        &self,
        path: &str,
        hash: &str,
        table: usize,
    ) -> Result<Arc<Vec<Record>>, sqlx::Error> {
        let key = format!("records:{path}:{hash}");
        let value = self.bulk.cache.get_or_load(self, key, || {
            let table = TABLES
                .get(table)
                .ok_or_else(|| storage_error("unknown bulk table"))?;
            let rows = codec::read(
                &self.payload_path(std::path::Path::new(path))?,
                hash,
                table,
                &self.descriptor.budget,
            )?;
            Ok(Cached::Records(Arc::new(rows)))
        })?;
        let Cached::Records(rows) = value else {
            unreachable!()
        };
        Ok(rows)
    }
}

impl crate::DatabaseManager {
    /// Resolve a complete row source for a read inside a write transaction.
    /// Read-pool connections resolve these logical views automatically.
    pub(crate) fn logical_table(&self, name: &'static str) -> String {
        if self.storage.as_ref().is_some_and(|s| s.has_bulk()) {
            if let Some(table) = TABLES.iter().find(|t| t.name == name) {
                return table.view();
            }
        }
        name.to_string()
    }
}
