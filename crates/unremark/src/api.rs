use crate::types::{CommentInfo, ApiError};
use reqwest::StatusCode;
use std::time::Duration;
use tokio::time::sleep;
use log::debug;

pub(crate) async fn make_api_request(
    client: &reqwest::Client,
    api_key: &str,
    comment: &CommentInfo,
) -> Result<serde_json::Value, ApiError> {
    let max_retries = 3;
    let mut retry_delay = Duration::from_millis(1000);

    for attempt in 0..max_retries {
        if attempt > 0 {
            debug!("Retrying request (attempt {}/{})", attempt + 1, max_retries);
            sleep(retry_delay).await;
            retry_delay *= 2;
        }

        let message = serde_json::json!({
            "model": "ft:gpt-4o-mini-2024-07-18:personal:unremark:Aq45wBQq",
            "messages": [{
                "role": "user",
                "content": format!(
                    "Comment: '{}'\nContext: '{}'\nLine Number: {}\nIs this comment redundant or useful? Please respond with a JSON object containing the following fields: is_redundant, comment_line_number, comment_text, explanation",
                    comment.text,
                    comment.context,
                    comment.line_number
                )
            }],
            "max_tokens": 500,
            "temperature": 0.0,
            "top_p": 1.0,
            "n": 1,
            "stream": false
        });

        match client
            .post("https://api.openai.com/v1/chat/completions")
            .header("Authorization", format!("Bearer {}", api_key))
            .json(&message)
            .send()
            .await
        {
            Ok(response) => {
                match response.status() {
                    StatusCode::OK => {
                        return response.json().await.map_err(|e| {
                            ApiError::Other(format!("Failed to parse response: {}", e))
                        });
                    }
                    StatusCode::TOO_MANY_REQUESTS => {
                        if attempt == max_retries - 1 {
                            return Err(ApiError::RateLimit(
                                "Rate limit exceeded after all retries".to_string(),
                            ));
                        }
                        if let Some(retry_after) = response.headers()
                            .get("retry-after")
                            .and_then(|h| h.to_str().ok())
                            .and_then(|s| s.parse::<u64>().ok())
                        {
                            retry_delay = Duration::from_secs(retry_after);
                        }
                        continue;
                    }
                    status => {
                        if attempt == max_retries - 1 {
                            return Err(ApiError::Other(
                                format!("Request failed with status: {}", status),
                            ));
                        }
                        continue;
                    }
                }
            }
            Err(e) => {
                if e.is_timeout() {
                    if attempt == max_retries - 1 {
                        return Err(ApiError::Timeout(
                            "Request timed out after all retries".to_string(),
                        ));
                    }
                } else if e.is_connect() {
                    if attempt == max_retries - 1 {
                        return Err(ApiError::Network(
                            "Failed to connect after all retries".to_string(),
                        ));
                    }
                } else {
                    if attempt == max_retries - 1 {
                        return Err(ApiError::Other(
                            format!("Request failed: {}", e),
                        ));
                    }
                }
                continue;
            }
        }
    }

    Err(ApiError::Other("Maximum retries exceeded".to_string()))
}