mod monitor_keystroke_commands;
pub mod call_ai;
pub mod type_and_animate;
pub mod screenshot;

pub use monitor_keystroke_commands::run_keystroke_monitor;
pub use call_ai::{call_ai, call_ai_with_screenshot, AIProvider};
pub use type_and_animate::{type_slowly, delete_characters};
pub use screenshot::capture_main_window_screenshot;