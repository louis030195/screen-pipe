// screenpipe — AI that knows everything you've seen, said, or heard
// https://screenpipe.com

use super::*;
use crate::DatabaseManager;

pub async fn run_command(
    command: &str,
    root: &Path,
    destination: Option<&Path>,
) -> Result<serde_json::Value, sqlx::Error> {
    let output =
        || destination.ok_or_else(|| storage_error("this operation requires a destination"));
    match command {
        "init" => {
            let db =
                DatabaseManager::new_hybrid(root, Default::default(), Default::default()).await?;
            db.close().await;
            serde_json::to_value(StorageDescriptor::read(root)?).map_err(storage_error)
        }
        "migrate" => {
            serde_json::to_value(migrate(root, Default::default(), Default::default()).await?)
                .map_err(storage_error)
        }
        "compare" => serde_json::to_value(compare(output()?, root).await?).map_err(storage_error),
        "cancel" => {
            cancel_migration(root, Default::default()).await?;
            Ok(serde_json::json!({"cancelled":true}))
        }
        "compact" => {
            compact(root, Default::default()).await?;
            Ok(serde_json::json!({"complete":true}))
        }
        "restore" => {
            restore(root, output()?, Default::default()).await?;
            Ok(serde_json::json!({"complete":true}))
        }
        "export-sqlite" => {
            export_sqlite(root, output()?, Default::default()).await?;
            Ok(serde_json::json!({"complete":true}))
        }
        "status" => serde_json::to_value(StorageDescriptor::read(root)?).map_err(storage_error),
        "verify" | "seal" | "reclaim" | "backup" => {
            let db = DatabaseManager::new(
                root.join("db.sqlite")
                    .to_str()
                    .ok_or_else(|| storage_error("non-UTF8 database path"))?,
                Default::default(),
            )
            .await?;
            let result = async {
                match command {
                    "verify" => {
                        db.verify_storage().await?;
                        Ok(serde_json::json!({"verified":true}))
                    }
                    "seal" => Ok(serde_json::json!({"sealed":db.seal_frame_payloads().await?})),
                    "reclaim" => {
                        Ok(serde_json::json!({"removed":db.reclaim_frame_payloads().await?}))
                    }
                    "backup" => {
                        db.backup_to(
                            output()?
                                .to_str()
                                .ok_or_else(|| storage_error("non-UTF8 backup path"))?,
                        )
                        .await?;
                        Ok(serde_json::json!({"complete":true}))
                    }
                    _ => unreachable!(),
                }
            }
            .await;
            db.close().await;
            result
        }
        _ => Err(storage_error("unknown storage operation")),
    }
}
