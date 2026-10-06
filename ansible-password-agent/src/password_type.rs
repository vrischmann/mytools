/// Type of password to retrieve.
///
/// vault  — Ansible vault encryption password (default)
/// become — Ansible privilege escalation (sudo) password
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasswordType {
    Vault,
    Become,
}

impl PasswordType {
    /// Returns the key identifier used for backend storage/retrieval.
    pub fn as_key(self) -> &'static str {
        match self {
            PasswordType::Vault => "vault",
            PasswordType::Become => "become",
        }
    }

    /// Path of the matching 1Password secret, for `op read`.
    pub fn op_secret(self) -> &'static str {
        match self {
            PasswordType::Vault => "op://Private/My Vault ansible/password",
            PasswordType::Become => "op://Private/sudo password/password",
        }
    }
}
