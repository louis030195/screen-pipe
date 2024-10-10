use anyhow::{Result, Context};
use dotenv::dotenv;
use reqwest::Client;
use serde_json::{json, Value};

pub async fn call_openai(prompt: String, context: String, expect_json: bool) -> Result<String> {
    dotenv().ok(); // Load .env file
    let api_key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY not set");
    
    let client = Client::new();

    let messages = vec![
        json!({
            "role": "system",
            "content": context
        }),
        json!({
            "role": "user",
            "content": prompt
        })
    ];

    let mut body = json!({
        "model": "gpt-4o",
        "messages": messages,
        "temperature": 0.2
    });

    if expect_json {
        // Only add response_format if expect_json is true
        body["response_format"] = json!({"type": "json_object"});
    }

    let response: Value = client.post("https://api.openai.com/v1/chat/completions")
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .send()
        .await?
        .json()
        .await?;

    // Log the entire response for debugging
    // println!("raw api response: {}", serde_json::to_string_pretty(&response)?);

    if response.get("error").is_some() {
        // If there's an error, return it as a string
        let error_message = response["error"]["message"]
            .as_str()
            .unwrap_or("Unknown error")
            .to_string();
        return Err(anyhow::anyhow!(error_message));
    }

    let content = response["choices"]
        .get(0)
        .and_then(|choice| choice["message"]["content"].as_str())
        .context("failed to extract content from response")?
        .to_string();

    if expect_json {
        // Parse the content as JSON to ensure it's valid
        serde_json::from_str::<Value>(&content)
            .context("response content is not valid JSON")?;
    }

    Ok(content)
}