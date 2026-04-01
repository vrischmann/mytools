#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "macos")]
pub mod macos;

use anyhow::Result;

/// Trait for platform-specific secure password storage backends.
pub trait PasswordBackend {
    /// Retrieve a cached password by key.
    ///
    /// Returns `Ok(Some(password))` if found and valid, `Ok(None)` if not
    /// found or expired, or an error for backend failures.
    fn get(key: &str) -> Result<Option<String>>;

    /// Store a password under the given key.
    ///
    /// On Linux this sets a 600-second kernel timeout.
    /// On macOS this stores in Keychain with biometric access control.
    fn set(key: &str, secret: &str) -> Result<()>;
}

/// Retrieve a cached password using the appropriate platform backend.
pub fn get(key: &str) -> Result<Option<String>> {
    #[cfg(target_os = "linux")]
    {
        linux::LinuxBackend::get(key)
    }
    #[cfg(target_os = "macos")]
    {
        macos::MacOSBackend::get(key)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        compile_error!("ansible-password-agent only supports Linux and macOS")
    }
}

/// Store a password using the appropriate platform backend.
pub fn set(key: &str, secret: &str) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        linux::LinuxBackend::set(key, secret)
    }
    #[cfg(target_os = "macos")]
    {
        macos::MacOSBackend::set(key, secret)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        compile_error!("ansible-password-agent only supports Linux and macOS")
    }
}
