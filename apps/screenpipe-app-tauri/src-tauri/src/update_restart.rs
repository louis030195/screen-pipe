// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use std::time::Duration;
use tokio::sync::{RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Storage recovery is restartable, but exiting during native model creation
/// can race ONNX Runtime's process destructors (#3622). Hold this barrier from
/// audio initialization through the end of ServerCore startup. An updater that
/// wins during recovery prevents startup from entering native initialization.
pub(crate) static RESTART_SAFETY: RestartSafety = RestartSafety(RwLock::const_new(()));

pub(crate) struct RestartSafety(RwLock<()>);

impl RestartSafety {
    pub(crate) async fn native_startup(&self) -> RwLockReadGuard<'_, ()> {
        self.0.read().await
    }

    pub(crate) async fn prepare_restart(
        &self,
        timeout: Duration,
    ) -> Option<RwLockWriteGuard<'_, ()>> {
        tokio::time::timeout(timeout, self.0.write()).await.ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn recovery_does_not_block_updates_and_cannot_start_native_init_after_handoff() {
        let safety = RestartSafety(RwLock::new(()));
        // Database recovery does not hold the native initialization barrier.
        let update = safety
            .prepare_restart(Duration::from_secs(1))
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(10), safety.native_startup())
                .await
                .is_err()
        );
        // A failed/cancelled install releases the reservation, allowing the
        // original startup to continue without relaunching or losing progress.
        drop(update);
        let _startup = tokio::time::timeout(Duration::from_secs(1), safety.native_startup())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn native_initialization_still_defers_restart_until_it_finishes() {
        let safety = RestartSafety(RwLock::new(()));
        let startup = safety.native_startup().await;
        assert!(safety
            .prepare_restart(Duration::from_millis(10))
            .await
            .is_none());
        drop(startup);
        assert!(safety
            .prepare_restart(Duration::from_secs(1))
            .await
            .is_some());
    }
}
