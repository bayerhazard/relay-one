//! AES-256-GCM encryption-at-rest for sensitive data (passwords).
//!
//! # Design
//! - Server version: encryption key is a file in the data directory
//!   (`/data/Relay/encryption.key`, mode 0600). No OS keychain — the web
//!   server has no keychain, and the data directory is the single backup root.
//! - Encrypted values are stored as `$aes-gcm$<base64(nonce || ciphertext)>`.
//! - The prefix makes encrypted vs. plaintext values unambiguous for migration.
//! - `decrypt()` transparently handles plaintext values (returns as-is) so
//!   existing plaintext passwords continue to work until re-encrypted.

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, Nonce,
};
use aes_gcm::aead::rand_core::RngCore;
use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use std::path::Path;
use std::sync::OnceLock;

const KEY_FILE_NAME: &str = "encryption.key";
const ENCRYPTED_PREFIX: &str = "$aes-gcm$";

/// Global encryption key, initialized once at app startup.
static ENCRYPTION_KEY: OnceLock<[u8; 32]> = OnceLock::new();

/// Path to the encryption key file within the data directory.
fn key_path(app_data_dir: &Path) -> std::path::PathBuf {
    app_data_dir.join(KEY_FILE_NAME)
}

/// Generate a new random 256-bit key.
fn generate_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    key
}

/// Load key from file (0600) or create it.
fn load_or_create_key(app_data_dir: &Path) -> Result<[u8; 32], String> {
    let path = key_path(app_data_dir);

    // 1. Existing file
    if path.exists() {
        let data = std::fs::read(&path).map_err(|e| format!("Encryption key read failed: {e}"))?;
        if data.len() == 32 {
            let mut key = [0u8; 32];
            key.copy_from_slice(&data);
            tracing::info!("Encryption key loaded from {:?}", path);
            return Ok(key);
        }
        return Err("Encryption key file has unexpected length".into());
    }

    // 2. Create new key file
    let key = generate_key();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Err(e) = std::fs::write(&path, &key) {
        return Err(format!("Encryption key write failed: {e}"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    tracing::info!("Created new encryption key at {:?}", path);
    Ok(key)
}

/// Initialize the global encryption key.
///
/// Must be called once at startup. Safe to call multiple times — subsequent
/// calls are no-ops.
pub fn init_key(app_data_dir: &Path) -> Result<(), String> {
    if ENCRYPTION_KEY.get().is_some() {
        return Ok(());
    }
    let key = load_or_create_key(app_data_dir)?;
    ENCRYPTION_KEY
        .set(key)
        .map_err(|_| "Encryption key already initialized".to_string())
}

/// Return a reference to the global encryption key.
fn get_key() -> Result<&'static [u8; 32], String> {
    ENCRYPTION_KEY
        .get()
        .ok_or_else(|| "Encryption key not initialized. Call crypto::init_key() first.".to_string())
}

/// Encrypt `plaintext` using AES-256-GCM.
///
/// Returns a string in the format `$aes-gcm$<base64(nonce || ciphertext)>`.
/// Each call generates a fresh random 12-byte nonce.
pub fn encrypt(plaintext: &str) -> Result<String, String> {
    let key = get_key()?;
    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|e| format!("Failed to create cipher: {e}"))?;

    // Generate a random 96-bit nonce
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| format!("Encryption failed: {e}"))?;

    // Format: nonce (12) || ciphertext (includes 16-byte GCM tag)
    let mut combined = Vec::with_capacity(12 + ciphertext.len());
    combined.extend_from_slice(&nonce_bytes);
    combined.extend_from_slice(&ciphertext);

    let encoded = BASE64.encode(&combined);
    Ok(format!("{ENCRYPTED_PREFIX}{encoded}"))
}

/// Decrypt a value previously produced by [`encrypt`].
///
/// # Migration support
/// If `encrypted` does not have the `$aes-gcm$` prefix, it is returned as-is.
/// This allows existing plaintext passwords to work without immediate migration.
/// Call [`reencrypt_if_plaintext`] to upgrade them to encrypted form.
pub fn decrypt(encrypted: &str) -> Result<String, String> {
    let key = get_key()?;

    // Plaintext passthrough for migration
    let encoded = match encrypted.strip_prefix(ENCRYPTED_PREFIX) {
        Some(s) => s,
        None => return Ok(encrypted.to_string()),
    };

    let combined = BASE64
        .decode(encoded)
        .map_err(|e| format!("Failed to decode base64: {e}"))?;

    if combined.len() < 12 {
        return Err("Encrypted data too short".into());
    }

    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let nonce = Nonce::from_slice(nonce_bytes);

    let cipher =
        Aes256Gcm::new_from_slice(key).map_err(|e| format!("Failed to create cipher: {e}"))?;

    let plaintext = cipher
        .decrypt(nonce, ciphertext)
        .map_err(|e| format!("Decryption failed: {e}"))?;

    String::from_utf8(plaintext).map_err(|e| format!("Decrypted data is not valid UTF-8: {e}"))
}

/// Check whether a stored value is encrypted (has the `$aes-gcm$` prefix).
pub fn is_encrypted(value: &str) -> bool {
    value.starts_with(ENCRYPTED_PREFIX)
}

/// If `stored` is a plaintext password, encrypt it and return the encrypted form.
///
/// This is the migration helper: call it whenever a password is read from the DB.
/// If the value was already encrypted, it is returned unchanged.
#[allow(dead_code)]
pub fn reencrypt_if_plaintext(stored: &str) -> Result<String, String> {
    if is_encrypted(stored) {
        Ok(stored.to_string())
    } else {
        encrypt(stored)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::Once;

    /// Global test initialization — runs exactly once across all crypto tests.
    static INIT: Once = Once::new();
    static TEST_DIR: once_cell::sync::Lazy<tempfile::TempDir> =
        once_cell::sync::Lazy::new(|| {
            let dir = tempfile::tempdir().expect("failed to create temp dir");
            init_key(dir.path()).expect("failed to init key");
            dir
        });

    fn ensure_init() -> &'static tempfile::TempDir {
        let _ = INIT.call_once(|| {
            let _ = &*TEST_DIR;
        });
        &TEST_DIR
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let _dir = ensure_init();
        let original = "my_secret_password_123!";
        let encrypted = encrypt(original).unwrap();
        assert!(encrypted.starts_with(ENCRYPTED_PREFIX));
        let decrypted = decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, original);
    }

    #[test]
    fn test_encrypt_produces_different_output_each_time() {
        let _dir = ensure_init();
        let original = "same_password";
        let a = encrypt(original).unwrap();
        let b = encrypt(original).unwrap();
        assert_ne!(a, b, "each encryption must use a fresh nonce");
    }

    #[test]
    fn test_decrypt_plaintext_passthrough() {
        let _dir = ensure_init();
        let plain = "plaintext_password";
        let result = decrypt(plain).unwrap();
        assert_eq!(result, plain);
    }

    #[test]
    fn test_is_encrypted() {
        let _dir = ensure_init();
        assert!(is_encrypted("$aes-gcm$abc123"));
        assert!(!is_encrypted("plaintext"));
        assert!(!is_encrypted(""));
    }

    #[test]
    fn test_reencrypt_if_plaintext() {
        let _dir = ensure_init();
        // Plaintext → encrypted
        let result = reencrypt_if_plaintext("my_plain_password").unwrap();
        assert!(is_encrypted(&result));
        assert_eq!(decrypt(&result).unwrap(), "my_plain_password");

        // Already encrypted → unchanged
        let already = encrypt("already_encrypted").unwrap();
        let result2 = reencrypt_if_plaintext(&already).unwrap();
        assert_eq!(result2, already);
    }

    #[test]
    fn test_decrypt_tampered_data_fails() {
        let _dir = ensure_init();
        let encrypted = encrypt("test_value").unwrap();
        // Corrupt the base64 data
        let tampered = encrypted.replace('a', "b");
        let result = decrypt(&tampered);
        assert!(result.is_err() || result.unwrap() != "test_value");
    }

    #[test]
    fn test_encrypt_empty_string() {
        let _dir = ensure_init();
        let encrypted = encrypt("").unwrap();
        let decrypted = decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, "");
    }

    #[test]
    fn test_encrypt_unicode() {
        let _dir = ensure_init();
        let original = "pässwörd_üñîçødé_日本語";
        let encrypted = encrypt(original).unwrap();
        let decrypted = decrypt(&encrypted).unwrap();
        assert_eq!(decrypted, original);
    }
}
