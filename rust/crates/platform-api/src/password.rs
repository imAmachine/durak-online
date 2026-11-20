use ring::pbkdf2;
use std::{num::NonZeroU32, sync::OnceLock};

const ITERATIONS: u32 = 600_000;
const SALT_LEN: usize = 16;
const KEY_LEN: usize = 32;
const PREFIX: &str = "pbkdf2_sha256";
const LEGACY_ALGORITHM: &str = "sha256";

static DUMMY_SALT: OnceLock<[u8; SALT_LEN]> = OnceLock::new();

fn b64(bytes: &[u8]) -> String {
    use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine};
    STANDARD_NO_PAD.encode(bytes)
}

fn unb64(text: &str) -> Option<Vec<u8>> {
    use base64::{engine::general_purpose::STANDARD_NO_PAD, Engine};
    STANDARD_NO_PAD.decode(text).ok()
}

fn derive(password: &str, salt: &[u8], iterations: u32) -> [u8; KEY_LEN] {
    let mut key = [0u8; KEY_LEN];
    let rounds = NonZeroU32::new(iterations.max(1)).expect("iterations are non-zero");
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        rounds,
        salt,
        password.as_bytes(),
        &mut key,
    );
    key
}

fn verify_key(password: &str, salt: &[u8], iterations: u32, expected: &[u8]) -> bool {
    let rounds = NonZeroU32::new(iterations.max(1)).expect("iterations are non-zero");
    pbkdf2::verify(
        pbkdf2::PBKDF2_HMAC_SHA256,
        rounds,
        salt,
        password.as_bytes(),
        expected,
    )
    .is_ok()
}

pub(crate) fn is_valid(password: &str) -> bool {
    (4..=128).contains(&password.chars().count())
}

pub(crate) fn hash(password: &str) -> String {
    let mut salt = [0u8; SALT_LEN];
    getrandom::getrandom(&mut salt).expect("OS random source unavailable");
    let key = derive(password, &salt, ITERATIONS);
    format!("{PREFIX}${ITERATIONS}${}${}", b64(&salt), b64(&key))
}

pub(crate) fn verify(password: &str, stored: &str) -> bool {
    let mut parts = stored.split('$');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(PREFIX), Some(iterations), Some(salt), Some(key)) => {
            let (Ok(iterations), Some(salt), Some(key)) =
                (iterations.parse::<u32>(), unb64(salt), unb64(key))
            else {
                return false;
            };
            if salt.len() < 8 || key.len() != KEY_LEN || iterations == 0 || iterations > 5_000_000 {
                return false;
            }
            verify_key(password, &salt, iterations, &key)
        }
        (Some(LEGACY_ALGORITHM), Some(hash), None, None) => {
            let expected = super::oauth::digest(password);
            constant_time_eq(hash.as_bytes(), expected.as_bytes())
        }
        _ => false,
    }
}

pub(crate) fn needs_upgrade(stored: &str) -> bool {
    match stored.split('$').next() {
        Some(PREFIX) => stored
            .split('$')
            .nth(1)
            .and_then(|value| value.parse::<u32>().ok())
            .is_none_or(|iterations| iterations < ITERATIONS),
        _ => true,
    }
}

pub(crate) fn burn(password: &str) {
    let salt = DUMMY_SALT.get_or_init(|| {
        let mut salt = [0u8; SALT_LEN];
        getrandom::getrandom(&mut salt).expect("OS random source unavailable");
        salt
    });
    let _ = derive(password, salt, ITERATIONS);
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub(crate) fn is_temporary(id: &str) -> bool {
    id.starts_with("temp:")
}

pub(crate) fn is_discord(id: &str) -> bool {
    id.starts_with("discord:")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_are_salted_and_verified() {
        let first = hash("секретный-пароль");
        let second = hash("секретный-пароль");
        assert_ne!(first, second);
        assert!(first.starts_with("pbkdf2_sha256$"));
        assert!(verify("секретный-пароль", &first));
        assert!(verify("секретный-пароль", &second));
        assert!(!verify("другой-пароль", &first));
    }

    #[test]
    fn legacy_sha256_hashes_still_authenticate_and_need_upgrade() {
        let stored = format!("sha256${}", super::super::oauth::digest("старый"));
        assert!(verify("старый", &stored));
        assert!(!verify("новый", &stored));
        assert!(needs_upgrade(&stored));
        assert!(!needs_upgrade(&hash("старый")));
        assert!(needs_upgrade(&format!(
            "{PREFIX}${}${}${}",
            ITERATIONS / 2,
            b64(&[1u8; SALT_LEN]),
            b64(&[2u8; KEY_LEN])
        )));
    }

    #[test]
    fn malformed_hashes_never_verify() {
        for stored in [
            "",
            "sha256",
            "sha256$",
            "pbkdf2_sha256$abc$aa$bb",
            "pbkdf2_sha256$1000$$",
            "pbkdf2_sha256$1000$!!$!!",
        ] {
            assert!(!verify("что-угодно", stored), "{stored}");
        }
    }
}
