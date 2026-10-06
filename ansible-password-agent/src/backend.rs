use anyhow::{anyhow, Context, Result};
use linux_keyutils::{KeyError, KeyRing, KeyRingIdentifier};

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
/// previous login session; a fresh `add_key` then replaces them.
fn is_miss(e: &KeyError) -> bool {
    matches!(
        e,
        KeyError::KeyDoesNotExist | KeyError::KeyExpired | KeyError::KeyRevoked
    )
}

/// Linux backend using the Kernel Key Retention Service via the
/// user session keyring (`@us`).
///
/// The keyring lives in unswappable kernel memory, is tied to the login
/// session (shared by all descendant processes — fish, coding agents,
/// ansible password-file invocations alike), and vanishes on logout.
/// Keys expire after [`timeout_secs()`]; the timeout is refreshed on
/// every successful read.
pub struct LinuxBackend;

impl LinuxBackend {
    /// Candidate keyrings, most to least preferred.
    ///
    /// Prefer the user session keyring: shared by every process of the
    /// login session. Fall back to the process session keyring, then the
    /// user keyring, for environments where the user session keyring is
    /// missing or not in the possession chain.
    fn keyrings() -> [KeyRing; 3] {
        [
            KeyRingIdentifier::UserSession,
            KeyRingIdentifier::Session,
            KeyRingIdentifier::User,
        ]
        .map(|id| KeyRing::from_special_id(id, true).ok())
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .try_into()
        .unwrap_or_else(|_| {
            // Unreachable in practice: every element above either Ok or None.
            [Self::empty_ring(), Self::empty_ring(), Self::empty_ring()]
        })
    }

    fn empty_ring() -> KeyRing {
        KeyRing::from_special_id(KeyRingIdentifier::Thread, true)
            .expect("thread keyring can always be created")
    }

    /// Find a secret by searching each candidate keyring in turn.
    /// Returns `Ok(None)` when no keyring holds the key.
    fn find(key: &str) -> Result<Option<(KeyRing, linux_keyutils::Key)>> {
        let description = key_description(key);
        for ring in Self::keyrings() {
            match ring.search(&description) {
                Ok(k) => return Ok(Some((ring, k))),
                // AccessDenied can happen for rings outside our possession
                // chain (e.g. a fresh `keyctl session` ring); keep looking.
                Err(e) if is_miss(&e) || matches!(e, KeyError::AccessDenied) => continue,
                Err(e) => return Err(anyhow!("failed to search keyring: {e}")),
            }
        }
        Ok(None)
    }

    pub fn get(key: &str) -> Result<Option<String>> {
        let Some((_ring, k)) = Self::find(key)? else {
            return Ok(None);
        };

        // Refresh the timeout: an actively used cache stays warm.
        k.set_timeout(timeout_secs())
            .map_err(|e| anyhow!("failed to refresh key timeout: {e}"))?;

        let payload = k
            .read_to_vec()
            .map_err(|e| anyhow!("failed to read key payload: {e}"))?;
        let secret = String::from_utf8(payload).context("key payload is not valid UTF-8")?;
        Ok(Some(secret))
    }

    pub fn set(key: &str, secret: &str) -> Result<()> {
        let description = key_description(key);

        // Revoke any existing key first: reprompting after a wrong password
        // must overwrite the cached secret, and updating a revoked/stale key
        // fails. Then insert fresh into the first keyring that accepts it.
        if let Ok(Some((_ring, existing))) = Self::find(key) {
            let _ = existing.revoke();
        }

        let mut last_err = None;
        for ring in Self::keyrings() {
            match ring.add_key(&description, secret.as_bytes()) {
                Ok(k) => {
                    k.set_timeout(timeout_secs())
                        .map_err(|e| anyhow!("failed to set key timeout: {e}"))?;
                    return Ok(());
                }
                Err(e) => last_err = Some(e),
            }
        }

        Err(anyhow!(
            "failed to add key to any keyring: {}",
            last_err.unwrap_or(KeyError::KeyDoesNotExist)
        ))
    }
}

/// Build the kernel keyring description from the logical key name.
fn key_description(key: &str) -> String {
    format!("{KEY_PREFIX}{key}")
}
