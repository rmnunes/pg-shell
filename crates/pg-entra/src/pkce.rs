//! RFC 7636 Proof Key for Code Exchange helpers.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};

pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

/// Fresh verifier (43 URL-safe chars from 32 random bytes) and its S256
/// challenge.
pub fn generate() -> Pkce {
    let verifier = random_urlsafe(32);
    let challenge = challenge_for(&verifier);
    Pkce {
        verifier,
        challenge,
    }
}

pub fn challenge_for(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// `bytes` random bytes from the OS-seeded CSPRNG, base64url without padding.
pub fn random_urlsafe(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc7636_appendix_b_vector() {
        assert_eq!(
            challenge_for("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn generated_verifier_is_within_spec_length_and_unique() {
        let a = generate();
        let b = generate();
        assert!((43..=128).contains(&a.verifier.len()));
        assert_ne!(a.verifier, b.verifier);
        assert_eq!(a.challenge, challenge_for(&a.verifier));
    }
}
