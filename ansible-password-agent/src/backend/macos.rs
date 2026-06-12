use anyhow::{bail, Context, Result};
use security_framework::passwords::{self, PasswordOptions};

use super::PasswordBackend;

/// Keychain service name used to namespace all passwords.
const KEYCHAIN_SERVICE: &str = "ansible-password-agent";

/// macOS backend using the Keychain Services API.
///
/// Passwords are stored permanently in the Keychain, encrypted at rest
/// with hardware-backed keys (Secure Enclave on Apple Silicon).
/// No per-access biometric gate is used, as the data protection keychain
/// requires entitlements not available to CLI tools.
pub struct MacOSBackend;

impl PasswordBackend for MacOSBackend {
    fn get(key: &str) -> Result<Option<String>> {
        let options = PasswordOptions::new_generic_password(KEYCHAIN_SERVICE, key);

        match passwords::generic_password(options) {
            Ok(bytes) => {
                let secret =
                    String::from_utf8(bytes).context("keychain payload is not valid UTF-8")?;
                Ok(Some(secret))
            }
            Err(e) => {
                let code = e.code();
                // Item does not exist — first run, caller should prompt.
                if code == -25300 {
                    // errSecItemNotFound
                    return Ok(None);
                }
                // User cancelled the biometric / password prompt.
                if code == -128 {
                    // errSecUserCanceled
                    bail!("user cancelled authentication prompt");
                }
                Err(e).context("failed to retrieve password from keychain")
            }
        }
    }

    fn set(key: &str, secret: &str) -> Result<()> {
        let mut options = PasswordOptions::new_generic_password(KEYCHAIN_SERVICE, key);

        // Disable iCloud sync — keep the secret on this device only.
        options.set_access_synchronized(Some(false));

        passwords::set_generic_password_options(secret.as_bytes(), options)
            .context("failed to save password to keychain")?;

        Ok(())
    }
}
