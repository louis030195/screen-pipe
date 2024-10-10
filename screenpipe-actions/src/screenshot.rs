use anyhow::{Result, Context};
use image::{DynamicImage, ImageBuffer};
use xcap::Window;

pub fn capture_main_window_screenshot() -> Result<DynamicImage> {
    let windows = Window::all().context("failed to list windows")?;

    let focused_window = windows
        .into_iter()
        .find(is_valid_window)
        .context("no valid focused window found")?;

    let buffer = focused_window.capture_image().context("failed to capture image")?;
    let image = DynamicImage::ImageRgba8(
        ImageBuffer::from_raw(buffer.width() as u32, buffer.height() as u32, buffer.into_raw())
            .context("failed to create image from buffer")?,
    );

    // save the screenshot to the current folder
    image.save("screenshot.png").context("failed to save screenshot")?;

    Ok(image)
}

fn is_valid_window(window: &Window) -> bool {
    let not_minimized = !window.is_minimized();
    let not_window_server = window.app_name() != "Window Server";
    let not_contexts = window.app_name() != "Contexts";
    let has_title = !window.title().is_empty();

    not_minimized && not_window_server && not_contexts && has_title
}