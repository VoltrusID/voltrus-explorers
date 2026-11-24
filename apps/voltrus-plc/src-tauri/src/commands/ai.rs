use std::time::Duration;

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::Emitter;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

fn build_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))
}

#[tauri::command]
pub async fn chat_claude_stream(
    window: tauri::Window,
    messages: Vec<ChatMessage>,
    api_key: String,
    model: String,
) -> Result<(), String> {
    let client = build_http_client()?;

    let mut anthropic_messages: Vec<serde_json::Value> = Vec::new();
    let mut system: Option<String> = None;

    for msg in &messages {
        if msg.role == "system" {
            system = Some(msg.content.clone());
        } else {
            anthropic_messages.push(serde_json::json!({
                "role": msg.role,
                "content": msg.content,
            }));
        }
    }

    let mut body = serde_json::json!({
        "model": model,
        "max_tokens": 4096,
        "stream": true,
        "messages": anthropic_messages,
    });

    if let Some(sys) = system {
        body["system"] = serde_json::json!(sys);
    }

    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Claude API request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("Claude API error {}: {}", status, text));
    }

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    while let Some(chunk) = StreamExt::next(&mut stream).await {
        let chunk = chunk.map_err(|e| format!("Stream error: {}", e))?;
        let chunk_str = String::from_utf8_lossy(&chunk);
        buffer.push_str(&chunk_str);

        while let Some(line_end) = buffer.find('\n') {
            let line = buffer[..line_end].trim().to_string();
            buffer = buffer[line_end + 1..].to_string();

            if line.is_empty() || line.starts_with("event:") {
                continue;
            }

            if let Some(data) = line.strip_prefix("data: ") {
                if let Ok(event) = serde_json::from_str::<serde_json::Value>(data) {
                    if event["type"] == "content_block_delta" {
                        if let Some(text) = event["delta"]["text"].as_str() {
                            let _ = window.emit("ai-chunk", text.to_string());
                        }
                    } else if event["type"] == "message_stop" {
                        let _ = window.emit("ai-done", ());
                        return Ok(());
                    } else if event["type"] == "error" {
                        let msg = event["error"]["message"]
                            .as_str()
                            .unwrap_or("Unknown error");
                        let _ = window.emit("ai-done", ());
                        return Err(msg.to_string());
                    }
                }
            }
        }
    }

    let _ = window.emit("ai-done", ());
    Ok(())
}

#[tauri::command]
pub async fn chat_ollama_stream(
    window: tauri::Window,
    messages: Vec<ChatMessage>,
    endpoint: String,
    model: String,
) -> Result<(), String> {
    let client = build_http_client()?;

    let ollama_messages: Vec<serde_json::Value> = messages
        .iter()
        .map(|m| {
            serde_json::json!({
                "role": m.role,
                "content": m.content,
            })
        })
        .collect();

    let body = serde_json::json!({
        "model": model,
        "messages": ollama_messages,
        "stream": true,
    });

    let url = format!("http://{}/api/chat", endpoint.trim_end_matches('/'));

    let response = client
        .post(&url)
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Ollama API request failed: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("Ollama API error {}: {}", status, text));
    }

    let mut stream = response.bytes_stream();
    let mut buffer = String::new();

    while let Some(chunk) = StreamExt::next(&mut stream).await {
        let chunk = chunk.map_err(|e| format!("Stream error: {}", e))?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(line_end) = buffer.find('\n') {
            let line = buffer[..line_end].trim().to_string();
            buffer = buffer[line_end + 1..].to_string();

            if line.is_empty() {
                continue;
            }
            if let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) {
                if let Some(token) = event["message"]["content"].as_str() {
                    if !token.is_empty() {
                        let _ = window.emit("ai-chunk", token.to_string());
                    }
                }
                if event["done"].as_bool() == Some(true) {
                    let _ = window.emit("ai-done", ());
                    return Ok(());
                }
            }
        }
    }

    let _ = window.emit("ai-done", ());
    Ok(())
}
