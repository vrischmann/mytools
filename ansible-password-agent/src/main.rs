mod backend;
mod tty;

use anyhow::Result;
use clap::Parser;

/// Secure credential provider for Ansible vault and become passwords.
///
/// Outputs the requested password to stdout. Designed to be used as
/// Ansible's --vault-password-file or --become-password-file.
///
/// On Linux, passwords are cached in kernel keyring memory for 10 minutes.
/// On macOS, passwords are stored in the Keychain with biometric protection.
#[derive(Parser, Debug)]
#[command(name = "ansible-password-agent", version, about)]
struct Cli {
    /// Type of password to retrieve.
    ///
    /// vault    — Ansible vault encryption password (default)
    /// become   — Ansible privilege escalation (sudo) password
    #[arg(long, default_value = "vault", value_name = "TYPE")]
    r#type: PasswordType,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
enum PasswordType {
    Vault,
    Become,
}

impl PasswordType {
    /// Returns the key identifier used for backend storage/retrieval.
    fn as_key(self) -> &'static str {
        match self {
            PasswordType::Vault => "vault",
            PasswordType::Become => "become",
        }
    }

    /// Returns the user-facing prompt message.
    fn prompt_message(self) -> &'static str {
        match self {
            PasswordType::Vault => "Enter Ansible vault password: ",
            PasswordType::Become => "Enter Ansible become password: ",
        }
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let key = cli.r#type.as_key();

    // 1. Try to retrieve from the secure backend.
    match backend::get(key)? {
        Some(secret) => {
            print!("{secret}");
            return Ok(());
        }
        None => {}
    }

    // 2. Not cached — prompt the user.
    let secret = tty::prompt_password(cli.r#type.prompt_message())?;

    // 3. Save to the backend for future use.
    backend::set(key, &secret)?;

    // 4. Output to stdout for Ansible to consume.
    print!("{secret}");
    Ok(())
}

fn main() {
    if run().is_err() {
        std::process::exit(1);
    }
}
