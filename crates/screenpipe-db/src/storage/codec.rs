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
    #[cfg(feature = "storage-bench-experiments")]
    if super::experiments::enabled("selective-decode") {
        if let Some(requested) = requested {
            return selected_columns(&reader, projection, requested, budget);
        }
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

/// Decode selected flat records directly from their columns. Skipped records
/// do not allocate strings or row objects; Parquet still decompresses pages.
#[cfg(feature = "storage-bench-experiments")]
pub(super) fn column_positions(
    group: &dyn parquet::file::reader::RowGroupReader,
    column: usize,
    positions: &[usize],
    nullable: bool,
) -> Result<Vec<Field>, sqlx::Error> {
    use parquet::column::reader::ColumnReader;
    macro_rules! selected {
        ($reader:ident, $convert:expr) => {{
            let mut output = Vec::with_capacity(positions.len());
            let mut cursor = 0;
            let mut index = 0;
            while index < positions.len() {
                let start = positions[index];
                let skip = start
                    .checked_sub(cursor)
                    .ok_or_else(|| storage_error("unordered selection"))?;
                if $reader.skip_records(skip).map_err(storage_error)? != skip {
                    return Err(storage_error("incomplete selected column skip"));
                }
                let mut end = index + 1;
                while end < positions.len() && positions[end] == positions[end - 1] + 1 {
                    end += 1;
                }
                let count = end - index;
                let mut definitions = Vec::new();
                let mut values = Vec::new();
                let (records, _, _) = $reader
                    .read_records(
                        count,
                        nullable.then_some(&mut definitions),
                        None,
                        &mut values,
                    )
                    .map_err(storage_error)?;
                if records != count {
                    return Err(storage_error("incomplete selected column"));
                }
                let mut values = values.into_iter();
                if nullable {
                    if definitions.len() != count {
                        return Err(storage_error("incomplete selected definitions"));
                    }
                    for definition in definitions {
                        output.push(match definition {
                            0 => Field::Null,
                            1 => ($convert)(
                                values
                                    .next()
                                    .ok_or_else(|| storage_error("missing selected value"))?,
                            )?,
                            _ => return Err(storage_error("invalid selected definition")),
                        });
                    }
                } else {
                    for _ in 0..count {
                        output.push(($convert)(
                            values
                                .next()
                                .ok_or_else(|| storage_error("missing selected value"))?,
                        )?);
                    }
                }
                if values.next().is_some() {
                    return Err(storage_error("extra selected values"));
                }
                cursor = positions[end - 1] + 1;
                index = end;
            }
            Ok(output)
        }};
    }
    match group.get_column_reader(column).map_err(storage_error)? {
        ColumnReader::Int64ColumnReader(mut reader) => {
            selected!(reader, |v| Ok::<_, sqlx::Error>(Field::Long(v)))
        }
        ColumnReader::DoubleColumnReader(mut reader) => {
            selected!(reader, |v| Ok::<_, sqlx::Error>(Field::Double(v)))
        }
        ColumnReader::ByteArrayColumnReader(mut reader) => {
            selected!(reader, |v: ByteArray| Ok::<_, sqlx::Error>(Field::Str(
                std::str::from_utf8(v.data())
                    .map_err(storage_error)?
                    .to_owned()
            )))
        }
        _ => Err(storage_error("unsupported selected column")),
    }
}

#[cfg(feature = "storage-bench-experiments")]
fn selected_columns(
    file: &SerializedFileReader<File>,
    projection: Projection,
    requested: &std::collections::BTreeSet<i64>,
    budget: &StorageBudget,
) -> Result<Vec<FramePayload>, sqlx::Error> {
    use parquet::column::reader::ColumnReader;
    let mut output = Vec::new();
    let mut bytes = 0;
    for index in 0..file.num_row_groups() {
        if let Some(parquet::file::statistics::Statistics::Int64(stats)) =
            file.metadata().row_group(index).column(0).statistics()
        {
            if let (Some(min), Some(max)) = (stats.min_opt(), stats.max_opt()) {
                if min > max {
                    return Err(storage_error("invalid Parquet ID bounds"));
                }
                if requested.range(*min..=*max).next().is_none() {
                    continue;
                }
            }
        }
        let group = file.get_row_group(index).map_err(storage_error)?;
        let ColumnReader::Int64ColumnReader(mut id_reader) =
            group.get_column_reader(0).map_err(storage_error)?
        else {
            return Err(storage_error("invalid ID column"));
        };
        let count = group.metadata().num_rows() as usize;
        let mut ids = Vec::new();
        let (records, _, _) = id_reader
            .read_records(count, None, None, &mut ids)
            .map_err(storage_error)?;
        if records != count || ids.len() != count {
            return Err(storage_error("incomplete IDs"));
        }
        let positions: Vec<_> = ids
            .iter()
            .enumerate()
            .filter_map(|(i, id)| requested.contains(id).then_some(i))
            .collect();
        let generations = column_positions(&*group, 1, &positions, false)?;
        let first = column_positions(&*group, 2, &positions, true)?;
        let second = column_positions(&*group, 3, &positions, true)?;
        let text = |field| match field {
            Field::Null => Ok(None),
            Field::Str(s) => Ok(Some(s)),
            _ => Err(storage_error("invalid selected text")),
        };
        for (((position, generation), first), second) in positions
            .into_iter()
            .zip(generations)
            .zip(first)
            .zip(second)
        {
            let Field::Long(generation) = generation else {
                return Err(storage_error("invalid selected generation"));
            };
            let mut row = FramePayload {
                id: ids[position],
                generation,
                ..Default::default()
            };
            match projection {
                Projection::Search => {
                    row.full_text = text(first)?;
                    row.accessibility_text = text(second)?;
                }
                Projection::Detail => {
                    row.accessibility_tree_json = text(first)?;
                    row.text_json = text(second)?;
                }
                Projection::All => unreachable!(),
            }
            bytes += row.bytes();
            if bytes > budget.decode_bytes {
                return Err(storage_error("payload decode budget exceeded"));
            }
            output.push(row);
        }
    }
    Ok(output)
}
