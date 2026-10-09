//! Keys for the data providers that need an account. They come from the OS
//! keychain (the app reads them and hands them to the hub); `Debug` never
//! prints them.

#[derive(Clone, Default, PartialEq)]
pub struct Credentials {
    /// Alpaca market data: key id, secret, and whether the account has the
    /// paid SIP feed (else the free IEX feed).
    pub alpaca: Option<(String, String, bool)>,
    /// OANDA: API token, account id, and whether it's a live account (else
    /// practice).
    pub oanda: Option<(String, String, bool)>,
    /// Upstox access token (valid for a day).
    pub upstox: Option<String>,
    /// Interactive Brokers Client Portal gateway URL.
    pub ibkr: Option<String>,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let set = |b: bool| if b { "set" } else { "not set" };
        f.debug_struct("Credentials")
            .field("alpaca", &set(self.alpaca.is_some()))
            .field("oanda", &set(self.oanda.is_some()))
            .field("upstox", &set(self.upstox.is_some()))
            .field("ibkr", &self.ibkr)
            .finish()
    }
}

/// Keychain entry names.
pub mod keys {
    pub const ALPACA_KEY: &str = "alpaca-key-id";
    pub const ALPACA_SECRET: &str = "alpaca-secret";
    pub const OANDA_TOKEN: &str = "oanda-token";
    pub const UPSTOX_TOKEN: &str = "upstox-token";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_secrets() {
        let c = Credentials {
            alpaca: Some(("KEYID".into(), "SECRETXYZ".into(), false)),
            upstox: Some("TOKEN123".into()),
            ..Default::default()
        };
        let s = format!("{c:?}");
        assert!(!s.contains("SECRETXYZ") && !s.contains("TOKEN123") && !s.contains("KEYID"));
        assert!(s.contains("alpaca: \"set\""));
    }
}
