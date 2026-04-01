use anyhow::{Context, Result};
use linux_keyutils::{KeyError, KeyRing, KeyRingIdentifier};

use super::PasswordBackend;

/// Key timeout in seconds (10 minutes).
const KEY_TIMEOUT_SECS: usize = 600;

/// Key description prefix used in the kernel keyring.
const KEY_PREFIX: &str = "ansible-password-agent:";

/// Linux backend using the Kernel Key Retention Service via the user session keyring.
///
/// Passwords are stored in unswappable kernel memory and automatically expire
/// after 10 minutes of inactivity. The timeout is refreshed on every successful read.
pub struct LinuxBackend;

impl PasswordBackend for LinuxBackend {
    fn get(key: &str) -> Result<Option<String>> {
        let description = key_description(key);
        let ring = user_keyring()?;

        match ring.search(&description) {
            Ok(k) => {
                // Refresh the timeout before reading the payload.
                k.set_timeout(KEY_TIMEOUT_SECS)
                    .context("failed to refresh key timeout")?;

                let payload = k
                    .read_to_vec()
                    .context("failed to read key payload")?;
                let secret = String::from_utf8(payload)
                    .context("key payload is not valid UTF-8")?;
                Ok(Some(secret))
            }
            Err(KeyError::KeyDoesNotExist | KeyError::KeyExpired) => Ok(None),
            Err(e) => Err(e).context("failed to search for key in keyring"),
        }
    }

    fn set(key: &str, secret: &str) -> Result<()> {
        let description = key_description(key);
        let ring = user_keyring()?;

        let k = ring
            .add_key(&description, secret.as_bytes())
            .context("failed to add key to keyring")?;

        k.set_timeout(KEY_TIMEOUT_SECS)
            .context("failed to set key timeout")?;

        Ok(())
    }
}

/// Get the user session keyring, creating it if necessary.
fn user_keyring() -> Result<KeyRing> {
    KeyRing::from_special_id(KeyRingIdentifier::UserSession, true)
        .context("failed to open user session keyring")
}

/// Build the kernel keyring description from the logical key name.
fn key_description(key: &str) -> String {
    format!("{KEY_PREFIX}{key}")
}
