use anyhow::{anyhow, Context, Result};
use linux_keyutils::{Key, KeyError, KeyRing, KeyRingIdentifier};

/// Key timeout in seconds. Configurable via `APA_TIMEOUT_SECS`.
const DEFAULT_TIMEOUT_SECS: usize = 3600;

/// Key description prefix used in the kernel keyring.
const KEY_PREFIX: &str = "ansible-password-agent:";

fn timeout_secs() -> usize {
    std::env::var("APA_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_TIMEOUT_SECS)
}

/// Errors that mean "no usable cached secret here" rather than a real
/// failure. `KeyRevoked`/`KeyExpired` cover stale keys left behind by a
/// previous session; a fresh `add_key` then replaces them. `AccessDenied`
/// covers keys we can see but do not possess.
fn is_miss(e: &KeyError) -> bool {
    matches!(
        e,
        KeyError::KeyDoesNotExist
            | KeyError::KeyExpired
            | KeyError::KeyRevoked
            | KeyError::AccessDenied
    )
}

/// Linux backend using the Kernel Key Retention Service via the
/// process session keyring (`@s`).
///
/// The keyring lives in unswappable kernel memory and is shared by every
/// descendant of the shell that owns it — fish, coding agents, and
/// ansible password-file invocations alike — and vanishes when the shell
/// exits. It is created on demand and installed for the process, so it
/// always sits inside our possession chain: every key we add is fully
/// manageable (read, refresh, revoke). Keys expire after
/// [`timeout_secs()`]; the timeout is refreshed on every successful read.
pub struct LinuxBackend;

impl LinuxBackend {
    /// The process session keyring, created on demand.
    fn session_keyring() -> Result<KeyRing> {
        KeyRing::from_special_id(KeyRingIdentifier::Session, true)
            .map_err(|e| anyhow!("failed to open the session keyring (@s): {e}"))
    }

    /// Find a cached key and refresh its timeout: an actively used cache
    /// stays warm.
    fn find(key: &str) -> Result<Option<Key>> {
        let ring = Self::session_keyring()?;
        match ring.search(&key_description(key)) {
            Ok(k) => {
                k.set_timeout(timeout_secs())
                    .map_err(|e| anyhow!("failed to refresh key timeout: {e}"))?;
                Ok(Some(k))
            }
            Err(e) if is_miss(&e) => Ok(None),
            Err(e) => Err(anyhow!("failed to search keyring: {e}")),
        }
    }

    /// Revoke the cached secret for a key, if present.
    /// Returns whether a key was found.
    pub fn remove(key: &str) -> Result<bool> {
        match Self::find(key)? {
            Some(k) => {
                k.revoke()
                    .map_err(|e| anyhow!("failed to revoke key: {e}"))?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    pub fn get(key: &str) -> Result<Option<String>> {
        let Some(k) = Self::find(key)? else {
            return Ok(None);
        };

        let payload = k
            .read_to_vec()
            .map_err(|e| anyhow!("failed to read key payload: {e}"))?;
        let secret = String::from_utf8(payload).context("key payload is not valid UTF-8")?;
        Ok(Some(secret))
    }

    pub fn set(key: &str, secret: &str) -> Result<()> {
        let ring = Self::session_keyring()?;
        let description = key_description(key);

        // Revoke any existing key first: reprompting after a wrong password
        // must overwrite the cached secret, and updating a revoked/stale key
        // fails.
        if let Ok(Some(existing)) = Self::find(key) {
            let _ = existing.revoke();
        }

        let k = ring
            .add_key(&description, secret.as_bytes())
            .map_err(|e| anyhow!("failed to add key to the session keyring: {e}"))?;
        k.set_timeout(timeout_secs()).map_err(|e| {
            // Never leave a key behind that we could not give an expiry.
            let _ = ring.unlink_key(k);
            anyhow!("failed to set key timeout: {e}")
        })?;
        Ok(())
    }
}

/// Build the kernel keyring description from the logical key name.
fn key_description(key: &str) -> String {
    format!("{KEY_PREFIX}{key}")
}
