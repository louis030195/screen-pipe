use std::path::Path;

use anyhow::Result;
use dirs::home_dir;
use screenpipe_core::{download_pipe, run_pipe};

#[tokio::main]
async fn main() -> Result<()> {
    // Set up the path to the screenpipe directory
    let screenpipe_dir = home_dir().unwrap().join(".screenpipe");

    // The name of the pipe (folder name in examples)
    let pipe_url = "screenpipe-core/examples/simple-ollama-pipe";
    let pipe_name = Path::new(pipe_url).file_name().unwrap().to_str().unwrap();
    println!("Starting the simple Ollama chat pipe...");

    // download the pipe
    download_pipe(pipe_url, screenpipe_dir.clone()).await?;

    // Run the pipe
    match run_pipe(pipe_name, screenpipe_dir).await {
        Ok(_) => println!("Pipe execution completed successfully."),
        Err(e) => eprintln!("Error executing pipe: {}", e),
    }

    Ok(())
}
