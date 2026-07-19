use clap::Parser;

/// Display a byte size in human-readable form using binary units (KiB, MiB, …).
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Size in bytes.
    size: u64,
}

const UNITS: [&str; 7] = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];

/// Formats a byte count into a compact human-readable string.
///
/// Values below 1024 are shown in bytes. Larger values are scaled into the
/// largest binary unit that keeps the mantissa below 1024, with up to two
/// decimal places (trailing zeros are trimmed).
fn humanize(bytes: u64) -> String {
    if bytes < 1024 {
        return format!("{bytes}B");
    }

    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }

    // Trim trailing zeros so exact powers of 1024 render cleanly (e.g. "1KiB").
    let mantissa = format!("{value:.2}");
    let mantissa = mantissa.trim_end_matches('0').trim_end_matches('.');
    format!("{}{}", mantissa, UNITS[unit])
}

fn main() {
    let cli = Cli::parse();
    println!("{}", humanize(cli.size));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_bytes() {
        assert_eq!(humanize(0), "0B");
        assert_eq!(humanize(1), "1B");
        assert_eq!(humanize(1023), "1023B");
    }

    #[test]
    fn kib() {
        assert_eq!(humanize(1024), "1KiB");
        assert_eq!(humanize(1536), "1.5KiB");
        assert_eq!(humanize(1600), "1.56KiB");
    }

    #[test]
    fn mib() {
        assert_eq!(humanize(1024 * 1024), "1MiB");
        assert_eq!(humanize(1_234_567), "1.18MiB");
    }

    #[test]
    fn large_units() {
        assert_eq!(humanize(1024u64.pow(3)), "1GiB");
        assert_eq!(humanize(1024u64.pow(4)), "1TiB");
        assert_eq!(humanize(1024u64.pow(5)), "1PiB");
        assert_eq!(humanize(1024u64.pow(6)), "1EiB");
    }
}
