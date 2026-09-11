// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use super::{storage_error, FramePayload, Projection, StorageBudget};
use parquet::basic::{Compression, ZstdLevel};
use parquet::data_type::{ByteArray, ByteArrayType, Int64Type};
use parquet::file::properties::WriterProperties;
use parquet::file::reader::{FileReader, SerializedFileReader};
use parquet::file::writer::SerializedFileWriter;
use parquet::record::{Field, RowAccessor};
use parquet::schema::parser::parse_message_type;
use sha2::{Digest, Sha256};
use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

pub const SCHEMA_VERSION: u32 = 1;
const SEARCH_SCHEMA: &str = "message screenpipe_search_v1 { REQUIRED INT64 id; REQUIRED INT64 generation; OPTIONAL BYTE_ARRAY full_text (UTF8); OPTIONAL BYTE_ARRAY accessibility_text (UTF8); }";
const DETAIL_SCHEMA: &str = "message screenpipe_detail_v1 { REQUIRED INT64 id; REQUIRED INT64 generation; OPTIONAL BYTE_ARRAY accessibility_tree_json (UTF8); OPTIONAL BYTE_ARRAY text_json (UTF8); }";

fn schema(projection: Projection) -> &'static str {
    match projection {
        Projection::Search => SEARCH_SCHEMA,
        Projection::Detail => DETAIL_SCHEMA,
        Projection::All => unreachable!("each immutable file has one projection"),
    }
}

pub fn checksum(path: &Path) -> Result<String, sqlx::Error> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let len = file.read(&mut buffer)?;
        if len == 0 {
            break;
        }
        hasher.update(&buffer[..len]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn write(
    path: &Path,
    rows: &[FramePayload],
    projection: Projection,
    budget: &StorageBudget,
) -> Result<String, sqlx::Error> {
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let schema = Arc::new(parse_message_type(schema(projection)).map_err(storage_error)?);
    let properties = Arc::new(
        WriterProperties::builder()
            .set_compression(Compression::ZSTD(
                ZstdLevel::try_new(3).map_err(storage_error)?,
            ))
            .set_dictionary_enabled(true)
            .build(),
    );
    let mut writer = SerializedFileWriter::new(file, schema, properties).map_err(storage_error)?;
    for chunk in rows.chunks(budget.row_group_rows) {
        let mut group = writer.next_row_group().map_err(storage_error)?;
        for column in 0..4 {
            let mut out = group
                .next_column()
                .map_err(storage_error)?
                .ok_or_else(|| storage_error("missing Parquet column"))?;
            if column < 2 {
                let values: Vec<i64> = chunk
                    .iter()
                    .map(|r| if column == 0 { r.id } else { r.generation })
                    .collect();
                out.typed::<Int64Type>()
                    .write_batch(&values, None, None)
                    .map_err(storage_error)?;
            } else {
                let values: Vec<Option<&str>> = chunk
                    .iter()
                    .map(|r| match (projection, column) {
                        (Projection::Search, 2) => r.full_text.as_deref(),
                        (Projection::Search, 3) => r.accessibility_text.as_deref(),
                        (Projection::Detail, 2) => r.accessibility_tree_json.as_deref(),
                        (Projection::Detail, 3) => r.text_json.as_deref(),
                        _ => unreachable!(),
                    })
                    .collect();
                let definitions: Vec<i16> = values.iter().map(|s| i16::from(s.is_some())).collect();
                let present: Vec<ByteArray> =
                    values.into_iter().flatten().map(ByteArray::from).collect();
                out.typed::<ByteArrayType>()
                    .write_batch(&present, Some(&definitions), None)
                    .map_err(storage_error)?;
            }
            out.close().map_err(storage_error)?;
        }
        group.close().map_err(storage_error)?;
    }
    writer.into_inner().map_err(storage_error)?.sync_all()?;
    checksum(path)
}

pub fn read(
    path: &Path,
    projection: Projection,
    expected_checksum: &str,
    budget: &StorageBudget,
) -> Result<Vec<FramePayload>, sqlx::Error> {
    read_selected(path, projection, expected_checksum, budget, None)
}

/// Row-group statistics select complete groups; exact IDs are checked by the
/// catalog reader after decoding. Checksums cover the entire immutable file.
pub fn read_selected(
    path: &Path,
    projection: Projection,
    expected_checksum: &str,
    budget: &StorageBudget,
    requested: Option<&std::collections::BTreeSet<i64>>,
) -> Result<Vec<FramePayload>, sqlx::Error> {
    if checksum(path)? != expected_checksum {
        return Err(storage_error("Parquet checksum mismatch"));
    }
    let reader = SerializedFileReader::new(File::open(path)?).map_err(storage_error)?;
    let expected = parse_message_type(schema(projection)).map_err(storage_error)?;
    if reader.metadata().file_metadata().schema() != &expected {
        return Err(storage_error("unsupported Parquet schema"));
    }
    let metadata = reader.metadata();
    let decoded: i64 = metadata
        .row_groups()
        .iter()
        .map(|g| g.total_byte_size())
        .sum();
    if decoded < 0 || decoded as u64 > budget.decode_bytes as u64 {
        return Err(storage_error("payload decode budget exceeded"));
    }
    let count = metadata.file_metadata().num_rows();
    if count < 0 || count as usize > budget.file_rows {
        return Err(storage_error("payload row budget exceeded"));
    }
    let mut rows = Vec::with_capacity(requested.map_or(count as usize, |ids| ids.len()));
    let mut bytes = 0;
    for group_index in 0..reader.num_row_groups() {
        if let (Some(ids), Some(parquet::file::statistics::Statistics::Int64(stats))) = (
            requested,
            metadata.row_group(group_index).column(0).statistics(),
        ) {
            if let (Some(min), Some(max)) = (stats.min_opt(), stats.max_opt()) {
                if min > max {
                    return Err(storage_error("invalid Parquet ID bounds"));
                }
                if ids.range(*min..=*max).next().is_none() {
                    continue;
                }
            }
        }
        let group = reader.get_row_group(group_index).map_err(storage_error)?;
        for result in group.get_row_iter(None).map_err(storage_error)? {
            let row = result.map_err(storage_error)?;
            let id = row.get_long(0).map_err(storage_error)?;
            if requested.is_some_and(|ids| !ids.contains(&id)) {
                continue;
            }
            let text = |index| -> Result<Option<String>, sqlx::Error> {
                match row.get_column_iter().nth(index).map(|(_, v)| v) {
                    Some(Field::Null) => Ok(None),
                    Some(Field::Str(s)) => Ok(Some(s.clone())),
                    _ => Err(storage_error("invalid Parquet string field")),
                }
            };
            let mut payload = FramePayload {
                id,
                generation: row.get_long(1).map_err(storage_error)?,
                ..Default::default()
            };
            match projection {
                Projection::Search => {
                    payload.full_text = text(2)?;
                    payload.accessibility_text = text(3)?;
                }
                Projection::Detail => {
                    payload.accessibility_tree_json = text(2)?;
                    payload.text_json = text(3)?;
                }
                Projection::All => unreachable!(),
            }
            bytes += payload.bytes();
            if bytes > budget.decode_bytes {
                return Err(storage_error("payload decode budget exceeded"));
            }
            rows.push(payload);
        }
    }
    Ok(rows)
}
