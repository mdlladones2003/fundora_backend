use rand::{Rng, rngs::OsRng};
use sha2::{Digest, Sha256};

pub fn is_valid_email(email: &str) -> bool {
    let email = email.trim();
    if email.is_empty() || email.len() > 254 {
        return false;
    }
    match email.split_once('@') {
        Some((local, domain)) => {
            !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
        }
        None => false,
    }
}

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

#[cfg(test)]
mod tests {
    use super::is_valid_email;

    #[test]
    fn validates_email_addresses_used_for_auth() {
        assert!(is_valid_email(" donor@example.org "));
        assert!(!is_valid_email(""));
        assert!(!is_valid_email("not-an-email"));
        assert!(!is_valid_email("donor@.example.org"));
    }
}
