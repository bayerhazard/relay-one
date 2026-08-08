//! Web Push (RFC 8030 + RFC 8291) support.
//!
//! The server holds a VAPID key pair (generated once, stored in settings),
//! stores push subscriptions per account, and sends notifications when new
//! mail arrives. Encryption uses aes128gcm (RFC 8291) with ECDH (P-256).

use std::sync::Arc;

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes128Gcm, Nonce};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use p256::ecdsa::signature::Signer;
use p256::ecdsa::{SigningKey, VerifyingKey};
use p256::elliptic_curve::sec1::ToEncodedPoint;
use p256::PublicKey as P256PublicKey;
use p256::SecretKey as P256SecretKey;
use rand::rngs::OsRng;

use crate::AppState;

const VAPID_PUBLIC_KEY: &str = "vapid_public_key";
const VAPID_PRIVATE_KEY: &str = "vapid_private_key";
const VAPID_SUBJECT: &str = "vapid_subject";

#[derive(Debug, Clone)]
pub struct PushSubscription {
    pub id: i64,
    pub account_id: i64,
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}

/// Ensure the VAPID key pair exists in settings. Returns the public key
/// (base64url, uncompressed 65-byte P-256 point).
pub fn ensure_vapid_keys(
    conn: &rusqlite::Connection,
    subject: &str,
) -> Result<String, String> {
    if let Ok(Some(pk)) = crate::cache::settings::get_setting(conn, VAPID_PUBLIC_KEY) {
        return Ok(pk);
    }

    let signing_key = SigningKey::random(&mut OsRng);
    let verifying_key = VerifyingKey::from(&signing_key);
    let public = verifying_key.to_encoded_point(false);
    let public_b64 = URL_SAFE_NO_PAD.encode(public.as_bytes());
    let private_b64 = URL_SAFE_NO_PAD.encode(signing_key.to_bytes().as_slice());

    crate::cache::settings::set_setting(conn, VAPID_PUBLIC_KEY, &public_b64)
        .map_err(|e| format!("VAPID public: {e}"))?;
    crate::cache::settings::set_setting(conn, VAPID_PRIVATE_KEY, &private_b64)
        .map_err(|e| format!("VAPID private: {e}"))?;
    crate::cache::settings::set_setting(conn, VAPID_SUBJECT, subject)
        .map_err(|e| format!("VAPID subject: {e}"))?;
    Ok(public_b64)
}

fn load_vapid(
    conn: &rusqlite::Connection,
) -> Result<(VerifyingKey, SigningKey, String), String> {
    let get = |key: &str| -> Result<String, String> {
        crate::cache::settings::get_setting(conn, key)
            .map_err(|e| format!("VAPID settings: {e}"))?
            .ok_or_else(|| format!("VAPID-Key '{}' nicht initialisiert", key))
    };
    let pub_b64 = get(VAPID_PUBLIC_KEY)?;
    let priv_b64 = get(VAPID_PRIVATE_KEY)?;
    let subject = get(VAPID_SUBJECT)?;

    let pub_bytes = URL_SAFE_NO_PAD
        .decode(&pub_b64)
        .map_err(|e| format!("VAPID public decode: {e}"))?;
    let priv_bytes = URL_SAFE_NO_PAD
        .decode(&priv_b64)
        .map_err(|e| format!("VAPID private decode: {e}"))?;

    let signing = SigningKey::from_slice(&priv_bytes)
        .map_err(|e| format!("VAPID private key: {e}"))?;
    let verifying = VerifyingKey::from_sec1_bytes(&pub_bytes)
        .map_err(|e| format!("VAPID public key: {e}"))?;
    Ok((verifying, signing, subject))
}

/// Build the VAPID Authorization header (ES256 JWT) for the push service.
/// Format: `vapid t=<jwt>,k=<public-key>` where the JWT's ECDSA signature is
/// DER-encoded (RFC 7515) and the key is the uncompressed P-256 point.
fn build_vapid_auth(
    signing: &SigningKey,
    subject: &str,
    audience: &str,
) -> Result<String, String> {
    let now = chrono::Utc::now().timestamp();
    let header = serde_json::json!({ "typ": "JWT", "alg": "ES256" });
    let claims = serde_json::json!({
        "aud": audience,
        "exp": now + 12 * 60 * 60,
        "sub": subject,
    });
    let enc = |v: &serde_json::Value| -> String { URL_SAFE_NO_PAD.encode(v.to_string().as_bytes()) };
    let signing_input = format!("{}.{}", enc(&header), enc(&claims));

    // ECDSA over P-256: r and s are each 32 bytes, raw concatenation is NOT
    // valid DER — encode as ASN.1 SEQUENCE of two INTEGERs.
    let signature: p256::ecdsa::Signature = signing.sign(signing_input.as_bytes());
    let (r, s) = signature.split_bytes();
    let der = der_encode_ecdsa(r.as_slice(), s.as_slice());
    let jwt = format!("{}.{}", signing_input, URL_SAFE_NO_PAD.encode(&der));

    let verifying = VerifyingKey::from(signing);
    let public_b64 = URL_SAFE_NO_PAD.encode(verifying.to_encoded_point(false).as_bytes());

    Ok(format!("vapid t={},k={}", jwt, public_b64))
}

/// Minimal ASN.1 DER encoding for an ECDSA signature (SEQUENCE of two INTEGERs).
fn der_encode_ecdsa(r: &[u8], s: &[u8]) -> Vec<u8> {
    let mut r_int = r.to_vec();
    // strip leading zeros, keep sign byte if high bit set
    while r_int.len() > 1 && r_int[0] == 0 {
        r_int.remove(0);
    }
    if r_int[0] & 0x80 != 0 {
        r_int.insert(0, 0);
    }
    let mut s_int = s.to_vec();
    while s_int.len() > 1 && s_int[0] == 0 {
        s_int.remove(0);
    }
    if s_int[0] & 0x80 != 0 {
        s_int.insert(0, 0);
    }
    let mut out = vec![0x30];
    let total = 2 + r_int.len() + 2 + s_int.len();
    out.push(total as u8);
    out.push(0x02);
    out.push(r_int.len() as u8);
    out.extend_from_slice(&r_int);
    out.push(0x02);
    out.push(s_int.len() as u8);
    out.extend_from_slice(&s_int);
    out
}

/// RFC 8291 aes128gcm encryption of `payload` for a subscription.
fn encrypt_payload(
    p256dh_b64: &str,
    auth_b64: &str,
    payload: &[u8],
) -> Result<Vec<u8>, String> {
    use hkdf::Hkdf;
    use p256::ecdh::diffie_hellman;
    use sha2::Sha256;

    let p256dh = URL_SAFE_NO_PAD
        .decode(p256dh_b64)
        .map_err(|e| format!("p256dh decode: {e}"))?;
    let auth = URL_SAFE_NO_PAD
        .decode(auth_b64)
        .map_err(|e| format!("auth decode: {e}"))?;

    let ua_public = P256PublicKey::from_sec1_bytes(&p256dh)
        .map_err(|e| format!("UA public key: {e}"))?;
    let as_secret = P256SecretKey::random(&mut OsRng);
    let as_public = as_secret.public_key();
    let as_public_bytes = as_public.to_encoded_point(false);

    let shared = diffie_hellman(as_secret.to_nonzero_scalar(), ua_public.as_affine());

    let auth_info = b"Content-Encoding: auth\0";
    let mut prk = [0u8; 32];
    Hkdf::<Sha256>::new(Some(auth.as_slice()), &shared.raw_secret_bytes())
        .expand(auth_info, &mut prk)
        .map_err(|e| format!("HKDF auth: {e}"))?;

    let salt = {
        let mut s = [0u8; 16];
        use rand::RngCore;
        OsRng.fill_bytes(&mut s);
        s
    };

    let mut key_info = b"WebPush: info\0".to_vec();
    key_info.extend_from_slice(ua_public.as_affine().to_encoded_point(false).as_bytes());
    key_info.extend_from_slice(as_public_bytes.as_bytes());
    let ikm = prk;
    let mut ik = [0u8; 16];
    Hkdf::<Sha256>::new(Some(&salt), &ikm)
        .expand(&key_info, &mut ik)
        .map_err(|e| format!("HKDF key: {e}"))?;

    let mut nonce = [0u8; 12];
    Hkdf::<Sha256>::new(Some(&salt), &ikm)
        .expand(b"Content-Encoding: nonce", &mut nonce)
        .map_err(|e| format!("HKDF nonce: {e}"))?;

    let cipher = Aes128Gcm::new_from_slice(&ik).map_err(|e| format!("AES key: {e}"))?;

    // aes128gcm record: salt(16) || rs(4 BE) || idlen(1) || keyid(0)
    let rs = 4096u32;
    let mut header = Vec::with_capacity(21);
    header.extend_from_slice(&salt);
    header.extend_from_slice(&rs.to_be_bytes());
    header.push(0);
    header.push(0);

    let mut ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce), payload)
        .map_err(|e| format!("AES-GCM: {e}"))?;
    ciphertext.push(0x02); // padding delimiter

    let mut out = header;
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

/// Store a subscription (upsert by endpoint).
pub fn upsert_subscription(
    conn: &rusqlite::Connection,
    account_id: i64,
    endpoint: &str,
    p256dh: &str,
    auth: &str,
) -> Result<i64, String> {
    conn.execute(
        "INSERT INTO push_subscriptions (account_id, endpoint, p256dh, auth)
         VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(endpoint) DO UPDATE SET
           account_id = excluded.account_id,
           p256dh = excluded.p256dh,
           auth = excluded.auth",
        rusqlite::params![account_id, endpoint, p256dh, auth],
    )
    .map_err(|e| format!("Subscription upsert: {e}"))?;
    let id = conn.last_insert_rowid();
    Ok(id)
}

/// Remove a subscription.
pub fn remove_subscription(
    conn: &rusqlite::Connection,
    endpoint: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM push_subscriptions WHERE endpoint = ?1",
        rusqlite::params![endpoint],
    )
    .map_err(|e| format!("Subscription delete: {e}"))?;
    Ok(())
}

/// List all subscriptions for an account.
pub fn list_subscriptions(
    conn: &rusqlite::Connection,
    account_id: i64,
) -> Result<Vec<PushSubscription>, String> {
    let mut stmt = conn
        .prepare("SELECT id, account_id, endpoint, p256dh, auth FROM push_subscriptions WHERE account_id = ?1")
        .map_err(|e| format!("Subscriptions query: {e}"))?;
    let rows = stmt
        .query_map(rusqlite::params![account_id], |row| {
            Ok(PushSubscription {
                id: row.get(0)?,
                account_id: row.get(1)?,
                endpoint: row.get(2)?,
                p256dh: row.get(3)?,
                auth: row.get(4)?,
            })
        })
        .map_err(|e| format!("Subscriptions rows: {e}"))?;
    let mut subs = Vec::new();
    for r in rows {
        subs.push(r.map_err(|e| format!("Subscription row: {e}"))?);
    }
    Ok(subs)
}

/// Send a push to every subscription of an account. Returns count sent.
pub async fn notify_account(
    state: &AppState,
    account_id: i64,
    title: &str,
    body: &str,
) -> Result<usize, String> {
    let subs = {
        let guard = state.cache_db.lock();
        let Some(conn) = guard.as_ref() else { return Err("DB nicht initialisiert".into()) };
        list_subscriptions(conn, account_id)?
    };
    if subs.is_empty() {
        return Ok(0);
    }

    let (_, signing, subject) = {
        let guard = state.cache_db.lock();
        let Some(conn) = guard.as_ref() else { return Err("DB nicht initialisiert".into()) };
        load_vapid(conn)?
    };

    let payload = serde_json::json!({ "title": title, "body": body });
    let payload_bytes = payload.to_string().into_bytes();

    let client = reqwest::Client::builder()
        .http2_adaptive_window(true)
        .build()
        .map_err(|e| format!("HTTP client: {e}"))?;

    let mut sent = 0usize;
    let mut failures = Vec::new();
    for sub in &subs {
        // derive audience from endpoint origin
        let audience = match sub.endpoint.split('/').nth(2) {
            Some(h) => format!("https://{}", h),
            None => {
                failures.push(format!("endpoint ohne Origin: {}", sub.endpoint));
                continue;
            }
        };
        let auth_header = match build_vapid_auth(&signing, &subject, &audience) {
            Ok(h) => h,
            Err(e) => {
                failures.push(e);
                continue;
            }
        };
        let encrypted = match encrypt_payload(&sub.p256dh, &sub.auth, &payload_bytes) {
            Ok(b) => b,
            Err(e) => {
                failures.push(format!("encrypt: {e}"));
                continue;
            }
        };

        let res = match client
            .post(&sub.endpoint)
            .header("TTL", "86400")
            .header("Authorization", &auth_header)
            .header("Content-Encoding", "aes128gcm")
            .header("Content-Type", "application/octet-stream")
            .body(encrypted)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("send: {e}"));
                continue;
            }
        };

        let status = res.status();
        if status.is_success() || status == reqwest::StatusCode::NOT_FOUND {
            if status == reqwest::StatusCode::NOT_FOUND {
                // subscription expired on the push service — drop it
                {
                    let guard = state.cache_db.lock();
                    if let Some(conn) = guard.as_ref() {
                        let _ = remove_subscription(conn, &sub.endpoint);
                    }
                }
            }
            sent += 1;
        } else {
            failures.push(format!("{} {}", status, sub.endpoint));
        }
    }

    if !failures.is_empty() {
        tracing::warn!(
            "WebPush: {} gesendet, {} fehlgeschlagen: {}",
            sent,
            failures.len(),
            failures.join("; ")
        );
    }
    Ok(sent)
}

/// Public VAPID key getter (kept for API symmetry).
pub fn get_vapid_public_key(state: &Arc<AppState>) -> Result<String, String> {
    let guard = state.cache_db.lock();
    let conn = guard.as_ref().ok_or("DB nicht initialisiert")?;
    ensure_vapid_keys(conn, "mailto:relay@aimighty.olares.de")
}
