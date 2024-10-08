use log::{debug, error};
use std::path::PathBuf;
use which::which;
use ffmpeg_sidecar::{
    command::ffmpeg_is_installed,
    download::{check_latest_version, download_ffmpeg_package, ffmpeg_download_url, unpack_ffmpeg},
    paths::sidecar_dir,
    version::ffmpeg_version,
};
#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(not(windows))]
const EXECUTABLE_NAME: &str = "ffmpeg";

#[cfg(windows)]
const EXECUTABLE_NAME: &str = "ffmpeg.exe";

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

pub fn find_ffmpeg_path() -> Option<PathBuf> {
    debug!("Starting search for ffmpeg executable");

    // Check if `ffmpeg` is in the PATH environment variable
    if let Ok(path) = which(EXECUTABLE_NAME) {
        debug!("Found ffmpeg in PATH: {:?}", path);
        return Some(path);
    }
    debug!("ffmpeg not found in PATH");

    // Check in current working directory
    if let Ok(cwd) = std::env::current_dir() {
        debug!("Current working directory: {:?}", cwd);
        let ffmpeg_in_cwd = cwd.join(EXECUTABLE_NAME);
        if ffmpeg_in_cwd.is_file() && ffmpeg_in_cwd.exists() {
            debug!(
                "Found ffmpeg in current working directory: {:?}",
                ffmpeg_in_cwd
            );
            return Some(ffmpeg_in_cwd);
        }
        debug!("ffmpeg not found in current working directory");
    }

    // Check in the same folder as the executable
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_folder) = exe_path.parent() {
            debug!("Executable folder: {:?}", exe_folder);
            let ffmpeg_in_exe_folder = exe_folder.join(EXECUTABLE_NAME);
            if ffmpeg_in_exe_folder.exists() {
                debug!(
                    "Found ffmpeg in executable folder: {:?}",
                    ffmpeg_in_exe_folder
                );
                return Some(ffmpeg_in_exe_folder);
            }
            debug!("ffmpeg not found in executable folder");

            // Platform-specific checks
            #[cfg(target_os = "macos")]
            {
                let resources_folder = exe_folder.join("../Resources");
                debug!("Resources folder: {:?}", resources_folder);
                let ffmpeg_in_resources = resources_folder.join(EXECUTABLE_NAME);
                if ffmpeg_in_resources.exists() {
                    debug!(
                        "Found ffmpeg in Resources folder: {:?}",
                        ffmpeg_in_resources
                    );
                    return Some(ffmpeg_in_resources);
                }
                debug!("ffmpeg not found in Resources folder");
            }

            #[cfg(target_os = "linux")]
            {
                let lib_folder = exe_folder.join("lib");
                debug!("Lib folder: {:?}", lib_folder);
                let ffmpeg_in_lib = lib_folder.join(EXECUTABLE_NAME);
                if ffmpeg_in_lib.exists() {
                    debug!("Found ffmpeg in lib folder: {:?}", ffmpeg_in_lib);
                    return Some(ffmpeg_in_lib);
                }
                debug!("ffmpeg not found in lib folder");
            }
        }
    }

    debug!("FFmpeg not found. Attempting to install...");

    if let Err(error) = handle_ffmpeg_installation() {
        error!("Failed to install FFmpeg: {}", error);
        return None;
    }

    // Check again after installation
    if let Ok(path) = which(EXECUTABLE_NAME) {
        debug!("Found ffmpeg after installation in PATH: {:?}", path);
        return Some(path);
    }

    error!("FFmpeg not found even after installation");
    None // Return None if ffmpeg is not found
}

fn handle_ffmpeg_installation() -> Result<(), String> {
    if ffmpeg_is_installed() {
        debug!("FFmpeg is already installed! 🎉");
        return Ok(());
    }

    debug!("FFmpeg not found. Attempting to install...");
    match check_latest_version() {
        Ok(version) => debug!("Latest available version: {}", version),
        Err(e) => debug!("Skipping version check due to error: {e}"),
    }

    let download_url = ffmpeg_download_url().map_err(|e| e.to_string())?;
    let destination = sidecar_dir().map_err(|e| e.to_string())?;

    debug!("Downloading from: {:?}", download_url);
    let archive_path =
        download_ffmpeg_package(download_url, &destination).map_err(|e| e.to_string())?;
    debug!("Downloaded package: {:?}", archive_path);

    debug!("Extracting...");
    unpack_ffmpeg(&archive_path, &destination).map_err(|e| e.to_string())?;

    let version = ffmpeg_version().map_err(|e| e.to_string())?;

    debug!("Done! Installed FFmpeg version {} 🏁", version);
    Ok(())
}
