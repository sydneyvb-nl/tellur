//! Authentication primitives: roles, API tokens, and password hashing.
//!
//! API tokens look like `tlr_<token_id>_<secret>`:
//! - `token_id` is a public lookup key (indexes the DB row),
//! - `secret` is high-entropy and stored only as an **Argon2id** hash.
//!
//! Splitting id from secret lets us look up a single row and then verify the
//! secret in constant work, instead of scanning and comparing every token.

use anyhow::{Result, bail};
use argon2::{
    Argon2, PasswordHasher, PasswordVerifier,
    password_hash::{PasswordHash, SaltString},
};
use rand_core::{OsRng, RngCore};

const TOKEN_PREFIX: &str = "tlr_";
const TOKEN_ID_BYTES: usize = 16;
const TOKEN_SECRET_BYTES: usize = 32;

/// Access role. Ordered from least to most privileged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Viewer,
    Contributor,
    Admin,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Contributor => "contributor",
            Role::Admin => "admin",
        }
    }

    pub fn parse(value: &str) -> Result<Role> {
        match value {
            "viewer" => Ok(Role::Viewer),
            "contributor" => Ok(Role::Contributor),
            "admin" => Ok(Role::Admin),
            other => bail!("unknown role: {other}"),
        }
    }

    /// Privilege rank for `>=` comparisons.
    fn rank(self) -> u8 {
        match self {
            Role::Viewer => 0,
            Role::Contributor => 1,
            Role::Admin => 2,
        }
    }

    /// True if this role meets or exceeds `required`.
    pub fn allows(self, required: Role) -> bool {
        self.rank() >= required.rank()
    }

    /// The more privileged of two roles. Used to combine an org-baseline role
    /// with an additive per-repo grant (grants elevate, never restrict).
    pub fn max(self, other: Role) -> Role {
        if self.rank() >= other.rank() {
            self
        } else {
            other
        }
    }
}

/// An authenticated caller: who they are, in which org, with what role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Principal {
    pub org_id: String,
    pub member_id: String,
    pub role: Role,
}

/// A freshly generated token. The plaintext is returned exactly once; only
/// `token_id` and `secret_hash` are persisted.
pub struct GeneratedToken {
    /// Full token string to hand to the user (shown once).
    pub plaintext: String,
    /// Public lookup id (stored).
    pub token_id: String,
    /// Argon2id PHC hash of the secret (stored).
    pub secret_hash: String,
}

/// Generate a new API token.
pub fn generate_token() -> Result<GeneratedToken> {
    let token_id = random_hex(TOKEN_ID_BYTES);
    let secret = random_hex(TOKEN_SECRET_BYTES);
    let secret_hash = hash_secret(&secret)?;
    Ok(GeneratedToken {
        plaintext: format!("{TOKEN_PREFIX}{token_id}_{secret}"),
        token_id,
        secret_hash,
    })
}

/// Split a token string into `(token_id, secret)`, or `None` if malformed.
pub fn parse_token(token: &str) -> Option<(String, String)> {
    let rest = token.strip_prefix(TOKEN_PREFIX)?;
    let (id, secret) = rest.split_once('_')?;
    if id.is_empty() || secret.is_empty() {
        return None;
    }
    Some((id.to_string(), secret.to_string()))
}

/// Hash a secret with Argon2id (returns a PHC string).
pub fn hash_secret(secret: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(secret.as_bytes(), &salt)
        .map_err(|e| anyhow::anyhow!("argon2 hashing failed: {e}"))?;
    Ok(hash.to_string())
}

/// Verify a secret against a stored Argon2id PHC hash. Never panics or leaks.
pub fn verify_secret(secret: &str, phc: &str) -> bool {
    match PasswordHash::new(phc) {
        Ok(parsed) => Argon2::default()
            .verify_password(secret.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    OsRng.fill_bytes(&mut buf);
    to_hex(&buf)
}

fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn role_ordering() {
        assert!(Role::Admin.allows(Role::Viewer));
        assert!(Role::Contributor.allows(Role::Contributor));
        assert!(!Role::Viewer.allows(Role::Admin));
        assert_eq!(Role::parse("admin").unwrap(), Role::Admin);
        assert!(Role::parse("root").is_err());
    }

    #[test]
    fn role_max_picks_higher_privilege() {
        assert_eq!(Role::Viewer.max(Role::Contributor), Role::Contributor);
        assert_eq!(Role::Admin.max(Role::Viewer), Role::Admin);
        assert_eq!(Role::Contributor.max(Role::Contributor), Role::Contributor);
    }

    #[test]
    fn token_roundtrip_and_verify() {
        let t = generate_token().unwrap();
        assert!(t.plaintext.starts_with("tlr_"));
        let (id, secret) = parse_token(&t.plaintext).unwrap();
        assert_eq!(id, t.token_id);
        assert!(verify_secret(&secret, &t.secret_hash));
        assert!(!verify_secret("wrong", &t.secret_hash));
    }

    #[test]
    fn malformed_tokens_rejected() {
        assert!(parse_token("nope").is_none());
        assert!(parse_token("tlr_only").is_none());
        assert!(parse_token("tlr__nosecret").is_none());
        assert!(parse_token("tlr_id_").is_none());
    }

    #[test]
    fn tokens_are_unique() {
        let a = generate_token().unwrap();
        let b = generate_token().unwrap();
        assert_ne!(a.plaintext, b.plaintext);
        assert_ne!(a.token_id, b.token_id);
    }
}
