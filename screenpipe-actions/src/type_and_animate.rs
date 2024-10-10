use tokio::time::sleep;
use std::time::Duration;
use anyhow::Result;
use enigo::{Enigo, KeyboardControllable};

async fn type_text(text: &str) -> Result<()> {
    let mut enigo = Enigo::new();
    for c in text.chars() {
        enigo.key_sequence(&c.to_string());
        // sleep(Duration::from_millis(1)).await;
    }
    Ok(())
}

pub async fn delete_characters(count: usize) -> Result<()> {
    let mut enigo = Enigo::new();
    for _ in 0..count {
        enigo.key_click(enigo::Key::Backspace);
    }
    Ok(())
}

pub async fn type_slowly(text: &str) -> Result<()> {
    for c in text.chars() {
        type_text(&c.to_string()).await?;
        sleep(Duration::from_millis(1)).await;
    }
    Ok(())
}

