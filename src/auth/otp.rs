use rand::{Rng, rngs::OsRng};
use sha2::{Digest, Sha256};

pub fn generate_code() -> String {
    format!("{:06}", OsRng.gen_range(0..1_000_000))
}

pub fn hash_code(email: &str, code: &str, pepper: &str) -> String {
    let mut h = Sha256::new();

    h.update(email.trim().to_lowercase());
    h.update(":");
    h.update(code);
    h.update(":");
    h.update(pepper);

    hex::encode(h.finalize())
}
