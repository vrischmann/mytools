use ansible_password_agent::backend::LinuxBackend;
use ansible_password_agent::password_type::PasswordType;
use anyhow::{bail, Context, Result};
use clap::Parser;

/// Session cache for Ansible vault and become passwords on Linux.
///
/// Designed to be used as Ansible's --vault-password-file or
/// --become-password-file (see ansible.cfg).
///
/// Passwords are cached in kernel keyring memory (shared by every process
/// of the shell session) and expire after one hour by default — tune with
/// APA_TIMEOUT_SECS. On a cache miss, the secret is fetched with
/// `op read`; the 1Password approval happens once, and every later call —
/// including from coding agents — is prompt-free until expiry.
#[derive(Parser, Debug)]
#[command(name = "ansible-password-agent", version, about)]
struct Cli {
    /// Type of password to retrieve.
    #[arg(long, default_value = "vault", value_name = "TYPE")]
    r#type: PasswordType,

    /// Ignore the cache and fetch the secret from 1Password again.
    #[arg(long)]
    refresh: bool,
}

fn fetch_from_op(password_type: PasswordType) -> Result<String> {
    let output = std::process::Command::new("op")
        .args([
            "read",
            "--account",
            "my.1password.eu",
            password_type.op_secret(),
        ])
        .output()
        .map_err(|e| anyhow::anyhow!("failed to run `op read`: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("`op read` failed: {}", stderr.trim());
    }

    let secret = String::from_utf8(output.stdout)
        .context("`op read` returned non-UTF-8 output")?
        .trim_end_matches(['\r', '\n'])
        .to_string();

    if secret.is_empty() {
        bail!("`op read` returned an empty secret");
    }

    Ok(secret)
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let key = cli.r#type.as_key();

    // 1. Try to retrieve from the cache.
    if !cli.refresh {
        if let Some(secret) = LinuxBackend::get(key)? {
            print!("{secret}");
            return Ok(());
        }
    }

    // 2. Cache miss (or --refresh): fetch from 1Password. If the vault is
    //    unlocked this is silent; otherwise it prompts exactly once.
    let secret = fetch_from_op(cli.r#type)?;

    // 3. Cache for the whole shell session.
    LinuxBackend::set(key, &secret)?;

    // 4. Output to stdout for Ansible to consume.
    print!("{secret}");
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
