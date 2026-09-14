// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use super::{Kind, Record, Table, Value, FILE_ROWS, GROUP_ROWS};
use crate::storage::{codec::checksum, storage_error, StorageBudget};
use parquet::{
    basic::{Compression, ZstdLevel},
    data_type::{ByteArray, ByteArrayType, DoubleType, Int64Type},
    file::{
        properties::WriterProperties,
        reader::{FileReader, SerializedFileReader},
        writer::SerializedFileWriter,
    },
    record::{Field, RowAccessor},
    schema::parser::parse_message_type,
};
use std::{
    fs::{File, OpenOptions},
    path::Path,
    sync::Arc,
};

fn schema(table: &Table) -> String {
    let columns = table
        .columns
        .iter()
        .map(|c| {
            format!(
                "OPTIONAL {} {}{};",
                match c.kind {
                    Kind::Text => "BYTE_ARRAY",
                    Kind::Integer => "INT64",
                    Kind::Real => "DOUBLE",
                },
                c.name,
                if matches!(c.kind, Kind::Text) {
                    " (UTF8)"
                } else {
                    ""
                }
            )
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!("message screenpipe_bulk_{}_v1 {{ REQUIRED INT64 id; REQUIRED INT64 generation; {columns} }}",table.name)
}

pub(super) fn write(path: &Path, table: &Table, rows: &[Record]) -> Result<String, sqlx::Error> {
    let file = OpenOptions::new().write(true).create_new(true).open(path)?;
    let properties = Arc::new(
        WriterProperties::builder()
            .set_compression(Compression::ZSTD(
                ZstdLevel::try_new(3).map_err(storage_error)?,
            ))
            .set_dictionary_enabled(true)
            .build(),
    );
    let mut writer = SerializedFileWriter::new(
        file,
        Arc::new(parse_message_type(&schema(table)).map_err(storage_error)?),
        properties,
    )
    .map_err(storage_error)?;
    for chunk in rows.chunks(GROUP_ROWS) {
        let mut group = writer.next_row_group().map_err(storage_error)?;
        for column in 0..table.columns.len() + 2 {
            let mut output = group
                .next_column()
                .map_err(storage_error)?
                .ok_or_else(|| storage_error("bulk column missing"))?;
            if column < 2 {
                let values = chunk
                    .iter()
                    .map(|r| if column == 0 { r.id } else { r.generation })
                    .collect::<Vec<_>>();
                output
                    .typed::<Int64Type>()
                    .write_batch(&values, None, None)
                    .map_err(storage_error)?;
            } else {
                let values = chunk
                    .iter()
                    .map(|r| &r.values[column - 2])
                    .collect::<Vec<_>>();
                let definitions = values
                    .iter()
                    .map(|v| i16::from(!matches!(v, Value::Null)))
                    .collect::<Vec<_>>();
                match table.columns[column - 2].kind {
                    Kind::Text => {
                        let values = values
                            .iter()
                            .filter_map(|v| {
                                if let Value::Text(s) = v {
                                    Some(ByteArray::from(s.as_str()))
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>();
                        output
                            .typed::<ByteArrayType>()
                            .write_batch(&values, Some(&definitions), None)
                            .map_err(storage_error)?;
                    }
                    Kind::Integer => {
                        let values = values
                            .iter()
                            .filter_map(|v| {
                                if let Value::Integer(i) = v {
                                    Some(*i)
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>();
                        output
                            .typed::<Int64Type>()
                            .write_batch(&values, Some(&definitions), None)
                            .map_err(storage_error)?;
                    }
                    Kind::Real => {
                        let values = values
                            .iter()
                            .filter_map(|v| {
                                if let Value::Real(f) = v {
                                    Some(*f)
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>();
                        output
                            .typed::<DoubleType>()
                            .write_batch(&values, Some(&definitions), None)
                            .map_err(storage_error)?;
                    }
                }
            }
            output.close().map_err(storage_error)?;
        }
        group.close().map_err(storage_error)?;
    }
    writer.into_inner().map_err(storage_error)?.sync_all()?;
    checksum(path)
}

pub(super) fn read(
    path: &Path,
    hash: &str,
    table: &Table,
    budget: &StorageBudget,
) -> Result<Vec<Record>, sqlx::Error> {
    let reader = checked_reader(path, hash, table, budget)?;
    let count = reader.metadata().file_metadata().num_rows();
    let mut rows = Vec::with_capacity(count as usize);
    let mut decoded = 0;
    for row in reader.get_row_iter(None).map_err(storage_error)? {
        let row = row.map_err(storage_error)?;
        let values = row
            .get_column_iter()
            .skip(2)
            .map(|(_, v)| match v {
                Field::Null => Ok(Value::Null),
                Field::Str(s) => Ok(Value::Text(s.clone())),
                Field::Long(i) => Ok(Value::Integer(*i)),
                Field::Double(f) => Ok(Value::Real(*f)),
                _ => Err(storage_error("invalid bulk value")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let record = Record {
            id: row.get_long(0).map_err(storage_error)?,
            generation: row.get_long(1).map_err(storage_error)?,
            values,
        };
        if rows.last().is_some_and(|r: &Record| r.id >= record.id) {
            return Err(storage_error("bulk row order invalid"));
        }
        decoded += record.bytes();
        if record.bytes() > budget.record_bytes || decoded > budget.decode_bytes {
            return Err(storage_error("bulk record budget exceeded"));
        }
        rows.push(record);
    }
    Ok(rows)
}

pub(super) fn checked_reader(
    path: &Path,
    hash: &str,
    table: &Table,
    budget: &StorageBudget,
) -> Result<SerializedFileReader<File>, sqlx::Error> {
    if checksum(path)? != hash {
        return Err(storage_error("bulk Parquet checksum mismatch"));
    }
    let reader = SerializedFileReader::new(File::open(path)?).map_err(storage_error)?;
    let expected = parse_message_type(&schema(table)).map_err(storage_error)?;
    if reader.metadata().file_metadata().schema() != &expected {
        return Err(storage_error("unsupported bulk schema"));
    }
    let count = reader.metadata().file_metadata().num_rows();
    let bytes: i64 = reader
        .metadata()
        .row_groups()
        .iter()
        .map(|g| g.total_byte_size())
        .sum();
    if count < 0 || count as usize > FILE_ROWS || bytes < 0 || bytes as usize > budget.decode_bytes
    {
        return Err(storage_error("bulk decode budget exceeded"));
    }
    Ok(reader)
}

pub(super) fn read_positions(
    path: &Path,
    hash: &str,
    table: &Table,
    budget: &StorageBudget,
    positions: &[usize],
) -> Result<Vec<Record>, sqlx::Error> {
    let file = checked_reader(path, hash, table, budget)?;
    let mut output = Vec::with_capacity(positions.len());
    let mut offset = 0;
    let mut bytes = 0;
    for index in 0..file.num_row_groups() {
        let count = file.metadata().row_group(index).num_rows() as usize;
        let selected: Vec<_> = positions
            .iter()
            .filter_map(|&p| (p >= offset && p < offset + count).then(|| p - offset))
            .collect();
        offset += count;
        if selected.is_empty() {
            continue;
        }
        let group = file.get_row_group(index).map_err(storage_error)?;
        let mut rows: Vec<_> = selected
            .iter()
            .map(|_| Record {
                id: 0,
                generation: 0,
                values: Vec::with_capacity(table.columns.len()),
            })
            .collect();
        for column in 0..table.columns.len() + 2 {
            let values =
                crate::storage::codec::column_positions(&*group, column, &selected, column >= 2)?;
            for (row, value) in rows.iter_mut().zip(values) {
                match (column, value) {
                    (0, Field::Long(id)) => row.id = id,
                    (1, Field::Long(generation)) => row.generation = generation,
                    (0..=1, _) => return Err(storage_error("invalid selected bulk identity")),
                    (_, Field::Null) => row.values.push(Value::Null),
                    (_, Field::Str(s)) => row.values.push(Value::Text(s)),
                    (_, Field::Long(i)) => row.values.push(Value::Integer(i)),
                    (_, Field::Double(f)) => row.values.push(Value::Real(f)),
                    _ => return Err(storage_error("invalid selected bulk value")),
                }
            }
        }
        for row in rows {
            bytes += row.bytes();
            if row.bytes() > budget.record_bytes || bytes > budget.decode_bytes {
                return Err(storage_error("bulk record budget exceeded"));
            }
            if output.last().is_some_and(|r: &Record| r.id >= row.id) {
                return Err(storage_error("bulk row order invalid"));
            }
            output.push(row);
        }
    }
    if output.len() != positions.len() {
        return Err(storage_error("incomplete selected bulk rows"));
    }
    Ok(output)
}
