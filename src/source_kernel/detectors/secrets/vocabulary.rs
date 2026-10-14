//! The words that make an environment variable a secret, and a fallback a
//! real one. A name is a secret when it carries one of `SECRET_WORDS` and does
//! not end in a suffix that names where a secret goes rather than the secret.
//! A fallback is a real secret when it is long enough to be one and is not a
//! placeholder nobody could mistake for a key.

/// The words a secret's name carries.
const SECRET_WORDS: [&str; 6] = ["API_KEY", "PASSWORD", "PRIVATE_KEY", "SECRET", "SIGNING_KEY", "TOKEN"];

/// The suffixes that name a location, a label or an identifier, not a secret.
const NOT_SECRET_SUFFIXES: [&str; 5] = ["_ENDPOINT", "_HEADER", "_ID", "_NAME", "_URL"];

/// The shortest fallback that can be a key.
const SHORTEST_SECRET: usize = 8;

/// Text that marks a fallback as a placeholder.
const PLACEHOLDERS: [&str; 5] = ["changeme", "example", "your_", "xxx", "0000"];

/// Is `name` the name of a secret?
pub(super) fn secret_name(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    SECRET_WORDS.iter().any(|word| name.contains(word))
        && !NOT_SECRET_SUFFIXES.iter().any(|suffix| name.ends_with(suffix))
}

/// Is `value` a fallback that can be a real secret?
pub(super) fn real_secret(value: &str) -> bool {
    let lowered = value.to_ascii_lowercase();
    value.chars().count() >= SHORTEST_SECRET && !PLACEHOLDERS.iter().any(|placeholder| lowered.contains(placeholder))
}

#[cfg(test)]
mod tests {
    use super::{real_secret, secret_name};

    #[test]
    fn a_name_is_a_secret_by_its_word_unless_its_suffix_names_a_location() {
        for name in ["JWT_SECRET", "github_token", "DB_PASSWORD", "SSH_PRIVATE_KEY", "STRIPE_API_KEY", "SIGNING_KEY"] {
            assert!(secret_name(name), "{name}");
        }
        for name in ["DATABASE_URL", "API_KEY_HEADER", "TOKEN_ENDPOINT", "SECRET_NAME", "CLIENT_SECRET_ID", "PORT"] {
            assert!(!secret_name(name), "{name}");
        }
    }

    #[test]
    fn a_fallback_is_a_secret_when_long_enough_and_no_placeholder() {
        for value in ["dev-signing-key", "sk_live_fallback", "hunter2hunter2"] {
            assert!(real_secret(value), "{value}");
        }
        for value in ["short", "changeme-now", "your_secret_here", "example-key-1", "xxxxxxxxxx", "00000000"] {
            assert!(!real_secret(value), "{value}");
        }
    }
}
