// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

//! Opt-in experiments for the marked, disposable HTTP benchmark fixture.
//! These switches are absent from normal builds.

use std::{path::Path, sync::OnceLock};

static METHODS: OnceLock<Vec<String>> = OnceLock::new();

pub fn configure(root: &Path, methods: &str) -> Result<(), sqlx::Error> {
    if !root.join(".screenpipe-api-benchmark").is_file() {
        return Err(super::storage_error(
            "experiments require a marked benchmark fixture",
        ));
    }
    let methods: Vec<String> = methods
        .split(',')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    if methods.iter().any(|s| {
        ![
            "frame-cache",
            "element-cache",
            "element-lookup",
            "read-lock",
            "snapshot",
            "snapshot-admission",
            "selective-decode",
        ]
        .contains(&s.as_str())
    }) {
        return Err(super::storage_error("unknown benchmark method"));
    }
    METHODS
        .set(methods)
        .map_err(|_| super::storage_error("benchmark methods already configured"))
}

pub(crate) fn enabled(method: &str) -> bool {
    METHODS
        .get()
        .is_some_and(|methods| methods.iter().any(|s| s == method))
}

use crate::cancellable_query::CancellableReadConnection;
use sqlx::{pool::PoolConnection, Sqlite, SqliteConnection, SqlitePool, Transaction};
use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, OwnedMutexGuard};
use tokio_util::sync::CancellationToken;

type SharedConnection = Arc<Mutex<Option<CancellableReadConnection>>>;
tokio::task_local! {
    static SNAPSHOT: SharedConnection;
    static REVISION: Arc<tokio::sync::OnceCell<(i64,i64)>>;
}

/// The benchmark's API request owns one real read transaction. SQL statements
/// borrow it serially; other requests and the writer retain separate handles.
pub enum ReadConnection {
    Pool(PoolConnection<Sqlite>),
    Cancellable(CancellableReadConnection),
    Transaction(Transaction<'static, Sqlite>),
    Snapshot(OwnedMutexGuard<Option<CancellableReadConnection>>),
}
impl Deref for ReadConnection {
    type Target = SqliteConnection;
    fn deref(&self) -> &SqliteConnection {
        match self {
            Self::Pool(c) => c,
            Self::Cancellable(c) => c,
            Self::Transaction(c) => c,
            Self::Snapshot(c) => c.as_ref().unwrap(),
        }
    }
}
impl DerefMut for ReadConnection {
    fn deref_mut(&mut self) -> &mut SqliteConnection {
        match self {
            Self::Pool(c) => c,
            Self::Cancellable(c) => c,
            Self::Transaction(c) => c,
            Self::Snapshot(c) => c.as_mut().unwrap(),
        }
    }
}
impl ReadConnection {
    pub(super) async fn commit(self) -> Result<(), sqlx::Error> {
        if let Self::Transaction(tx) = self {
            tx.commit().await?;
        }
        Ok(())
    }
}
pub(crate) fn in_snapshot() -> bool {
    SNAPSHOT.try_with(|_| ()).is_ok()
}
pub(crate) fn response_admission() -> bool {
    in_snapshot() && enabled("snapshot-admission")
}

pub(crate) async fn revision(pool: &SqlitePool) -> Result<(i64, i64), sqlx::Error> {
    let cell = REVISION.with(Arc::clone);
    let revision=cell.get_or_try_init(|| async {
        sqlx::query_as("SELECT revision,(SELECT revision FROM _benchmark_revocation WHERE id=1) FROM storage_metadata WHERE singleton=1")
            .fetch_one(&mut *acquire(pool).await?).await
    }).await?;
    Ok(*revision)
}

async fn scoped() -> Option<ReadConnection> {
    let shared = SNAPSHOT.try_with(Arc::clone).ok()?;
    Some(ReadConnection::Snapshot(shared.lock_owned().await))
}
pub(crate) async fn acquire(pool: &SqlitePool) -> Result<ReadConnection, sqlx::Error> {
    if let Some(c) = scoped().await {
        return Ok(c);
    }
    Ok(ReadConnection::Pool(pool.acquire().await?))
}
pub(crate) async fn search(pool: &SqlitePool) -> Result<ReadConnection, sqlx::Error> {
    if let Some(c) = scoped().await {
        return Ok(c);
    }
    Ok(ReadConnection::Cancellable(
        CancellableReadConnection::acquire(
            pool,
            Instant::now() + crate::cancellable_query::SEARCH_QUERY_TIMEOUT,
            CancellationToken::new(),
        )
        .await?,
    ))
}
pub(super) async fn frame_snapshot(pool: &SqlitePool) -> Result<ReadConnection, sqlx::Error> {
    if let Some(c) = scoped().await {
        return Ok(c);
    }
    Ok(ReadConnection::Transaction(pool.begin().await?))
}

struct RequestSnapshot {
    connection: SharedConnection,
    cancellation: CancellationToken,
}
impl Drop for RequestSnapshot {
    fn drop(&mut self) {
        self.cancellation.cancel();
        let connection = Arc::clone(&self.connection);
        tokio::spawn(async move {
            if let Some(mut c) = connection.lock().await.take() {
                if sqlx::query("ROLLBACK").execute(&mut *c).await.is_err() {
                    c.discard().await;
                } else {
                    let _ = c.release().await;
                }
            }
        });
    }
}

pub async fn snapshot<T>(
    pool: &SqlitePool,
    read: impl std::future::Future<Output = T>,
) -> Result<T, sqlx::Error> {
    // Leave a read-pool handle available for response admission. This benchmark
    // process owns one DB; production admission belongs to the manager lifetime.
    static LANES: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    let _lane = if enabled("snapshot-admission") {
        Some(
            LANES
                .get_or_init(|| {
                    Arc::new(tokio::sync::Semaphore::new(
                        pool.options()
                            .get_max_connections()
                            .saturating_sub(1)
                            .max(1) as usize,
                    ))
                })
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| sqlx::Error::PoolClosed)?,
        )
    } else {
        None
    };
    let cancellation = CancellationToken::new();
    let mut connection = CancellableReadConnection::acquire(
        pool,
        Instant::now() + Duration::from_secs(60),
        cancellation.clone(),
    )
    .await?;
    sqlx::query("BEGIN").execute(&mut *connection).await?;
    let owner = RequestSnapshot {
        connection: Arc::new(Mutex::new(Some(connection))),
        cancellation,
    };
    let result = REVISION
        .scope(
            Arc::new(tokio::sync::OnceCell::new()),
            SNAPSHOT.scope(Arc::clone(&owner.connection), read),
        )
        .await;
    // Complete normal rollback before handing the response to the transport.
    // The owner also performs interrupt/rollback cleanup if the future drops.
    if let Some(mut connection) = owner.connection.lock().await.take() {
        if let Err(error) = sqlx::query("ROLLBACK").execute(&mut *connection).await {
            connection.discard().await;
            return Err(error);
        }
        connection.release().await?;
    }
    Ok(result)
}

impl crate::DatabaseManager {
    pub(crate) async fn acquire_read(&self) -> Result<ReadConnection, sqlx::Error> {
        acquire(&self.pool).await
    }
}
