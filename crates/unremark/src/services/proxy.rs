use async_trait::async_trait;
use serde::{Serialize, Deserialize};
use reqwest::Client;
use crate::types::CommentInfo;
use crate::constants::PROXY_ENDPOINT;

#[derive(Debug, Serialize)]
struct ProxyRequest {
    comments: Vec<CommentInfo>,
}

#[derive(Debug, Deserialize)]
struct ProxyResponse {
    redundant_comments: Vec<CommentInfo>,
}

#[async_trait]
pub trait AnalysisService: Send + Sync {
    async fn analyze_comments_with_proxy(&self, comments: Vec<CommentInfo>) -> Result<Vec<CommentInfo>, String>;
}

pub struct ProxyAnalysisService {
    pub endpoint: String,
}

#[async_trait]
impl AnalysisService for ProxyAnalysisService {
    async fn analyze_comments_with_proxy(&self, comments: Vec<CommentInfo>) -> Result<Vec<CommentInfo>, String> {
        let client = Client::new();
        
        let request = ProxyRequest { comments };

        let response = client
            .post(&format!("{}/analyze", self.endpoint))
            .json(&request)
            .send()
            .await
            .map_err(|e| format!("Proxy request failed: {}", e))?;

        if !response.status().is_success() {
            return Err(format!("Proxy error: {}", response.status()));
        }

        let analysis: ProxyResponse = response
            .json()
            .await
            .map_err(|e| format!("Failed to parse proxy response: {}", e))?;

        Ok(analysis.redundant_comments)
    }
}

pub fn create_analysis_service(proxy_endpoint: Option<String>) -> Box<dyn AnalysisService + Send + Sync> {
    Box::new(ProxyAnalysisService {
        endpoint: proxy_endpoint.unwrap_or_else(|| PROXY_ENDPOINT.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_proxy_service() {
        let service = ProxyAnalysisService {
            endpoint: PROXY_ENDPOINT.to_string(),
        };

        let comments = vec![
            CommentInfo {
                text: "// Adds two numbers".to_string(),
                line_number: 1,
                context: "fn add(a: i32, b: i32) -> i32 { a + b }".to_string(),
            },
            CommentInfo {
                text: "// Returns the sum".to_string(),
                line_number: 2,
                context: "a + b".to_string(),
            },
        ];

        let result = service.analyze_comments_with_proxy(comments).await;
        assert!(result.is_ok());
    }
} 