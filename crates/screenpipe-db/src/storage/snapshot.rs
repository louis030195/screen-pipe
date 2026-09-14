// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

//! One bounded SQLite read transaction per logical hybrid read. Immutable
//! Parquet generations remain pinned until the read and response admission end.

use crate::cancellable_query::CancellableReadConnection;
use sqlx::{pool::PoolConnection, Sqlite, SqliteConnection, SqlitePool, Transaction};
use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, OwnedMutexGuard, OwnedRwLockReadGuard};
use tokio_util::sync::CancellationToken;

type SharedConnection = Arc<Mutex<Option<CancellableReadConnection>>>;
struct Context {
    pool: SqlitePool,
    connection: SharedConnection,
    revision: tokio::sync::OnceCell<(i64, i64)>,
    lease: Arc<OwnedRwLockReadGuard<()>>,
}

tokio::task_local! { static SNAPSHOT: Arc<Context>; }

/// Statements within one read borrow its connection serially. Other readers
/// and the existing writer continue on their own SQLite connections.
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
fn context(pool: &SqlitePool) -> Option<Arc<Context>> {
    SNAPSHOT.try_with(Arc::clone).ok().filter(|ctx| {
        // Pool clones share their options address; another manager, including
        // another in-memory database, must never borrow this transaction.
        std::ptr::eq(ctx.pool.options(), pool.options())
    })
}

pub(crate) fn in_snapshot(pool: &SqlitePool) -> bool {
    context(pool).is_some()
}

pub(super) fn lease(pool: &SqlitePool) -> Option<Arc<OwnedRwLockReadGuard<()>>> {
    context(pool).map(|ctx| Arc::clone(&ctx.lease))
}

pub(crate) async fn revision(pool: &SqlitePool) -> Result<(i64, i64), sqlx::Error> {
    let ctx = context(pool).expect("snapshot belongs to this pool");
    let revision = ctx.revision.get_or_try_init(|| async {
        sqlx::query_as("SELECT revision,(SELECT revision FROM _storage_revocation WHERE id=1) FROM storage_metadata WHERE singleton=1")
            .fetch_one(&mut *acquire(pool).await?).await
    }).await?;
    Ok(*revision)
}

async fn scoped(pool: &SqlitePool) -> Option<ReadConnection> {
    let ctx = context(pool)?;
    Some(ReadConnection::Snapshot(
        Arc::clone(&ctx.connection).lock_owned().await,
    ))
}
pub(crate) async fn acquire(pool: &SqlitePool) -> Result<ReadConnection, sqlx::Error> {
    if let Some(c) = scoped(pool).await {
        return Ok(c);
    }
    Ok(ReadConnection::Pool(pool.acquire().await?))
}
pub(crate) async fn search(pool: &SqlitePool) -> Result<ReadConnection, sqlx::Error> {
    if let Some(c) = scoped(pool).await {
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
    if let Some(c) = scoped(pool).await {
        return Ok(c);
    }
    Ok(ReadConnection::Transaction(pool.begin().await?))
}

struct RequestSnapshot {
    context: Arc<Context>,
    lane: Option<tokio::sync::OwnedSemaphorePermit>,
    cancellation: CancellationToken,
}
impl Drop for RequestSnapshot {
    fn drop(&mut self) {
        self.cancellation.cancel();
        let context = Arc::clone(&self.context);
        let lane = self.lane.take();
        tokio::spawn(async move {
            let _lane = lane;
            if let Some(mut c) = context.connection.lock().await.take() {
                if sqlx::query("ROLLBACK").execute(&mut *c).await.is_err() {
                    c.discard().await;
                } else {
                    let _ = c.release().await;
                }
            }
        });
    }
}

pub(super) async fn run<T>(
    db: &crate::DatabaseManager,
    read: impl std::future::Future<Output = T>,
) -> Result<T, sqlx::Error> {
    let storage = db.storage.as_ref().expect("hybrid read");
    let pool = &db.pool;
    let lanes = storage.read_lanes.get_or_init(|| {
        Arc::new(tokio::sync::Semaphore::new(
            pool.options()
                .get_max_connections()
                .saturating_sub(1)
                .max(1) as usize,
        ))
    });
    let lane = tokio::select! {
        biased;
        _ = storage.closing.cancelled() => return Err(sqlx::Error::PoolClosed),
        lane = Arc::clone(lanes).acquire_owned() => lane.map_err(|_| sqlx::Error::PoolClosed)?,
    };
    let lease = tokio::select! {
        biased;
        _ = storage.closing.cancelled() => return Err(sqlx::Error::PoolClosed),
        lease = Arc::clone(&storage.leases).read_owned() => Arc::new(lease),
    };
    let cancellation = storage.closing.child_token();
    let mut connection = CancellableReadConnection::acquire(
        pool,
        Instant::now() + Duration::from_secs(storage.descriptor.budget.operation_timeout_secs),
        cancellation.clone(),
    )
    .await?;
    sqlx::query("BEGIN").execute(&mut *connection).await?;
    let owner = RequestSnapshot {
        context: Arc::new(Context {
            pool: pool.clone(),
            connection: Arc::new(Mutex::new(Some(connection))),
            revision: tokio::sync::OnceCell::new(),
            lease,
        }),
        lane: Some(lane),
        cancellation,
    };
    let (token, result) = SNAPSHOT
        .scope(Arc::clone(&owner.context), async {
            let token = db.storage_read_token().await?;
            Ok::<_, sqlx::Error>((token, read.await))
        })
        .await?;
    // Finish the transaction before final admission, keeping the file pin.
    // Dropping this future interrupts SQLite and rolls back before pool reuse.
    if let Some(mut connection) = owner.context.connection.lock().await.take() {
        if let Err(error) = sqlx::query("ROLLBACK").execute(&mut *connection).await {
            connection.discard().await;
            return Err(error);
        }
        connection.release().await?;
    }
    let _admission = token.admit(pool).await?;
    Ok(result)
}

impl crate::DatabaseManager {
    /// Share a SQLite snapshot across candidate selection, counts and payload
    /// hydration. Ordinary writes do not revoke it; deletion and privacy do.
    pub async fn read_snapshot<T>(
        &self,
        read: impl std::future::Future<Output = T>,
    ) -> Result<T, sqlx::Error> {
        // Query futures contain nested search fan-out and hydration state.
        // Keep that state on the heap across snapshot scopes so native Tokio
        // workers retain their normal stack size.
        let read = Box::pin(read);
        let Some(storage) = &self.storage else {
            return Ok(read.await);
        };
        if in_snapshot(&self.pool) {
            return Ok(read.await);
        }
        tokio::time::timeout(
            Duration::from_secs(storage.descriptor.budget.operation_timeout_secs),
            run(self, read),
        )
        .await
        .map_err(|_| super::storage_error("payload read deadline exceeded"))?
    }

    pub(crate) async fn acquire_read(&self) -> Result<ReadConnection, sqlx::Error> {
        acquire(&self.pool).await
    }
}
