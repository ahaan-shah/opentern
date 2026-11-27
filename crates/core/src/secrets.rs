//! Secrets (API keys and tokens) live in the operating system's keychain,
//! never in the workspace database or in logs.

/// A stored secret, if there is one (and the keychain is reachable).
pub fn get(key: &str) -> Option<String> {
    keyring::Entry::new(service(), key)
        .and_then(|e| e.get_password())
        .ok()
        .filter(|s| !s.is_empty())
}

/// Stores (or, with an empty value, removes) a secret.
pub fn set(key: &str, value: &str) -> Result<(), String> {
    let e = keyring::Entry::new(service(), key).map_err(|e| e.to_string())?;
    if value.is_empty() {
        match e.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err.to_string()),
        }
    } else {
        e.set_password(value).map_err(|e| e.to_string())
    }
}

/// The keychain's name for Opentern's keys: a development copy keeps its
/// own ("opentern-dev"), so it never reads or changes the real app's.
fn service() -> &'static str {
    crate::app_name()
}
