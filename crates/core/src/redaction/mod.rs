//! Redaction engine — secret detection and content sanitization

use regex::Regex;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RedactionConfig {
    pub mode: RedactionMode,
    pub hash_prompts: bool,
    #[serde(default)]
    pub store_prompt_excerpt: bool,
    #[serde(default)]
    pub redact_patterns: Vec<String>,
    #[serde(default)]
    pub redact_paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RedactionMode {
    None,
    Automatic,
    Strict,
    HashOnly,
    Custom,
}

impl Default for RedactionConfig {
    fn default() -> Self {
        Self {
            mode: RedactionMode::Automatic,
            hash_prompts: true,
            store_prompt_excerpt: false,
            redact_patterns: vec![
                r"(?i)api[_-]?key\s*[=:]\s*\S{10,}".to_string(),
                r"(?i)secret[_-]?key\s*[=:]\s*\S{10,}".to_string(),
                r"(?i)password\s*[=:]\s*\S{6,}".to_string(),
                r"(?i)token\s*[=:]\s*\S{15,}".to_string(),
                r"(?i)bearer\s+\S{15,}".to_string(),
                r"sk-[a-zA-Z0-9]{20,}".to_string(),
                r"ghp_[a-zA-Z0-9]{36}".to_string(),
                r"AKIA[0-9A-Z]{16}".to_string(),
                r#"-----BEGIN (?:RSA |EC )?PRIVATE KEY-----"#.to_string(),
            ],
            redact_paths: vec![
                ".env*".to_string(),
                "**/*.pem".to_string(),
                "**/*.key".to_string(),
            ],
        }
    }
}

#[derive(Debug, Clone)]
pub struct RedactionResult {
    pub has_secrets: bool,
    pub findings: Vec<SecretFinding>,
    pub redacted_content: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SecretFinding {
    pub pattern_name: String,
    pub line_number: Option<usize>,
}

pub struct RedactionEngine {
    config: RedactionConfig,
    compiled_patterns: Vec<Regex>,
}

impl RedactionEngine {
    pub fn new(config: RedactionConfig) -> Self {
        let compiled_patterns = config
            .redact_patterns
            .iter()
            .filter_map(|p| Regex::new(p).ok())
            .collect();
        Self { config, compiled_patterns }
    }

    pub fn default_engine() -> Self {
        Self::new(RedactionConfig::default())
    }

    pub fn scan(&self, content: &str) -> Vec<SecretFinding> {
        let mut findings = Vec::new();
        for (line_num, line) in content.lines().enumerate() {
            for pattern in &self.compiled_patterns {
                if pattern.is_match(line) {
                    findings.push(SecretFinding {
                        pattern_name: pattern.to_string(),
                        line_number: Some(line_num + 1),
                    });
                }
            }
        }
        findings
    }

    pub fn scan_and_redact(&self, content: &str) -> RedactionResult {
        if self.config.mode == RedactionMode::None {
            return RedactionResult {
                has_secrets: false,
                findings: Vec::new(),
                redacted_content: Some(content.to_string()),
            };
        }

        let findings = self.scan(content);
        let has_secrets = !findings.is_empty();

        let redacted = if has_secrets {
            let mut redacted = content.to_string();
            for pattern in &self.compiled_patterns {
                redacted = pattern.replace_all(&redacted, "[REDACTED]").to_string();
            }
            Some(redacted)
        } else {
            Some(content.to_string())
        };

        RedactionResult { has_secrets, findings, redacted_content: redacted }
    }

    pub fn is_sensitive_path(&self, file_path: &str) -> bool {
        for pattern in &self.config.redact_paths {
            if crate::glob::glob_match(pattern, file_path) {
                return true;
            }
        }
        // Also check well-known sensitive file extensions.
        let sensitive_extensions = [".pem", ".key", ".p12", ".pfx", ".jks"];
        if sensitive_extensions.iter().any(|ext| file_path.ends_with(ext)) {
            return true;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_api_key() {
        let engine = RedactionEngine::default_engine();
        let findings = engine.scan("api_key=sk-abclongkeyvalue12345");
        assert!(!findings.is_empty());
    }

    #[test]
    fn test_detect_aws_key() {
        let engine = RedactionEngine::default_engine();
        let findings = engine.scan("aws_access_key_id=AKIAIOSFODNN7EXAMPLE");
        assert!(!findings.is_empty());
    }

    #[test]
    fn test_detect_private_key() {
        let engine = RedactionEngine::default_engine();
        let content = "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA";
        let findings = engine.scan(content);
        assert!(!findings.is_empty());
    }

    #[test]
    fn test_no_secrets() {
        let engine = RedactionEngine::default_engine();
        let findings = engine.scan("const x = 42;");
        assert!(findings.is_empty());
    }

    #[test]
    fn test_redact_content() {
        let engine = RedactionEngine::default_engine();
        let result = engine.scan_and_redact("api_key=sk-abclongkeyvalue12345\nconst x = 42;");
        assert!(result.has_secrets);
        assert!(result.redacted_content.unwrap().contains("[REDACTED]"));
    }

    #[test]
    fn test_sensitive_path() {
        let engine = RedactionEngine::default_engine();
        assert!(engine.is_sensitive_path(".env"));
        assert!(engine.is_sensitive_path(".env.production"));
        assert!(!engine.is_sensitive_path("src/main.rs"));
    }

    #[test]
    fn test_mode_none() {
        let config = RedactionConfig {
            mode: RedactionMode::None,
            ..Default::default()
        };
        let engine = RedactionEngine::new(config);
        let result = engine.scan_and_redact("api_key=sk-abclongkeyvalue12345");
        assert!(!result.has_secrets);
    }
}
