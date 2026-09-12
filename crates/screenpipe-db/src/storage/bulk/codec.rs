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
