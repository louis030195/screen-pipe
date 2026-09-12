// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use super::{codec, elements, storage_error};
use crate::storage::StorageBudget;
use parquet::{
    column::reader::ColumnReader,
    file::reader::{FileReader, RowGroupReader},
};
use std::{collections::HashMap, path::Path};

pub(super) struct Entry {
    pub id: i64,
    pub generation: i64,
    pub frame: i64,
    pub sort: i64,
    pub visible: i64,
    pub redacted: i64,
    pub source: u16,
    pub role: u16,
    pub present: u8,
}

/// Search metadata is decoded directly from its Parquet columns. Compact
/// arrays and dictionaries serve filtering/order/counts; text and geometry
/// remain in the full-record projection until a caller requests them.
pub(super) struct ElementIndex {
    pub rows: Vec<Entry>,
    pub sources: Vec<String>,
    pub roles: Vec<String>,
}

impl ElementIndex {
    pub fn bytes(&self) -> usize {
        self.rows.capacity() * std::mem::size_of::<Entry>()
            + (self.sources.capacity() + self.roles.capacity()) * std::mem::size_of::<String>()
            + self
                .sources
                .iter()
                .chain(&self.roles)
                .map(String::capacity)
                .sum::<usize>()
    }
}

fn integers(
    group: &dyn RowGroupReader,
    column: usize,
    nullable: bool,
) -> Result<Vec<Option<i64>>, sqlx::Error> {
    let count = group.metadata().num_rows() as usize;
    let ColumnReader::Int64ColumnReader(mut reader) =
        group.get_column_reader(column).map_err(storage_error)?
    else {
        return Err(storage_error("invalid element index column"));
    };
    let mut values = Vec::new();
    let mut definitions = Vec::new();
    let (records, _, _) = reader
        .read_records(
            count,
            nullable.then_some(&mut definitions),
            None,
            &mut values,
        )
        .map_err(storage_error)?;
    if records != count {
        return Err(storage_error("incomplete element index"));
    }
    if !nullable {
        return Ok(values.into_iter().map(Some).collect());
    }
    let mut values = values.into_iter();
    definitions
        .into_iter()
        .map(|level| match level {
            0 => Ok(None),
            1 => values
                .next()
                .map(Some)
                .ok_or_else(|| storage_error("incomplete element index value")),
            _ => Err(storage_error("invalid element index definition")),
        })
        .collect()
}

fn dictionary(
    group: &dyn RowGroupReader,
    column: usize,
    map: &mut HashMap<String, u16>,
    strings: &mut Vec<String>,
) -> Result<Vec<u16>, sqlx::Error> {
    let count = group.metadata().num_rows() as usize;
    let ColumnReader::ByteArrayColumnReader(mut reader) =
        group.get_column_reader(column).map_err(storage_error)?
    else {
        return Err(storage_error("invalid element dictionary column"));
    };
    let mut values = Vec::new();
    let mut definitions = Vec::new();
    let (records, _, _) = reader
        .read_records(count, Some(&mut definitions), None, &mut values)
        .map_err(storage_error)?;
    if records != count || values.len() != count || definitions.iter().any(|v| *v != 1) {
        return Err(storage_error("incomplete element dictionary"));
    }
    values
        .into_iter()
        .map(|value| {
            let text = std::str::from_utf8(value.data()).map_err(storage_error)?;
            if let Some(id) = map.get(text) {
                return Ok(*id);
            }
            let id = u16::try_from(strings.len()).map_err(storage_error)?;
            strings.push(text.to_owned());
            map.insert(text.to_owned(), id);
            Ok(id)
        })
        .collect()
}

pub(super) fn read(
    path: &Path,
    hash: &str,
    budget: &StorageBudget,
) -> Result<ElementIndex, sqlx::Error> {
    let file = codec::checked_reader(path, hash, &elements::TABLE, budget)?;
    let count = file.metadata().file_metadata().num_rows() as usize;
    let mut index = ElementIndex {
        rows: Vec::with_capacity(count),
        sources: Vec::new(),
        roles: Vec::new(),
    };
    let mut sources = HashMap::new();
    let mut roles = HashMap::new();
    for group in 0..file.num_row_groups() {
        let group = file.get_row_group(group).map_err(storage_error)?;
        let ids = integers(&*group, 0, false)?;
        let generations = integers(&*group, 1, false)?;
        let frames = integers(&*group, 2, true)?;
        let source = dictionary(&*group, 3, &mut sources, &mut index.sources)?;
        let role = dictionary(&*group, 4, &mut roles, &mut index.roles)?;
        let sorts = integers(&*group, 13, true)?;
        let visible = integers(&*group, 15, true)?;
        let redacted = integers(&*group, 16, true)?;
        if [
            generations.len(),
            frames.len(),
            source.len(),
            role.len(),
            sorts.len(),
            visible.len(),
            redacted.len(),
        ]
        .into_iter()
        .any(|len| len != ids.len())
        {
            return Err(storage_error("element index column lengths differ"));
        }
        for i in 0..ids.len() {
            let required = |value: Option<i64>| {
                value.ok_or_else(|| storage_error("required element index value is NULL"))
            };
            let id = required(ids[i])?;
            if index.rows.last().is_some_and(|row| row.id >= id) {
                return Err(storage_error("element index order invalid"));
            }
            index.rows.push(Entry {
                id,
                generation: required(generations[i])?,
                frame: required(frames[i])?,
                sort: required(sorts[i])?,
                visible: visible[i].unwrap_or(0),
                redacted: redacted[i].unwrap_or(0),
                present: u8::from(visible[i].is_some()) | (u8::from(redacted[i].is_some()) << 1),
                source: source[i],
                role: role[i],
            });
        }
    }
    Ok(index)
}
