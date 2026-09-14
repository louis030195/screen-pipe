// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

//! Physical reclamation is restricted to the offline migration owner. SQLite
//! keeps its page map; only unused freelist leaves become filesystem holes.

use super::storage_error;
use sqlx::SqliteConnection;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

pub(super) fn allocated(path: &Path) -> Result<u64, sqlx::Error> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(std::fs::metadata(path)?.blocks().saturating_mul(512))
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            FileStandardInfo, GetFileInformationByHandleEx, FILE_STANDARD_INFO,
        };
        let file = File::open(path)?;
        let mut info: FILE_STANDARD_INFO = unsafe { std::mem::zeroed() };
        // SAFETY: info is a correctly sized writable FILE_STANDARD_INFO.
        if unsafe {
            GetFileInformationByHandleEx(
                file.as_raw_handle(),
                FileStandardInfo,
                (&mut info as *mut FILE_STANDARD_INFO).cast(),
                std::mem::size_of_val(&info) as u32,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(info.AllocationSize as u64)
    }
}

pub(super) fn footprint(root: &Path) -> Result<u64, sqlx::Error> {
    super::inventory::inventory(root)?
        .iter()
        .try_fold(0_u64, |n, p| Ok(n.saturating_add(allocated(p)?)))
}

/// Probe an owned scratch file on the exact destination volume, before touching
/// the database. A successful syscall alone does not prove blocks were freed.
pub(super) fn probe(root: &Path) -> Result<(), sqlx::Error> {
    let mut file = tempfile::NamedTempFile::new_in(root)?;
    let bytes = vec![0x5a; 1024 * 1024];
    file.write_all(&bytes)?;
    file.as_file().sync_all()?;
    let before = allocated(file.path())?;
    punch(file.as_file(), 0, bytes.len() as u64).map_err(|e| {
        storage_error(format!(
            "filesystem reclamation unavailable; migration has not started: {e}"
        ))
    })?;
    file.as_file().sync_all()?;
    if allocated(file.path())? >= before {
        return Err(storage_error(
            "this filesystem does not release sparse file blocks; migration has not started",
        ));
    }
    file.seek(SeekFrom::Start(0))?;
    let mut actual = vec![1; bytes.len()];
    file.read_exact(&mut actual)?;
    if actual.iter().any(|b| *b != 0) {
        return Err(storage_error("filesystem reclamation probe failed"));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn punch(file: &File, offset: u64, bytes: u64) -> Result<(), sqlx::Error> {
    use std::os::fd::AsRawFd;
    let hole = libc::fpunchhole_t {
        fp_flags: 0,
        reserved: 0,
        fp_offset: offset as i64,
        fp_length: bytes as i64,
    };
    // SAFETY: file remains open and the kernel reads the documented structure.
    if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_PUNCHHOLE, &hole) } == -1 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn punch(file: &File, offset: u64, bytes: u64) -> Result<(), sqlx::Error> {
    use std::os::fd::AsRawFd;
    // SAFETY: the descriptor is borrowed and ranges are validated by the caller.
    if unsafe {
        libc::fallocate(
            file.as_raw_fd(),
            libc::FALLOC_FL_PUNCH_HOLE | libc::FALLOC_FL_KEEP_SIZE,
            offset as i64,
            bytes as i64,
        )
    } == -1
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}

#[cfg(windows)]
fn punch(file: &File, offset: u64, bytes: u64) -> Result<(), sqlx::Error> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::{
        Ioctl::{FILE_ZERO_DATA_INFORMATION, FSCTL_SET_SPARSE, FSCTL_SET_ZERO_DATA},
        IO::DeviceIoControl,
    };
    let mut returned = 0;
    // SAFETY: synchronous calls borrow the live handle; input has the documented layout.
    unsafe {
        if DeviceIoControl(
            file.as_raw_handle(),
            FSCTL_SET_SPARSE,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
        let range = FILE_ZERO_DATA_INFORMATION {
            FileOffset: offset as i64,
            BeyondFinalZero: (offset + bytes) as i64,
        };
        if DeviceIoControl(
            file.as_raw_handle(),
            FSCTL_SET_ZERO_DATA,
            (&range as *const FILE_ZERO_DATA_INFORMATION).cast(),
            std::mem::size_of_val(&range) as u32,
            std::ptr::null_mut(),
            0,
            &mut returned,
            std::ptr::null_mut(),
        ) == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    Ok(())
}

/// `file` must be opened BEFORE the exclusively locked SQLite connection and
/// closed AFTER it. Closing another Unix fd for this inode can release SQLite's
/// process-wide POSIX locks. This function never opens/closes another db fd.
pub(super) async fn free_leaves(
    conn: &mut SqliteConnection,
    file: &mut File,
    memory_limit: usize,
) -> Result<(), sqlx::Error> {
    let mode: String = sqlx::query_scalar("PRAGMA locking_mode")
        .fetch_one(&mut *conn)
        .await?;
    if mode != "exclusive" {
        return Err(storage_error(
            "reclamation requires exclusive offline SQLite ownership",
        ));
    }
    super::schema::construction_checkpoint(conn).await?;
    let pages: i64 = sqlx::query_scalar("PRAGMA page_count")
        .fetch_one(&mut *conn)
        .await?;
    let expected: i64 = sqlx::query_scalar("PRAGMA freelist_count")
        .fetch_one(&mut *conn)
        .await?;
    if expected == 0 {
        return Ok(());
    }
    let bitmap_bytes = (pages as usize).div_ceil(8);
    if pages <= 0 || bitmap_bytes > memory_limit / 2 {
        return Err(storage_error(
            "freelist map exceeds migration memory budget",
        ));
    }
    let mut header = [0_u8; 100];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut header)?;
    let word = |b: &[u8]| u32::from_be_bytes(b.try_into().unwrap()) as usize;
    let raw = u16::from_be_bytes([header[16], header[17]]) as usize;
    let page_size = if raw == 1 { 65536 } else { raw };
    if &header[..16] != b"SQLite format 3\0"
        || !page_size.is_power_of_two()
        || !(512..=65536).contains(&page_size)
        || word(&header[36..40]) != expected as usize
        || file.metadata()?.len() != pages as u64 * page_size as u64
    {
        return Err(storage_error(
            "checkpointed SQLite freelist header is invalid",
        ));
    }
    let mut leaves = vec![0_u8; bitmap_bytes];
    let mut seen = vec![0_u8; bitmap_bytes];
    let mut count = 0;
    let mut mark = |page: usize| -> Result<(), sqlx::Error> {
        if page <= 1 || page > pages as usize || seen[(page - 1) / 8] & (1 << ((page - 1) % 8)) != 0
        {
            return Err(storage_error("invalid or repeated SQLite freelist page"));
        }
        seen[(page - 1) / 8] |= 1 << ((page - 1) % 8);
        count += 1;
        Ok(())
    };
    let mut trunk = word(&header[32..36]);
    let mut page = vec![0_u8; page_size];
    while trunk != 0 {
        mark(trunk)?;
        file.seek(SeekFrom::Start((trunk - 1) as u64 * page_size as u64))?;
        file.read_exact(&mut page)?;
        let n = word(&page[4..8]);
        if n > (page_size - header[20] as usize) / 4 - 2 {
            return Err(storage_error("invalid SQLite freelist trunk length"));
        }
        for entry in page[8..8 + n * 4].chunks_exact(4) {
            let leaf = word(entry);
            mark(leaf)?;
            leaves[(leaf - 1) / 8] |= 1 << ((leaf - 1) % 8);
        }
        trunk = word(&page[..4]);
    }
    if count != expected as usize {
        return Err(storage_error("SQLite freelist count differs"));
    }
    // 64 KiB also covers NTFS sparse deallocation units. Only wholly free
    // aligned extents are touched, including on databases with smaller pages.
    let alignment = 65536_u64;
    let mut first = None;
    for index in 0..=pages as usize {
        let free = index < pages as usize && leaves[index / 8] & (1 << (index % 8)) != 0;
        if free {
            first.get_or_insert(index);
        } else if let Some(start) = first.take() {
            let lower = (start as u64 * page_size as u64).div_ceil(alignment) * alignment;
            let upper = index as u64 * page_size as u64 / alignment * alignment;
            if upper > lower {
                punch(file, lower, upper - lower)?;
                super::faults::checkpoint("migration_blocks_reclaimed");
            }
        }
    }
    file.sync_all()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Connection;

    #[tokio::test]
    async fn fragmented_freelist_preserves_live_rows_and_file_length() {
        let root = tempfile::tempdir().unwrap();
        probe(root.path()).unwrap();
        let path = root.path().join("index.sqlite");
        let mut file = File::options()
            .read(true)
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        let mut conn = SqliteConnection::connect_with(
            &sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .pragma("journal_mode", "WAL")
                .pragma("locking_mode", "EXCLUSIVE")
                .pragma("secure_delete", "OFF"),
        )
        .await
        .unwrap();
        sqlx::raw_sql("CREATE TABLE cells(id INTEGER PRIMARY KEY,payload BLOB); WITH RECURSIVE n(id) AS (VALUES(1) UNION ALL SELECT id+1 FROM n WHERE id<256) INSERT INTO cells SELECT id,randomblob(8192) FROM n; DELETE FROM cells WHERE id%2=1;").execute(&mut conn).await.unwrap();
        let expected: Vec<(i64, Vec<u8>)> = sqlx::query_as("SELECT * FROM cells ORDER BY id")
            .fetch_all(&mut conn)
            .await
            .unwrap();
        super::super::schema::construction_checkpoint(&mut conn)
            .await
            .unwrap();
        let length = file.metadata().unwrap().len();
        let before = allocated(&path).unwrap();
        assert!(free_leaves(&mut conn, &mut file, 0).await.is_err());
        assert_eq!(
            allocated(&path).unwrap(),
            before,
            "rejected scan must not punch anything"
        );
        free_leaves(&mut conn, &mut file, 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(file.metadata().unwrap().len(), length);
        conn.close().await.unwrap();
        let mut conn = SqliteConnection::connect_with(
            &sqlx::sqlite::SqliteConnectOptions::new()
                .filename(&path)
                .pragma("journal_mode", "WAL")
                .pragma("locking_mode", "EXCLUSIVE")
                .pragma("secure_delete", "OFF"),
        )
        .await
        .unwrap();
        let actual: Vec<(i64, Vec<u8>)> = sqlx::query_as("SELECT * FROM cells ORDER BY id")
            .fetch_all(&mut conn)
            .await
            .unwrap();
        assert_eq!(actual, expected);
        sqlx::query("DELETE FROM cells")
            .execute(&mut conn)
            .await
            .unwrap();
        free_leaves(&mut conn, &mut file, 1024 * 1024)
            .await
            .unwrap();
        assert!(allocated(&path).unwrap() < before / 2);
        assert_eq!(file.metadata().unwrap().len(), length);
        assert_eq!(
            sqlx::query_scalar::<_, String>("PRAGMA integrity_check")
                .fetch_one(&mut conn)
                .await
                .unwrap(),
            "ok"
        );
        conn.close().await.unwrap();
        drop(file);
    }
}
