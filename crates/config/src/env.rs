//! Environment configuration module
//!
//! Loads API keys and secrets from .env file

use std::env;

/// Initialize environment from .env file
pub fn init() {
    dotenvy::dotenv().ok();
}

/// Get required environment variable, panics if not found
pub fn require(key: &str) -> String {
    env::var(key).expect(&format!("Environment variable '{}' not set", key))
}

/// Get optional environment variable
pub fn optional(key: &str) -> Option<String> {
    env::var(key).ok()
}

/// Get with fallback
pub fn get(key: &str, fallback: &str) -> String {
    env::var(key).unwrap_or_else(|_| fallback.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_with_fallback() {
        assert_eq!(get("NONEXISTENT_KEY_12345", "default"), "default");
    }
}
