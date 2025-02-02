pub const OPENAI_MODEL: &str = "ft:gpt-4o-mini-2024-07-18:personal:unremark:Aq45wBQq"; 

pub const CACHE_FILE_NAME: &str = "unremark_cache.json";

pub const DEFAULT_PROXY_ENDPOINT: &str = "http://localhost:5000";

pub fn get_proxy_endpoint() -> String {
    std::env::var("PROXY_ENDPOINT").unwrap_or_else(|_| DEFAULT_PROXY_ENDPOINT.to_string())
}