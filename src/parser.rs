use serde::Deserialize;
use std::error::Error;
use std::fs::File;
use std::io::BufReader;

#[derive(Debug, Deserialize, PartialEq, Clone)]
pub struct AuthConfig {
    pub header_name: String,
    pub expected_value: String,
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct Endpoint {
    pub base: String,
    pub path: String,
    pub port: u32,

    #[serde(default)]
    pub strip_prefix: bool,

    pub auth: Option<AuthConfig>,
}

#[derive(Debug, Deserialize, PartialEq)]
pub struct Config {
    pub endpoints: Vec<Endpoint>,
}

pub fn parse_config_from_str(json_str: &str) -> Result<Config, serde_json::Error> {
    serde_json::from_str(json_str)
}

pub fn parse_config_from_file(file_path: &str) -> Result<Config, Box<dyn Error>> {
    let file = File::open(file_path)?;
    let reader = BufReader::new(file);
    let config = serde_json::from_reader(reader)?;
    Ok(config)
}

// ========================================
// TESTS
// ========================================
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_valid_config() {
        let raw_json = r#"
        {
            "endpoints": [
                { "base": "http://localhost:8000", "path": "/api/v1/users", "port": 8081 },
                { "base": "http://localhost:8001", "path": "/api/v1/payments", "port": 8082 }
            ]
        }
        "#;

        let config = parse_config_from_str(raw_json).expect("Failed to parse a valid JSON");

        assert_eq!(config.endpoints.len(), 2);
        assert_eq!(config.endpoints[0].base, "http://localhost:8000");
        assert_eq!(config.endpoints[0].path, "/api/v1/users");
        assert_eq!(config.endpoints[0].port, 8081);
    }

    #[test]
    fn test_parse_missing_field() {
        let raw_json = r#"
        {
            "endpoints": [
                { "path": "/api/v1/users", "port": "1234" }
            ]
        }
        "#;

        let result = parse_config_from_str(raw_json);
        assert!(result.is_err());
    }
}
