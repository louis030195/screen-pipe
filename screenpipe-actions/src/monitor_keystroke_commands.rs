use tokio::sync::mpsc;
use rdev::{listen, Event, EventType};
use std::sync::Arc;
use tokio::sync::Mutex;

pub enum KeystrokeCommand {
    DoubleSlash,
    // Add other commands as needed
}

pub struct KeystrokeMonitor {
    tx: mpsc::Sender<KeystrokeCommand>,
    last_slash: Arc<Mutex<Option<std::time::Instant>>>,
}

impl KeystrokeMonitor {
    pub fn new(tx: mpsc::Sender<KeystrokeCommand>) -> Self {
        KeystrokeMonitor {
            tx,
            last_slash: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn start_monitoring(&self) -> anyhow::Result<()> {
        let tx = self.tx.clone();
        let last_slash = self.last_slash.clone();

        tokio::task::spawn_blocking(move || {
            if let Err(error) = listen(move |event| {
                let _ = handle_event(event, &tx, &last_slash);
            }) {
                eprintln!("error: {:?}", error)
            }
        });

        Ok(())
    }
}

fn handle_event(
    event: Event,
    tx: &mpsc::Sender<KeystrokeCommand>,
    last_slash: &Arc<Mutex<Option<std::time::Instant>>>
) -> anyhow::Result<()> {
    if let EventType::KeyPress(key) = event.event_type {
        if key == rdev::Key::Slash {
            let mut last_slash_guard = last_slash.blocking_lock();
            let now = std::time::Instant::now();
            if let Some(last) = *last_slash_guard {
                if now.duration_since(last).as_millis() < 500 {
                    tx.blocking_send(KeystrokeCommand::DoubleSlash)?;
                    *last_slash_guard = None;
                    return Ok(());
                }
            }
            *last_slash_guard = Some(now);
        }
    }
    Ok(())
}

pub async fn run_keystroke_monitor(tx: mpsc::Sender<KeystrokeCommand>) -> anyhow::Result<()> {
    let monitor = KeystrokeMonitor::new(tx);
    monitor.start_monitoring().await?;
    Ok(())
}