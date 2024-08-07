pub mod core;
mod db;
pub mod logs;
mod plugin;
mod resource_monitor;
mod server;
mod video;

pub use core::{start_continuous_recording, RecordingState};
pub use db::{ContentType, DatabaseManager, SearchResult};
pub use logs::MultiWriter;
pub use resource_monitor::{ResourceMonitor, RestartSignal};
pub use server::health_check;
pub use server::AppState;
pub use server::HealthCheckResponse;
pub use server::Server;
pub use video::VideoCapture;
