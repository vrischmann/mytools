mod backend;
mod tty;
use anyhow::{bail, Result};
use clap::Parser;
use std::io::{self, Read};

/// Secure credential provider for Ansible vault and become passwords.
///
/// On Linux, passwords are cached in kernel keyring memory for 10 minutes.
/// On macOS, passwords are stored in the Keychain with biometric protection.
#[derive(Parser, Debug)]
#[command(name = "ansible-password-agent", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Store a password.
    ///
    /// Reads from stdin when piped, otherwise prompts interactively.
    /// Examples:
    ///   op read "op://vault/item/field" | ansible-password-agent store
    ///   ansible-password-agent store
    Store {
        /// Type of password to store.
        #[arg(long, default_value = "vault", value_name = "TYPE")]
        r#type: PasswordType,
    },
    /// Retrieve a stored password and write it to stdout.
    ///
    /// If the password is not cached, prompts via the terminal,
    /// stores it, then outputs it. Suitable for use as Ansible's
    /// --vault-password-file or --become-password-file.
    Get {
        /// Type of password to retrieve.
        #[arg(long, default_value = "vault", value_name = "TYPE")]
        r#type: PasswordType,
    },
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

/// Read a secret from stdin, trimming the trailing newline.
///
/// Empty input (just a newline or EOF with no data) is treated as an error.
fn read_stdin() -> Result<String> {
    let mut buf = String::new();
    io::stdin().read_to_string(&mut buf)?;
    let secret = buf.trim_end_matches('\n').trim_end_matches('\r');
    if secret.is_empty() {
        bail!("empty password on stdin");
    }
    Ok(secret.to_owned())
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Store { r#type } => {
            let key = r#type.as_key();

            let secret = if atty::is(atty::Stream::Stdin) {
                tty::prompt_password(r#type.prompt_message())?
            } else {
                read_stdin()?
            };

            backend::set(key, &secret)?;
        }
        Command::Get { r#type } => {
            let key = r#type.as_key();

            match backend::get(key)? {
                Some(secret) => print!("{secret}"),
                None => bail!("no {key} password stored; use `ansible-password-agent store --type {key}` first"),
            }
        }
    }

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
