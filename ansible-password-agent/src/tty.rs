use anyhow::{bail, Context, Result};

/// Prompt the user for a password via `/dev/tty`, ensuring it never echoes
/// to the terminal and is independent of any stdin redirection.
///
/// Returns an error if the prompt fails or the user enters an empty string
/// (interpreted as cancellation).
pub fn prompt_password(msg: &str) -> Result<String> {
    let password = rpassword::prompt_password(msg).context("failed to read password from tty")?;
    if password.is_empty() {
        bail!("user cancelled password prompt (empty input)");
    }
    Ok(password)
}
