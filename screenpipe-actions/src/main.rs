mod monitor_keystroke_commands;
mod call_ai;
mod type_and_animate;
mod screenshot;

use monitor_keystroke_commands::{run_keystroke_monitor, KeystrokeCommand};
use tokio::sync::mpsc;
use tokio::process::Command;
use call_ai::{call_ai_with_screenshot, AIProvider};
use reqwest;
use serde_json::Value;
use anyhow::Context;
use type_and_animate::{type_slowly, delete_characters};
use crate::screenshot::capture_main_window_screenshot;
use base64::{Engine as _, engine::general_purpose};
use image::DynamicImage;

async fn run_swift_script() -> anyhow::Result<String> {
    let output = Command::new("swift")
        .arg("src/print_all_attributes.swift")
        .output()
        .await?;

    if !output.status.success() {
        anyhow::bail!("swift script failed: {}", String::from_utf8_lossy(&output.stderr));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    println!("swift script output: {}", stdout);
    Ok(stdout)
}

async fn search_localhost(query: &str) -> anyhow::Result<String> {
    let client = reqwest::Client::new();
    let url = format!("http://localhost:3030/search?q={}&content_type=all&limit=5&offset=0", query);
    let response = client.get(&url).send().await?.text().await?;
    Ok(response)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("starting keystroke monitor. press '//' to print attributes for the current app and call openai.");
    let (tx, mut rx) = mpsc::channel(100);
    
    tokio::spawn(async move {
        if let Err(error) = run_keystroke_monitor(tx).await {
            eprintln!("error in keystroke monitoring: {:?}", error);
        }
    });

    while let Some(command) = rx.recv().await {
        match command {
            KeystrokeCommand::DoubleSlash => {
                println!("double slash detected. capturing screenshot and calling ai...");

                // Spawn a task for run_swift_script
                let swift_task = tokio::spawn(run_swift_script());

                let screenshot = capture_main_window_screenshot()?;
                type_slowly("thinking").await?;

                let prompt = r#"Based on the desktop screenshot, provide a concise response.
                Output your response in JSON format as follows:
                {
                    "response": "Your response text here"
                }
                The response should be casual, social media style.
                Do no be salesy.
                Be emotional, capture emotions of the context.
                The response should match the length of the message we are responding to.
                It should not look like it was written by AI.
                "#.to_string();

                // Choose the AI provider here
                let provider = AIProvider::OpenAI; // or AIProvider::Claude

                // Wrap the AI call in a Future
                let ai_task = tokio::spawn(call_ai_with_screenshot(provider, prompt, true, screenshot));

                // Wait for both tasks to complete
                let (ai_result, swift_result) = tokio::join!(ai_task, swift_task);

                // Handle AI result
                match ai_result {
                    Ok(ai_response) => match ai_response {
                        Ok(response) => {
                            delete_characters("thinking".len()).await?;
                            delete_characters(2).await?; // Delete the double slash
                            println!("ai screenshot response: {}", response);
                            // Parse the JSON response
                            match serde_json::from_str::<serde_json::Value>(&response) {
                                Ok(json_response) => {
                                    if let Some(response_text) = json_response["response"].as_str() {
                                        type_slowly(response_text).await?;
                                    } else {
                                        eprintln!("response field not found in json or not a string");
                                    }
                                },
                                Err(e) => eprintln!("failed to parse response as json: {:?}", e),
                            }
                        }
                        Err(e) => {
                            delete_characters("thinking".len()).await?;
                            eprintln!("failed to get response from ai: {}", e);
                        }
                    },
                    Err(e) => eprintln!("error joining ai task: {:?}", e),
                }

                // Handle swift result
                match swift_result {
                    Ok(swift_output) => match swift_output {
                        Ok(output) => {
                            println!("swift script output: {}", output);
                            // You can process the swift output here if needed
                        }
                        Err(e) => eprintln!("error running swift script: {:?}", e),
                    },
                    Err(e) => eprintln!("error joining swift task: {:?}", e),
                }
            }
                // type_slowly("\n\nreading window history...").await?;
                
                // match run_swift_script().await {
                //     Ok(swift_output) => {
                //         delete_characters("reading window history...".len()).await?;
                        
                //         let context = format!("output: {}", swift_output);
                //         let prompt = "this is a print out of the entire UI of the active window, we see different attributes with coordinates, find double slash '//' to see where users curor is, 
                //         elements are not necessarily printed in chronological order, find the last message the user should respond to".to_string();
                        
                    //     type_slowly("searching relevant context...").await?;

                    //     match call_openai(prompt, context, false).await {
                    //         Ok(first_response) => {
                    //             delete_characters("searching relevant context...".len()).await?;
                    //             println!("<<<LLM First Call>>>: {}", first_response);
                                
                    //             // Second OpenAI call
                    //             let second_prompt = r#"provide 3 search queries to find relevant context in user files to continue the conversation. follow these guidelines:

                    //             1. focus on specificity, avoid generic words
                    //             2. use noun phrases, target key concepts
                    //             3. employ technical terms when appropriate
                    //             4. consider synonyms and related concepts
                    //             5. utilize proper nouns (names, places, products)
                    //             6. incorporate timeframes if relevant
                    //             7. prioritize unusual or unique words
                    //             8. consider file types if applicable

                    //             return a json object with an array of 3 queries, each query should be one to three words max. format:
                    //             {
                    //             "queries": [
                    //                 "query1",
                    //                 "query2",
                    //                 "query3"
                    //             ]
                    //             }"#.to_string();
                    //             let second_context = format!("conversation: {}", first_response);
                                
                    //             match call_openai(second_prompt, second_context, true).await {
                    //                 Ok(second_response) => {
                    //                     println!("<<<LLM Second Call>>>: {}", second_response);
                                        
                    //                     // parse the json response
                    //                     let queries: Value = serde_json::from_str(&second_response)
                    //                         .context("failed to parse second response as JSON")?;
                                        
                    //                     let mut search_results = Vec::new();

                    //                     type_slowly("analyzing search results...").await?;
                    //                     delete_characters("analyzing search results...".len()).await?;

                    //                     if let Some(queries_array) = queries["queries"].as_array() {
                    //                         for query in queries_array {
                    //                             if let Some(query_str) = query.as_str() {
                    //                                 match search_localhost(query_str).await {
                    //                                     Ok(search_result) => {
                    //                                         let truncated_result = search_result.chars().take(100).collect::<String>();
                    //                                         println!("search result for '{}': {}", query_str, truncated_result);
                    //                                         search_results.push(search_result);
                    //                                     },
                    //                                     Err(e) => eprintln!("error searching localhost for '{}': {:?}", query_str, e),
                    //                                 }
                    //                             }
                    //                         }
                    //                     } else {
                    //                         eprintln!("invalid json format for queries");
                    //                     }

                    //                     // Final LLM call
                    //                     let final_prompt = r#"Based on the conversation and search results, draft a concise response to the user's message.
                    //                     Provide the most relevant response message to continue original conversation even if there are no search results
                    //                     Output your response in JSON format as follows:
                    //                     {
                    //                         "response": "Your response text here"
                    //                     }
                    //                     Ensure the response matches the length and tone of the message we are responding to.
                    //                     Make the response concise and to the point.
                    //                     The response should be casual, social media style.
                    //                     It should not look like it was written by AI.
                    //                     "#.to_string();
                    //                     let final_context = format!("first llm response: {}\nsearch results: {}", first_response, search_results.join("\n"));

                    //                     match call_openai(final_prompt, final_context, true).await {
                    //                         Ok(final_response) => {
                    //                             delete_characters("analyzing search results...".len()).await?;
                    //                             // Parse the JSON response
                    //                             match serde_json::from_str::<serde_json::Value>(&final_response) {
                    //                                 Ok(json_response) => {
                    //                                     if let Some(response_text) = json_response["response"].as_str() {
                    //                                         println!("<<<LLM Final Response>>>: {}", response_text);
                    //                                         type_slowly(response_text).await?;
                    //                                     } else {
                    //                                         eprintln!("response field not found in JSON or not a string");
                    //                                     }
                    //                                 },
                    //                                 Err(e) => eprintln!("failed to parse final response as JSON: {:?}", e),
                    //                             }
                    //                         },
                    //                         Err(e) => eprintln!("error in final openai call: {:?}", e),
                    //                     }
                    //                 },
                    //                 Err(e) => {
                    //                     eprintln!("error in second openai call: {}", e);
                    //                     // Handle the error appropriately, maybe return early or use a default value
                    //                 },
                    //             }
                    //         },
                    //         Err(e) => eprintln!("error in first openai call: {:?}", e),
                    //     }
                    // }
                    // Err(e) => eprintln!("error running swift script: {:?}", e),
                // }
            // }
            // handle other commands if needed
        }
    }

    Ok(())
}