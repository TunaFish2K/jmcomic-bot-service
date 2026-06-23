use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

pub fn sign_artifact(secret: &str, artifact_id: &str, exp: i64) -> String {
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC accepts any key");
    mac.update(format!("{artifact_id}.{exp}").as_bytes());
    URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes())
}

pub fn verify_artifact_signature(secret: &str, artifact_id: &str, exp: i64, sig: &str) -> bool {
    if exp < now_unix() {
        return false;
    }

    let expected = sign_artifact(secret, artifact_id, exp);
    constant_time_eq(expected.as_bytes(), sig.as_bytes())
}

pub fn signed_file_url(secret: &str, artifact_id: &str, ttl_seconds: i64) -> String {
    let exp = now_unix() + ttl_seconds;
    let sig = sign_artifact(secret, artifact_id, exp);
    format!("/api/v1/files/{artifact_id}?exp={exp}&sig={sig}")
}

pub fn now_unix() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }

    let mut diff = 0_u8;
    for (left, right) in a.iter().zip(b.iter()) {
        diff |= left ^ right;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_url_validation_accepts_current_signature() {
        let exp = now_unix() + 60;
        let sig = sign_artifact("secret", "artifact", exp);
        assert!(verify_artifact_signature("secret", "artifact", exp, &sig));
    }

    #[test]
    fn signed_url_validation_rejects_expired_signature() {
        let exp = now_unix() - 1;
        let sig = sign_artifact("secret", "artifact", exp);
        assert!(!verify_artifact_signature("secret", "artifact", exp, &sig));
    }
}
