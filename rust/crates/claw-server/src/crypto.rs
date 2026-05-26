//! Encrypt API keys at rest using `CLAW_MASTER_KEY` (UTF-8 string; hashed to 32 bytes).

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use rand::Rng;
use sha2::{Digest, Sha256};

use crate::error::ServerError;

const NONCE_LEN: usize = 12;

fn cipher_from_master(master: &str) -> Result<Aes256Gcm, ServerError> {
    let key = Sha256::digest(master.as_bytes());
    Aes256Gcm::new_from_slice(&key).map_err(|e| ServerError::Internal(e.to_string()))
}

/// Hex-encoded `nonce || ciphertext`.
pub fn encrypt_secret(master: &str, plaintext: &str) -> Result<String, ServerError> {
    let cipher = cipher_from_master(master)?;
    let nonce_bytes: [u8; NONCE_LEN] = rand::thread_rng().gen();
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| ServerError::Internal(e.to_string()))?;
    let mut buf = nonce_bytes.to_vec();
    buf.extend_from_slice(&ciphertext);
    Ok(hex::encode(buf))
}

pub fn decrypt_secret(master: &str, blob_hex: &str) -> Result<String, ServerError> {
    let raw = hex::decode(blob_hex).map_err(|e| ServerError::Internal(e.to_string()))?;
    if raw.len() <= NONCE_LEN {
        return Err(ServerError::Internal(String::from(
            "invalid ciphertext length",
        )));
    }
    let (nonce_bytes, ct) = raw.split_at(NONCE_LEN);
    let nonce = Nonce::from_slice(nonce_bytes);
    let cipher = cipher_from_master(master)?;
    let plain = cipher
        .decrypt(nonce, ct)
        .map_err(|_| ServerError::Internal(String::from("decryption failed")))?;
    String::from_utf8(plain).map_err(|e| ServerError::Internal(e.to_string()))
}
