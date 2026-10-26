//! Configuration file parsing (RUST_REWRITE_PLAN.md §4.6).
//!
//! Port of `internal/config/config.go`, with:
//!
//! * the ignore matcher ported verbatim from Go 1.27 `path/filepath/match.go`
//!   (§4.4: the `glob` crate is not parity-equivalent — no backslash escapes,
//!   different negation and `[!…]` handling);
//! * the `$name` template ref parser + expander ported from `regexp`'s
//!   internal `extract`/`expand` (§4.1: `Rules::validate` must be able to
//!   reject templates that reference a non-existent group, which the `regex`
//!   crate's own expansion cannot — it silently substitutes empty).

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use regex::{Captures, Regex};
use serde::Deserialize;

/// Top-level gitjuggling configuration (Go: `config.Config`).
///
/// The workspace map is a [`BTreeMap`] (§4.5): Go iterates a
/// `map[string]*Workspace` in random order; keeping keys sorted makes error
/// messages and any future iteration deterministic.
#[derive(Debug, Default, Deserialize)]
pub struct Config {
    #[serde(default, deserialize_with = "yaml_null_as_default")]
    pub default_workspace: String,
    #[serde(default, deserialize_with = "yaml_null_as_default")]
    pub workspaces: BTreeMap<String, Option<Workspace>>,
}

/// Settings for a single workspace (Go: `config.Workspace`).
///
/// yaml.v3 decodes an explicit null (`key:` with no value) as the zero value
/// for non-pointer fields, while serde errors; [`yaml_null_as_default`]
/// mirrors yaml.v3 so the live config keeps parsing (§4.6).
#[derive(Debug, Default, Deserialize)]
pub struct Workspace {
    #[serde(default, deserialize_with = "yaml_null_as_default")]
    pub root: String,
    #[serde(default, deserialize_with = "yaml_null_as_default")]
    pub github_owners: Vec<String>,
    #[serde(default, deserialize_with = "yaml_null_as_default")]
    pub github_token: String,
    #[serde(default, deserialize_with = "yaml_null_as_default")]
    pub forgejo_url: String,
    #[serde(default, deserialize_with = "yaml_null_as_default")]
    pub forgejo_user: String,
    #[serde(default, deserialize_with = "yaml_null_as_default")]
    pub forgejo_token: String,
    #[serde(default, deserialize_with = "yaml_null_as_default")]
    pub rules: Rules,
    #[serde(default, deserialize_with = "yaml_null_as_default")]
    pub ignore: Vec<String>,
}

impl Workspace {
    /// Checks whether a repo name matches any entry in the ignore list.
    ///
    /// Each entry is matched as a glob pattern against the repo name only,
    /// case-insensitively (both sides lowercased, as in Go). A malformed
    /// pattern is treated as *no match*, not an error.
    pub fn is_ignored(&self, name: &str) -> bool {
        self.ignore
            .iter()
            .any(|pattern| matches!(match_pattern(&pattern.to_lowercase(), &name.to_lowercase()), Ok(true)))
    }
}

/// Where different categories of repos should be placed (Go: `config.Rules`).
#[derive(Debug, Default, Deserialize)]
pub struct Rules {
    #[serde(default, deserialize_with = "yaml_null_as_default")]
    pub base: String,
    #[serde(default, deserialize_with = "yaml_null_as_default")]
    pub forks: String,
    #[serde(default, deserialize_with = "yaml_null_as_default")]
    pub archived: String,
    #[serde(default, deserialize_with = "yaml_null_as_default")]
    pub patterns: Vec<PatternRule>,
}

impl Rules {
    /// Checks the pattern rules for correctness:
    ///
    /// - every pattern compiles as a regex,
    /// - every `to` template references at least one capture, so that
    ///   distinct repos never collapse onto a single directory,
    /// - every reference in `to` resolves to an existing group of the
    ///   pattern.
    pub fn validate(&self) -> Result<()> {
        for (i, rule) in self.patterns.iter().enumerate() {
            if rule.pattern.is_empty() {
                bail!("rules.patterns[{i}]: pattern is empty");
            }
            let re = Regex::new(&rule.pattern)
                .map_err(|err| anyhow!("rules.patterns[{i}]: invalid regex {:?}: {err}", rule.pattern))?;
            if rule.to.is_empty() {
                bail!("rules.patterns[{i}]: to is empty");
            }
            validate_pattern_template(&rule.to, &re).with_context(|| format!("rules.patterns[{i}]"))?;
        }
        Ok(())
    }
}

/// Routes repos whose name matches `pattern` into the directory produced by
/// expanding `to` (Go: `config.PatternRule`).
///
/// `to` is the full final path of the repo and may reference capture groups
/// from `pattern` using `$1`, `${1}`, `${name}`, or `$0` for the whole match.
/// Pattern rules take precedence over the fork/archived/base categories and
/// are evaluated in declared order; the first match wins.
#[derive(Debug, Default, Deserialize)]
pub struct PatternRule {
    #[serde(default, deserialize_with = "yaml_null_as_default")]
    pub pattern: String,
    #[serde(default, deserialize_with = "yaml_null_as_default")]
    pub to: String,
}

/// yaml.v3 decodes an explicit null as the zero value of a non-pointer field;
/// serde errors instead. Mirror yaml.v3 (§4.6) so configs like
/// `github_token:` (empty) keep parsing.
fn yaml_null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// Reads and parses a config file from the given path (Go: `LoadFrom`).
///
/// Every workspace's pattern rules are validated; a null workspace value is
/// skipped, as Go's `LoadFrom` skips a nil `*Workspace`.
pub fn load_from(path: &Path) -> Result<Config> {
    let data =
        std::fs::read_to_string(path).with_context(|| format!("failed to read config file {}", path.display()))?;

    let cfg: Config =
        serde_yaml_ng::from_str(&data).with_context(|| format!("failed to parse config file {}", path.display()))?;

    for (name, ws) in &cfg.workspaces {
        let Some(ws) = ws else { continue };
        ws.rules.validate().with_context(|| format!("workspace {name:?}"))?;
    }

    Ok(cfg)
}

/// Path of the default config file:
/// `<UserConfigDir>/gitjuggling/config.yaml` (Go: `os.UserConfigDir`).
///
/// `dirs::config_dir()` resolves like `os.UserConfigDir` on macOS and Linux
/// (`$HOME/Library/Application Support`, `$XDG_CONFIG_HOME` else
/// `$HOME/.config`).
pub fn default_path() -> Result<PathBuf> {
    let dir = dirs::config_dir().context("failed to determine config directory")?;
    Ok(dir.join("gitjuggling").join("config.yaml"))
}

/// Loads the config from [`default_path`] (Go: `LoadDefault`).
pub fn load_default() -> Result<Config> {
    load_from(&default_path()?)
}

impl Config {
    /// Resolves a workspace by name; an empty name falls back to
    /// `default_workspace` (Go: `GetWorkspace`).
    ///
    /// Deviation: Go stores `*Workspace` map values and returns a nil value
    /// for a null workspace entry (which panics on first use); here that is
    /// reported as not found instead.
    pub fn get_workspace(&self, name: &str) -> Result<&Workspace> {
        let name = if name.is_empty() { self.default_workspace.as_str() } else { name };

        if name.is_empty() {
            bail!("no workspace specified and no default_workspace configured");
        }

        match self.workspaces.get(name) {
            Some(Some(ws)) => Ok(ws),
            _ => bail!("workspace {name:?} not found in config"),
        }
    }
}

// ---------------------------------------------------------------------------
// Pattern-rule template expansion (Go: regexp's internal extract + expand,
// as copied into internal/config/config.go)
// ---------------------------------------------------------------------------

/// Expands `template` against `caps` (from matching a repo name with the rule
/// pattern), mirroring `regexp.ExpandString` exactly:
///
/// - `$name`, `${name}`, `${num}`, `$0` (whole match) are references;
/// - `$$` is an escaped dollar sign;
/// - a malformed reference is treated as literal `$` text;
/// - an unmatched or out-of-range reference expands to the empty string.
///
/// The `regex` crate's own expansion cannot be used instead: it cannot fail,
/// and `Rules::validate` exists precisely to reject templates whose
/// references do not resolve (§4.1).
pub fn expand_template(caps: &Captures<'_>, template: &str) -> String {
    let mut dst = String::new();
    let mut template = template;
    loop {
        let Some((before, after)) = template.split_once('$') else {
            break;
        };
        dst.push_str(before);
        template = after;
        if template.starts_with('$') {
            // Treat $$ as $.
            dst.push('$');
            template = &template[1..];
            continue;
        }
        let Some((name, num, rest)) = extract_ref(template) else {
            // Malformed; treat $ as raw text.
            dst.push('$');
            continue;
        };
        template = rest;
        if num >= 0 {
            if let Some(m) = caps.get(num as usize) {
                dst.push_str(m.as_str());
            }
        } else {
            // Go scans re.subexpNames for the first group with this name;
            // Captures::name resolves the named group the same way (None if
            // it does not exist or did not participate).
            if let Some(m) = caps.name(name) {
                dst.push_str(m.as_str());
            }
        }
    }
    dst.push_str(template);
    dst
}

/// Checks that `tmpl` references at least one capture group of `re` and that
/// every reference resolves to an existing group (Go:
/// `validatePatternTemplate`).
///
/// The reference syntax mirrors `regexp.ExpandString`: `$1`, `${1}`,
/// `${name}`, and `$0` (whole match) are references; `$$` is an escaped
/// dollar sign.
fn validate_pattern_template(tmpl: &str, re: &Regex) -> Result<()> {
    // Go's NumSubexp excludes group 0; captures_len includes it.
    let num_sub = re.captures_len() as i64 - 1;
    // Go's SubexpNames; capture_names yields None for the unnamed groups.
    let names: Vec<&str> = re.capture_names().flatten().collect();
    let mut refs = 0;
    let mut s = tmpl;
    loop {
        let Some((_, after)) = s.split_once('$') else {
            break;
        };
        s = after;
        if s.starts_with('$') {
            // $$ is an escaped dollar sign, not a reference.
            s = &s[1..];
            continue;
        }
        let Some((name, num, rest)) = extract_ref(s) else {
            // Malformed reference: Go treats the $ as literal text, so this
            // does not count as a reference. Keep scanning the rest.
            continue;
        };
        if num >= 0 {
            if num > num_sub {
                bail!(
                    "to {tmpl:?}: ${name} refers to a non-existent group (pattern has {num_sub} capture group(s))"
                );
            }
        } else if !names.contains(&name) {
            bail!("to {tmpl:?}: ${name} is not a named group in the pattern");
        }
        refs += 1;
        s = rest;
    }
    if refs == 0 {
        bail!("to {tmpl:?}: must reference at least one capture (e.g. $1 or $0)");
    }
    Ok(())
}

/// Mirrors the reference parsing of regexp's internal `extract` (copied into
/// config.go as `extractRef`): reads a leading "name" or "{name}" from `s`
/// (the leading `$` already consumed by the caller) and returns the name, its
/// numeric value (-1 for named references) and the unconsumed rest; `None` if
/// no well-formed reference was found.
///
/// Go's numeric-parse quirks are preserved: non-digits or a value reaching
/// 1e8 make it a named reference, and leading zeros (other than "0") do too.
fn extract_ref(s: &str) -> Option<(&str, i64, &str)> {
    if s.is_empty() {
        return None;
    }
    let brace = s.starts_with('{');
    let s = if brace { &s[1..] } else { s };

    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        // &str is always valid UTF-8, so decoding cannot fail here.
        let c = s[i..].chars().next().unwrap();
        if !is_ref_name_char(c) {
            break;
        }
        i += c.len_utf8();
    }
    if i == 0 {
        // empty name is not okay
        return None;
    }
    let name = &s[..i];
    let mut i = i;
    if brace {
        if i >= bytes.len() || bytes[i] != b'}' {
            // missing closing brace
            return None;
        }
        i += 1; // consume '}'
    }

    // Parse number, catching leading zeros and overflow (>= 1e8).
    let mut num: i64 = 0;
    for b in name.bytes() {
        if !b.is_ascii_digit() || num >= 100_000_000 {
            num = -1;
            break;
        }
        num = num * 10 + i64::from(b - b'0');
    }
    // Disallow leading zeros.
    if name.as_bytes()[0] == b'0' && name.len() > 1 {
        num = -1;
    }

    Some((name, num, &s[i..]))
}

/// Go's `unicode.IsLetter(r) || unicode.IsDigit(r) || r == '_'` gate on
/// reference-name characters.
///
/// Approximation: Rust's `char::is_alphabetic` is the Alphabetic property
/// (slightly wider than Go's L* categories) and `is_numeric` also admits Nl
/// and No code points where Go's `IsDigit` is Nd only. These categories can
/// never appear in a group name of either regex engine, so validation and
/// expansion results are unaffected.
fn is_ref_name_char(c: char) -> bool {
    c == '_' || c.is_alphabetic() || c.is_numeric()
}

// ---------------------------------------------------------------------------
// Ignore globs: verbatim port of Go 1.27 path/filepath/match.go (§4.4)
// ---------------------------------------------------------------------------

// The following four items are ported from the Go standard library file
// path/filepath/match.go:
//
//     Copyright 2010 The Go Authors. All rights reserved.
//     Use of this source code is governed by a BSD-style
//     license that can be found in the Go distribution's LICENSE file.
//
// Only the non-Windows semantics are ported (Separator = '/', '\' escapes).

/// Port of Go's `filepath.ErrBadPattern`: the pattern is malformed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchError;

impl fmt::Display for MatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("syntax error in pattern")
    }
}

impl std::error::Error for MatchError {}

/// Port of Go's `filepath.Match`: reports whether `name` matches the shell
/// file name pattern.
///
/// `*` matches any sequence of non-`/` characters, `?` any single non-`/`
/// character, `[...]` a character class (negated with `^`, ranges `a-z`,
/// backslash escapes inside), and matching requires the whole of `name`. The
/// only error is [`MatchError`] for a malformed pattern.
pub fn match_pattern(pattern: &str, name: &str) -> Result<bool, MatchError> {
    let mut pattern = pattern;
    let mut name = name;

    'pattern: while !pattern.is_empty() {
        let (star, chunk, rest) = scan_chunk(pattern);
        pattern = rest;
        if star && chunk.is_empty() {
            // Trailing * matches rest of string unless it has a /.
            return Ok(!name.contains('/'));
        }
        // Look for match at current position.
        match match_chunk(chunk, name) {
            // if we're the last chunk, make sure we've exhausted the name
            // otherwise we'll give a false result even if we could still match
            // using the star
            Ok((t, true)) if t.is_empty() || !pattern.is_empty() => {
                name = t;
                continue 'pattern;
            }
            Ok((_t, _ok)) => {}
            Err(err) => return Err(err),
        }
        if star {
            // Look for match skipping i+1 bytes.
            // Cannot skip /.
            let mut i = 0;
            while i < name.len() && name.as_bytes()[i] != b'/' {
                match match_chunk(chunk, &name[i + 1..]) {
                    Ok((t, true)) => {
                        // if we're the last chunk, make sure we exhausted the name
                        if pattern.is_empty() && !t.is_empty() {
                            i += 1;
                            continue;
                        }
                        name = &name[i + 1..];
                        continue 'pattern;
                    }
                    Ok((_t, false)) => i += 1,
                    Err(err) => return Err(err),
                }
            }
        }
        return Ok(false);
    }
    Ok(name.is_empty())
}

/// Port of Go's `scanChunk`: gets the next segment of pattern, which is a
/// non-star string possibly preceded by a star.
fn scan_chunk(pattern: &str) -> (bool, &str, &str) {
    let mut star = false;
    let mut pattern = pattern;
    while pattern.starts_with('*') {
        pattern = &pattern[1..];
        star = true;
    }
    let bytes = pattern.as_bytes();
    let mut inrange = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                // error check handled in matchChunk: bad pattern.
                if i + 1 < bytes.len() {
                    i += 1;
                }
            }
            b'[' => inrange = true,
            b']' => inrange = false,
            b'*' if !inrange => {
                return (star, &pattern[..i], &pattern[i..]);
            }
            _ => {}
        }
        i += 1;
    }
    (star, pattern, "")
}

/// Port of Go's `matchChunk`: checks whether chunk matches the beginning of
/// s. If so, it returns the remainder of s (after the match).
///
/// Chunk is all single-character operators: literals, char classes, and `?`.
/// A failed match keeps scanning the chunk so that a malformed pattern is
/// still reported (`failed` guards reads from `s` only).
fn match_chunk<'a>(chunk: &str, s: &'a str) -> Result<(&'a str, bool), MatchError> {
    let mut chunk = chunk;
    let mut s = s;
    let mut failed = false;
    while !chunk.is_empty() {
        failed = failed || s.is_empty();
        match chunk.as_bytes()[0] {
            b'[' => {
                // character class
                let mut r = '\0';
                if !failed {
                    let c = s.chars().next().unwrap();
                    s = &s[c.len_utf8()..];
                    r = c;
                }
                chunk = &chunk[1..];
                // possibly negated
                let negated = chunk.starts_with('^');
                if negated {
                    chunk = &chunk[1..];
                }
                // parse all ranges
                let mut matched = false;
                let mut nrange = 0;
                loop {
                    if chunk.starts_with(']') && nrange > 0 {
                        chunk = &chunk[1..];
                        break;
                    }
                    let (lo, rest) = get_esc(chunk)?;
                    chunk = rest;
                    let mut hi = lo;
                    if chunk.starts_with('-') {
                        let (h, rest) = get_esc(&chunk[1..])?;
                        chunk = rest;
                        hi = h;
                    }
                    matched = matched || (lo <= r && r <= hi);
                    nrange += 1;
                }
                failed = failed || matched == negated;
            }
            b'?' => {
                if !failed {
                    failed = s.starts_with('/');
                    let c = s.chars().next().unwrap();
                    s = &s[c.len_utf8()..];
                }
                chunk = &chunk[1..];
            }
            b'\\' => {
                chunk = &chunk[1..];
                if chunk.is_empty() {
                    return Err(MatchError);
                }
                // Go's fallthrough into default:
                if !failed {
                    failed = chunk.as_bytes()[0] != s.as_bytes()[0];
                    s = &s[1..];
                }
                chunk = &chunk[1..];
            }
            _ => {
                if !failed {
                    failed = chunk.as_bytes()[0] != s.as_bytes()[0];
                    s = &s[1..];
                }
                chunk = &chunk[1..];
            }
        }
    }
    if failed {
        return Ok(("", false));
    }
    Ok((s, true))
}

/// Port of Go's `getEsc`: gets a possibly-escaped character from chunk, for a
/// character class.
///
/// On success the returned rest is non-empty (Go errors on empty), so the
/// caller's `chunk[0] == '-'` range check is safe. Go also rejects invalid
/// UTF-8 here; `&str` cannot contain any.
fn get_esc(chunk: &str) -> Result<(char, &str), MatchError> {
    if chunk.is_empty() || chunk.starts_with('-') || chunk.starts_with(']') {
        return Err(MatchError);
    }
    let mut chunk = chunk;
    if chunk.starts_with('\\') {
        chunk = &chunk[1..];
        if chunk.is_empty() {
            return Err(MatchError);
        }
    }
    let r = chunk.chars().next().unwrap();
    let nchunk = &chunk[r.len_utf8()..];
    if nchunk.is_empty() {
        return Err(MatchError);
    }
    Ok((r, nchunk))
}

#[cfg(test)]
mod tests {
    use regex::Regex;

    use super::*;

    /// Redacted copy of the live config (§4.6): CI cannot read
    /// ~/Library/Application Support/gitjuggling/config.yaml, and a test that
    /// silently skips when the file is missing proves nothing.
    const FIXTURE: &str = include_str!("../tests/fixtures/config.yaml");

    /// Test helper mirroring Go's parseYAML: parses YAML into a Config
    /// without running validation.
    fn parse_yaml(input: &str) -> Result<Config> {
        Ok(serde_yaml_ng::from_str(input)?)
    }

    // go test-run: internal/config/config_test.go TestParseMinimalConfig
    #[test]
    fn test_parse_minimal_config() {
        let input = "
workspace:
  personal:
    root: /home/user/dev
    github_owners: [vrischmann]
    rules:
      base: /home/user/dev/repos
";
        let cfg = parse_yaml(input).unwrap();

        assert!(cfg.default_workspace.is_empty());

        let ws = cfg.workspaces.get("personal").expect("expected workspace 'personal' to exist");
        let ws = ws.as_ref().unwrap();

        assert_eq!(ws.root, "/home/user/dev");
        assert_eq!(ws.github_owners, vec!["vrischmann".to_string()]);

        assert!(ws.forgejo_url.is_empty());
        assert!(ws.forgejo_user.is_empty());
        assert!(ws.forgejo_token.is_empty());

        assert_eq!(ws.rules.base, "/home/user/dev/repos");
        assert!(ws.rules.forks.is_empty());
        assert!(ws.rules.archived.is_empty());
    }

    // go test-run: internal/config/config_test.go TestParseFullConfig
    #[test]
    fn test_parse_full_config() {
        let input = "
default_workspace: personal

workspace:
  personal:
    root: /home/user/dev
    github_owners: [vrischmann]
    forgejo_url: https://git.example.com
    forgejo_user: vincent
    forgejo_token: \"op://vault/item/field\"
    rules:
      base: /home/user/dev/repos
      forks: /home/user/dev/forks
      archived: /home/user/dev/archived

  work:
    root: /home/user/work
    github_owners: [MyOrg]
    rules:
      base: /home/user/work/repos
";
        let cfg = parse_yaml(input).unwrap();

        assert_eq!(cfg.default_workspace, "personal");
        assert_eq!(cfg.workspaces.len(), 2);

        let personal = cfg.workspaces["personal"].as_ref().unwrap();
        assert_eq!(personal.forgejo_url, "https://git.example.com");
        assert_eq!(personal.forgejo_user, "vincent");
        assert_eq!(personal.rules.forks, "/home/user/dev/forks");
        assert_eq!(personal.rules.archived, "/home/user/dev/archived");

        let work = cfg.workspaces["work"].as_ref().unwrap();
        assert_eq!(work.github_owners, vec!["MyOrg".to_string()]);
        assert!(work.forgejo_url.is_empty());
    }

    // go test-run: internal/config/config_test.go TestGetWorkspace
    #[test]
    fn test_get_workspace() {
        let input = "
default_workspace: personal

workspace:
  personal:
    root: /home/user/dev
    github_owners: [vrischmann]
    rules:
      base: /home/user/dev/repos
";
        let cfg = parse_yaml(input).unwrap();

        // Explicit name
        let ws = cfg.get_workspace("personal").unwrap();
        assert_eq!(ws.root, "/home/user/dev");

        // Default fallback
        let ws = cfg.get_workspace("").unwrap();
        assert_eq!(ws.root, "/home/user/dev");

        // Missing workspace
        assert!(cfg.get_workspace("nonexistent").is_err());
    }

    // go test-run: internal/config/config_test.go TestParseConfigWithIgnore
    #[test]
    fn test_parse_config_with_ignore() {
        let input = "
workspace:
  personal:
    root: /home/user/dev
    github_owners: [vrischmann]
    ignore:
      - dotfiles
      - \"temp-*\"
    rules:
      base: /home/user/dev/repos
";
        let cfg = parse_yaml(input).unwrap();

        let ws = cfg.workspaces["personal"].as_ref().unwrap();
        assert_eq!(ws.ignore, vec!["dotfiles".to_string(), "temp-*".to_string()]);
    }

    // go test-run: internal/config/config_test.go TestIsIgnored
    #[test]
    fn test_is_ignored() {
        let ws = Workspace {
            ignore: vec!["dotfiles".into(), "temp-*".into(), "test?repo".into()],
            ..Default::default()
        };

        let tests: &[(&str, bool)] = &[
            // Exact match
            ("dotfiles", true),
            ("Dotfiles", true), // case-insensitive
            ("my-dotfiles", false),
            // Glob pattern
            ("temp-foo", true),
            ("temp-bar-baz", true),
            ("TEMP-xyz", true), // case-insensitive
            ("temp", false),    // partial match doesn't count
            // Glob with ?
            ("test-repo", true),
            ("testXrepo", true),
            ("testXXrepo", false),
            // No match
            ("myproject", false),
            ("", false),
        ];

        for (name, want) in tests {
            let got = ws.is_ignored(name);
            assert_eq!(got, *want, "IsIgnored({name:?}) = {got}, want {want}");
        }
    }

    // go test-run: internal/config/config_test.go TestIsIgnoredEmpty
    #[test]
    fn test_is_ignored_empty() {
        let ws = Workspace::default();
        assert!(!ws.is_ignored("anything"));
    }

    // go test-run: internal/config/config_test.go TestRulesValidate
    #[test]
    fn test_rules_validate() {
        struct Case {
            name: &'static str,
            rules: Rules,
            want_err: &'static str, // substring of the error; empty = no error
        }
        // PatternRule/Rules literal construction helper
        fn rule(pattern: &str, to: &str) -> PatternRule {
            PatternRule {
                pattern: pattern.to_string(),
                to: to.to_string(),
            }
        }
        fn rules(patterns: Vec<PatternRule>) -> Rules {
            Rules {
                patterns,
                ..Default::default()
            }
        }

        let tests = vec![
            Case {
                name: "no patterns",
                rules: rules(vec![]),
                want_err: "",
            },
            Case {
                name: "valid capture",
                rules: rules(vec![rule(r"^workspace-(.+)$", "/ws/$1")]),
                want_err: "",
            },
            Case {
                name: "valid named group",
                rules: rules(vec![rule(r"^workspace-(?P<id>.+)$", "/ws/${id}")]),
                want_err: "",
            },
            Case {
                name: "valid whole match with no capture groups",
                rules: rules(vec![rule(r"^workspace-.+$", "/ws/$0")]),
                want_err: "",
            },
            Case {
                name: "to without capture",
                rules: rules(vec![rule(r"^workspace-(.+)$", "/ws/flat")]),
                want_err: "must reference at least one capture",
            },
            Case {
                name: "to is only an escaped dollar",
                rules: rules(vec![rule(r"^x$", "/ws/$$1")]),
                want_err: "must reference at least one capture",
            },
            Case {
                name: "invalid regex",
                rules: rules(vec![rule(r"^workspace-[", "/ws/$0")]),
                want_err: "invalid regex",
            },
            Case {
                name: "out of range group",
                rules: rules(vec![rule(r"^workspace-(.+)$", "/ws/$2")]),
                want_err: "non-existent group",
            },
            Case {
                name: "unknown named group",
                rules: rules(vec![rule(r"^workspace-(.+)$", "/ws/${nope}")]),
                want_err: "not a named group",
            },
            Case {
                name: "empty pattern",
                rules: rules(vec![rule("", "/ws/$0")]),
                want_err: "pattern is empty",
            },
            Case {
                name: "empty to",
                rules: rules(vec![rule(r"^x$", "")]),
                want_err: "to is empty",
            },
        ];
        for tt in tests {
            let err = tt.rules.validate().err();
            if tt.want_err.is_empty() {
                assert!(err.is_none(), "{}: unexpected error: {err:?}", tt.name);
                continue;
            }
            let err = err.unwrap_or_else(|| panic!("{}: expected error", tt.name));
            assert!(
                err.to_string().contains(tt.want_err),
                "{}: error {:?} does not contain {:?}",
                tt.name,
                err,
                tt.want_err
            );
        }
    }

    // go test-run: internal/config/config_test.go TestLoadFromRejectsInvalidPattern
    #[test]
    fn test_load_from_rejects_invalid_pattern() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        let content = "
workspace:
  personal:
    root: /home/user/dev
    github_owners: [vrischmann]
    rules:
      base: /home/user/dev/repos
      patterns:
        - pattern: '^workspace-(.+)$'
          to: /home/user/dev/workspaces/flat
";
        std::fs::write(&path, content).unwrap();

        let err = load_from(&path).expect_err("expected error");
        assert!(
            err.to_string().contains("must reference at least one capture"),
            "error {err:#} does not mention the missing capture reference"
        );
    }

    // §4.6 fixture test: parses a committed redacted copy of the live config
    // and asserts every field, including op:// token strings and
    // 4-space-indented maps.
    #[test]
    fn test_load_fixture_redacted_live_config() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, FIXTURE).unwrap();

        let cfg = load_from(&path).unwrap();

        assert_eq!(cfg.default_workspace, "personal");
        assert_eq!(cfg.workspaces.len(), 2);

        let personal = cfg.workspaces["personal"].as_ref().unwrap();
        assert_eq!(personal.root, "/home/user/dev");
        assert_eq!(personal.github_owners, vec!["vrischmann".to_string()]);
        assert_eq!(personal.github_token, ""); // explicit empty (null) value
        assert_eq!(personal.forgejo_url, "https://git.example.com");
        assert_eq!(personal.forgejo_user, "vincent");
        assert_eq!(personal.forgejo_token, "op://Vault/Item/Field");
        assert_eq!(personal.rules.base, "/home/user/dev/repos");
        assert_eq!(personal.rules.forks, "/home/user/dev/forks");
        assert_eq!(personal.rules.archived, "/home/user/dev/archived");
        assert_eq!(personal.rules.patterns.len(), 1);
        assert_eq!(personal.rules.patterns[0].pattern, "^workspace-(.+)$");
        assert_eq!(personal.rules.patterns[0].to, "/home/user/dev/workspaces/$1");
        assert_eq!(personal.ignore, vec!["dotfiles".to_string(), "temp-*".to_string()]);

        let work = cfg.workspaces["work"].as_ref().unwrap();
        assert_eq!(work.root, "/home/user/work");
        assert_eq!(work.github_owners, vec!["MyOrg".to_string()]);
        assert!(work.forgejo_url.is_empty());

        // §4.2 canary: the live pattern compiles with the Rust regex crate
        // and expands as Go's regexp would.
        let re = Regex::new(&personal.rules.patterns[0].pattern).unwrap();
        let caps = re.captures("workspace-foo").unwrap();
        assert_eq!(expand_template(&caps, "x/$0/y"), "x/workspace-foo/y");
    }

    // §5 M1: default path resolution matches Go's os.UserConfigDir-based
    // LoadDefault (dirs::config_dir resolves the same on macOS/Linux).
    //
    // Deliberately does NOT call load_default(): the default path points at
    // the user's live config, which tests must never read (§4.6).
    #[test]
    fn test_default_path() {
        let path = default_path().unwrap();
        assert!(path.is_absolute(), "default path {path:?} is not absolute");
        assert!(path.ends_with("gitjuggling/config.yaml"), "default path {path:?}");
    }

    // §4.1: expansion must mirror regexp.ExpandString. Every expected value
    // below was produced by the Go oracle (regexp.ExpandString, go1.27.3)
    // with pattern ^workspace-(.+)$ against "workspace-foo".
    #[test]
    fn test_expand_template_go_semantics() {
        let re = Regex::new(r"^workspace-(.+)$").unwrap();
        let caps = re.captures("workspace-foo").unwrap();

        let tests: &[(&str, &str)] = &[
            ("workspaces/$1", "workspaces/foo"),
            ("workspaces/${1}", "workspaces/foo"),
            ("workspaces/$name", "workspaces/"), // unknown ref → empty (collapse!)
            ("a/$2/b", "a//b"),                  // out of range → empty
            ("${nope}", ""),                     // unknown named → empty
            ("p/$$1", "p/$1"),                   // $$ is an escaped dollar
            ("x/$0/y", "x/workspace-foo/y"),
            ("$0a", ""),       // "0a" parses as a *named* ref, not $0 + "a"
            ("$1x", ""),       // "1x" parses as a *named* ref, not $1 + "x"
            ("a/${01}b", "a/b"), // leading zero → named ref → empty
            ("${100000000}", ""), // 1e8 → numeric, out of range → empty
            ("${999999999}", ""), // 9 digits → still numeric → out of range
            ("${1234567890}", ""), // ≥ 1e8 mid-parse → named ref → empty
            ("abc$", "abc$"),  // trailing $ is literal
            ("$", "$"),
            ("a$$", "a$"),
            ("$}", "$}"), // malformed ref → literal $
            ("$1$2", "foo"), // $2 out of range for this 1-group pattern
        ];
        for (tmpl, want) in tests {
            let got = expand_template(&caps, tmpl);
            assert_eq!(got, *want, "expand({tmpl:?})");
        }

        // Adjacent refs with two groups.
        let re2 = Regex::new(r"^(a)(b)$").unwrap();
        let caps2 = re2.captures("ab").unwrap();
        assert_eq!(expand_template(&caps2, "$2$1"), "ba");

        // A group that did not participate in the match expands to empty,
        // like Go's match[2i] < 0 check.
        let re3 = Regex::new(r"^(x)?y$").unwrap();
        let caps3 = re3.captures("y").unwrap();
        assert_eq!(expand_template(&caps3, "[$1]"), "[]");
    }

    // §4.1: the 1e8 overflow and leading-zero boundaries, on the validation
    // side (they change which error fires). Expected messages confirmed by
    // the Go oracle (config.go's validatePatternTemplate, go1.27.3).
    #[test]
    fn test_validate_template_numeric_boundaries() {
        let re = Regex::new(r"^workspace-(.+)$").unwrap();

        let err = validate_pattern_template("/ws/${999999999}", &re).expect_err("numeric out of range");
        assert!(
            err.to_string().contains("$999999999 refers to a non-existent group"),
            "{err}"
        );

        let err = validate_pattern_template("/ws/${1234567890}", &re).expect_err("overflow → named");
        assert!(
            err.to_string().contains("$1234567890 is not a named group"),
            "{err}"
        );

        let err = validate_pattern_template("/ws/${01}", &re).expect_err("leading zero → named");
        assert!(err.to_string().contains("$01 is not a named group"), "{err}");

        // $1 is in range, ${100000000} (1e8) is numeric but out of range.
        assert!(validate_pattern_template("/ws/$1", &re).is_ok());
        let err = validate_pattern_template("/ws/${100000000}", &re).expect_err("1e8 out of range");
        assert!(
            err.to_string().contains("$100000000 refers to a non-existent group"),
            "{err}"
        );
    }

    // Verbatim port of Go 1.27's path/filepath/match_test.go matchTests table
    // (the rows relevant on non-Windows, where '\' escapes).
    #[test]
    fn test_match_pattern_go_stdlib_table() {
        // (pattern, s, Ok(expected) | Err = ErrBadPattern)
        let tests: &[(&str, &str, Result<bool, MatchError>)] = &[
            ("abc", "abc", Ok(true)),
            ("*", "abc", Ok(true)),
            ("*c", "abc", Ok(true)),
            ("a*", "a", Ok(true)),
            ("a*", "abc", Ok(true)),
            ("a*", "ab/c", Ok(false)),
            ("a*/b", "abc/b", Ok(true)),
            ("a*/b", "a/c/b", Ok(false)),
            ("a*b*c*d*e*/f", "axbxcxdxe/f", Ok(true)),
            ("a*b*c*d*e*/f", "axbxcxdxexxx/f", Ok(true)),
            ("a*b*c*d*e*/f", "axbxcxdxe/xxx/f", Ok(false)),
            ("a*b*c*d*e*/f", "axbxcxdxexxx/fff", Ok(false)),
            ("a*b?c*x", "abxbbxdbxebxczzx", Ok(true)),
            ("a*b?c*x", "abxbbxdbxebxczzy", Ok(false)),
            ("ab[c]", "abc", Ok(true)),
            ("ab[b-d]", "abc", Ok(true)),
            ("ab[e-g]", "abc", Ok(false)),
            ("ab[^c]", "abc", Ok(false)),
            ("ab[^b-d]", "abc", Ok(false)),
            ("ab[^e-g]", "abc", Ok(true)),
            (r"a\*b", "a*b", Ok(true)),
            (r"a\*b", "ab", Ok(false)),
            ("a?b", "a☺b", Ok(true)),
            ("a[^a]b", "a☺b", Ok(true)),
            ("a???b", "a☺b", Ok(false)),
            ("a[^a][^a][^a]b", "a☺b", Ok(false)),
            ("[a-ζ]*", "α", Ok(true)),
            ("*[a-ζ]", "A", Ok(false)),
            ("a?b", "a/b", Ok(false)),
            ("a*b", "a/b", Ok(false)),
            (r"[\]a]", "]", Ok(true)),
            (r"[\-]", "-", Ok(true)),
            (r"[x\-]", "x", Ok(true)),
            (r"[x\-]", "-", Ok(true)),
            (r"[x\-]", "z", Ok(false)),
            (r"[\-x]", "x", Ok(true)),
            (r"[\-x]", "-", Ok(true)),
            (r"[\-x]", "a", Ok(false)),
            ("[]a]", "]", Err(MatchError)),
            ("[-]", "-", Err(MatchError)),
            ("[x-]", "x", Err(MatchError)),
            ("[x-]", "-", Err(MatchError)),
            ("[x-]", "z", Err(MatchError)),
            ("[-x]", "x", Err(MatchError)),
            ("[-x]", "-", Err(MatchError)),
            ("[-x]", "a", Err(MatchError)),
            (r"\", "a", Err(MatchError)),
            ("[a-b-c]", "a", Err(MatchError)),
            ("[", "a", Err(MatchError)),
            ("[^", "a", Err(MatchError)),
            ("[^bc", "a", Err(MatchError)),
            ("a[", "a", Err(MatchError)),
            ("a[", "ab", Err(MatchError)),
            ("a[", "x", Err(MatchError)),
            ("a/b[", "x", Err(MatchError)),
            ("*x", "xxx", Ok(true)),
        ];

        for (pattern, s, want) in tests {
            let got = match_pattern(pattern, s);
            assert_eq!(got, *want, "match_pattern({pattern:?}, {s:?})");
        }
    }
}
