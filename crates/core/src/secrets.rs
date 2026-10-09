//! Secrets (API keys and tokens) live in the operating system's keychain,
//! never in the workspace database or in logs.

const SERVICE: &str = "opentern";

/// A stored secret, if there is one (and the keychain is reachable).
pub fn get(key: &str) -> Option<String> {
    keyring::Entry::new(SERVICE, key)
        .and_then(|e| e.get_password())
        .ok()
        .filter(|s| !s.is_empty())
}

/// Stores (or, with an empty value, removes) a secret.
pub fn set(key: &str, value: &str) -> Result<(), String> {
    let e = keyring::Entry::new(SERVICE, key).map_err(|e| e.to_string())?;
    if value.is_empty() {
        match e.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err.to_string()),
        }
    } else {
        e.set_password(value).map_err(|e| e.to_string())
    }
}
