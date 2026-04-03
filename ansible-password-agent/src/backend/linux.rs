use anyhow::{Context, Result};
use linux_keyutils::{KeyError, KeyRing, KeyRingIdentifier};

use super::PasswordBackend;

/// Key timeout in seconds (10 minutes).
const KEY_TIMEOUT_SECS: usize = 600;

/// Key description prefix used in the kernel keyring.
const KEY_PREFIX: &str = "ansible-password-agent:";

/// Expected session keyring description when running inside
/// `keyctl session ansible_password_agent`.
const SESSION_KEYRING_NAME: &str = "ansible_password_agent";

/// Linux backend using the Kernel Key Retention Service via the process session keyring.
///
/// Passwords are stored in unswappable kernel memory and automatically expire
/// after 10 minutes of inactivity. The timeout is refreshed on every successful read.
///
/// Uses the process session keyring (`@s`) rather than the user session keyring
/// (`@us`) because on modern Fedora kernels (6.x+) the user session keyring has
/// default restrictions that prevent reading key payloads.
pub struct LinuxBackend;

impl PasswordBackend for LinuxBackend {
    fn get(key: &str) -> Result<Option<String>> {
        let description = key_description(key);
        let ring = session_keyring()?;

        match ring.search(&description) {
            Ok(k) => {
                // Refresh the timeout before reading the payload.
                k.set_timeout(KEY_TIMEOUT_SECS)
                    .map_err(|e| anyhow::anyhow!("failed to refresh key timeout: {e}"))?;

                let payload = k
                    .read_to_vec()
                    .map_err(|e| anyhow::anyhow!("failed to read key payload: {e}"))?;
                let secret =
                    String::from_utf8(payload).context("key payload is not valid UTF-8")?;
                Ok(Some(secret))
            }
            Err(KeyError::KeyDoesNotExist | KeyError::KeyExpired) => Ok(None),
            Err(e) => Err(anyhow::anyhow!("failed to search for key in keyring: {e}")),
        }
    }

    fn set(key: &str, secret: &str) -> Result<()> {
        let description = key_description(key);
        let ring = session_keyring()?;

        let k = ring
            .add_key(&description, secret.as_bytes())
            .map_err(|e| anyhow::anyhow!("failed to add key to keyring: {e}"))?;

        k.set_timeout(KEY_TIMEOUT_SECS)
            .map_err(|e| anyhow::anyhow!("failed to set key timeout: {e}"))?;

        Ok(())
    }
}

/// Return the process session keyring.
///
/// Requires that a session keyring already exists for the process and that
/// it was created by `keyctl session ansible_vault`. Returns an error otherwise.
fn session_keyring() -> Result<KeyRing> {
    let ring = KeyRing::from_special_id(KeyRingIdentifier::Session, false)
        .map_err(|e| anyhow::anyhow!("failed to open session keyring: {e}"))?;

    ensure_vault_session(&ring)?;

    Ok(ring)
}

/// Verify that the current session keyring is the isolated vault session.
///
/// This prevents passwords from being stored in the user's default session
/// keyring, which would be accessible to unrelated processes.
fn ensure_vault_session(ring: &KeyRing) -> Result<()> {
    let metadata = ring
        .metadata()
        .map_err(|e| anyhow::anyhow!("failed to read session keyring metadata: {e}"))?;

    if metadata.get_description() != SESSION_KEYRING_NAME {
        anyhow::bail!(
            "not running inside an ansible_password_agent session keyring; \n\
             please start a new shell with: apa"
        );
    }

    Ok(())
}

/// Build the kernel keyring description from the logical key name.
fn key_description(key: &str) -> String {
    format!("{KEY_PREFIX}{key}")
}
