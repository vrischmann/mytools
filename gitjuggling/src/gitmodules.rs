//! `.gitmodules` parsing (port of `internal/gitmodules/gitmodules.go`).
//!
//! This is a hand-written parser, not a standard INI parser: sections are
//! `[submodule "<name>"]`, key/value pairs are `key = value` lines terminated
//! by end-of-line, and a quoted name ends at the literal `"]` sequence. The
//! port follows the Go parser statement by statement, including its quirks:
//!
//! * the section header must be exactly `[submodule ` (with the trailing
//!   space);
//! * the parser stops at EOF mid-way and keeps what it has (the `eof` error
//!   is swallowed by the parse loop);
//! * `eat_whitespace` also eats `\r`, so CRLF files parse;
//! * values are `strings.TrimSpace`d.

use std::collections::HashMap;
use std::fmt;

use anyhow::Result;

/// A single submodule entry in a .gitmodules file (Go: `gitmodules.GitSubmodule`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitSubmodule {
    pub name: String,
    pub path: String,
    pub url: String,
    pub branch: String,
}

/// All parsed submodules from a .gitmodules file (Go: `gitmodules.GitModules`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitModules {
    pub submodules: Vec<GitSubmodule>,
}

impl GitModules {
    /// Checks if any submodule has the given path (Go: `Contains`).
    pub fn contains(&self, path: &str) -> bool {
        self.submodules.iter().any(|sub| sub.path == path)
    }
}

/// Parses the content of a .gitmodules file (Go: `Parse`).
pub fn parse(input: &str) -> Result<GitModules> {
    let mut p = Parser { input };
    let result = p.parse()?;
    Ok(result)
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Internal parse errors, mirroring the Go parser's `fmt.Errorf` messages.
/// `Eof` is special-cased by the parse loop (it ends parsing instead of
/// failing), exactly like Go's `err.Error() == "eof"` check.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ParseError {
    Eof,
    ExpectedSubmodule(String),
    ExpectedOpeningQuote(String),
    ExpectedClosingQuote,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Eof => f.write_str("eof"),
            // Go: fmt.Errorf("expected [submodule ...], got %q", p.peek(20))
            Self::ExpectedSubmodule(got) => write!(f, "expected [submodule ...], got {got:?}"),
            // Go: fmt.Errorf("expected opening quote, got %q", p.peek(5))
            Self::ExpectedOpeningQuote(got) => write!(f, "expected opening quote, got {got:?}"),
            // Go: fmt.Errorf("expected closing \"]")
            Self::ExpectedClosingQuote => f.write_str("expected closing \"]"),
        }
    }
}

impl std::error::Error for ParseError {}

struct Parser<'a> {
    input: &'a str,
}

impl Parser<'_> {
    fn parse(&mut self) -> Result<GitModules, ParseError> {
        let mut result = GitModules::default();

        while !self.input.is_empty() {
            self.eat_whitespace();

            if self.input.is_empty() {
                break;
            }

            match self.parse_submodule() {
                Ok(submodule) => result.submodules.push(submodule),
                Err(ParseError::Eof) => break,
                Err(err) => return Err(err),
            }
        }

        Ok(result)
    }

    fn parse_submodule(&mut self) -> Result<GitSubmodule, ParseError> {
        self.eat_whitespace();

        // Parse [submodule "<foo>"]
        if !self.input.starts_with("[submodule ") {
            return Err(ParseError::ExpectedSubmodule(self.peek(20)));
        }
        self.input = &self.input["[submodule ".len()..];

        let name = self.parse_quoted_string()?;

        // Parse key=value pairs until next section or EOF
        let mut kvs: HashMap<String, String> = HashMap::new();

        loop {
            self.eat_whitespace();

            if self.input.is_empty() || self.input.starts_with('[') {
                break;
            }

            let Ok(key) = self.parse_until(b'=') else {
                break;
            };
            let value = self.parse_until_eol();

            kvs.insert(key, value);
        }

        Ok(GitSubmodule {
            name,
            path: kvs.remove("path").unwrap_or_default(),
            url: kvs.remove("url").unwrap_or_default(),
            branch: kvs.remove("branch").unwrap_or_default(),
        })
    }

    fn parse_quoted_string(&mut self) -> Result<String, ParseError> {
        if !self.input.starts_with('"') {
            return Err(ParseError::ExpectedOpeningQuote(self.peek(5)));
        }
        self.input = &self.input[1..];

        // Find the closing "]
        let Some(idx) = self.input.find("\"]") else {
            return Err(ParseError::ExpectedClosingQuote);
        };

        let result = self.input[..idx].to_string();
        self.input = &self.input[idx + 2..];

        Ok(result)
    }

    fn parse_until(&mut self, ch: u8) -> Result<String, ParseError> {
        for (i, &b) in self.input.as_bytes().iter().enumerate() {
            if b == ch {
                let result = self.input[..i].trim().to_string();
                self.input = &self.input[i + 1..];
                return Ok(result);
            }
        }
        Err(ParseError::Eof)
    }

    fn parse_until_eol(&mut self) -> String {
        match self.input.find('\n') {
            Some(i) => {
                let result = self.input[..i].trim().to_string();
                self.input = &self.input[i + 1..];
                result
            }
            // No newline found — rest of input is the value
            None => {
                let result = self.input.trim().to_string();
                self.input = "";
                result
            }
        }
    }

    fn eat_whitespace(&mut self) {
        while matches!(self.input.as_bytes().first(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.input = &self.input[1..];
        }
    }

    /// Go returns the raw byte prefix; here the prefix may cut a UTF-8
    /// sequence, so it is decoded lossily (error text only).
    fn peek(&self, n: usize) -> String {
        let n = n.min(self.input.len());
        String::from_utf8_lossy(&self.input.as_bytes()[..n]).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // go test-run: internal/gitmodules/gitmodules_test.go TestParseGitModules
    #[test]
    fn test_parse_git_modules() {
        let input = "
[submodule \"foobar\"]
    path = foo
    url = git@github.com:foo/bar.git
[submodule \"cpc\"]
    path = cpclol
    url = git@github.com:foo/cpc.git

[submodule \"yep\"]
    path = yop
    url = git@github.com:foo/yop.git
    branch = master
    foo = bar
";

        let result = parse(input).unwrap();

        assert_eq!(result.submodules.len(), 3, "expected 3 submodules");

        // First submodule
        let sub1 = &result.submodules[0];
        assert_eq!(sub1.name, "foobar");
        assert_eq!(sub1.path, "foo");
        assert_eq!(sub1.url, "git@github.com:foo/bar.git");

        // Second submodule
        let sub2 = &result.submodules[1];
        assert_eq!(sub2.name, "cpc");
        assert_eq!(sub2.path, "cpclol");
        assert_eq!(sub2.url, "git@github.com:foo/cpc.git");

        // Third submodule (with branch and extra key)
        let sub3 = &result.submodules[2];
        assert_eq!(sub3.name, "yep");
        assert_eq!(sub3.path, "yop");
        assert_eq!(sub3.url, "git@github.com:foo/yop.git");
        assert_eq!(sub3.branch, "master");
    }

    // go test-run: internal/gitmodules/gitmodules_test.go TestContains
    #[test]
    fn test_contains() {
        let input = "
[submodule \"foo\"]
    path = mypath
    url = git@github.com:foo/bar.git
";
        let result = parse(input).unwrap();

        assert!(result.contains("mypath"), "expected Contains(mypath) to be true");
        assert!(!result.contains("otherpath"), "expected Contains(otherpath) to be false");
    }

    // Extra pins on the hand-written parser's edge behavior (§4.8-adjacent:
    // freeze what the Go parser does so a rewrite cannot change it silently).
    #[test]
    fn test_parse_empty_input() {
        let result = parse("").unwrap();
        assert!(result.submodules.is_empty());
        assert!(!result.contains("anything"));
    }

    #[test]
    fn test_parse_rejects_missing_submodule_header() {
        let err = parse("  path = foo\n").unwrap_err();
        assert!(
            err.to_string().starts_with("expected [submodule ...], got"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn test_parse_rejects_unterminated_name() {
        let err = parse("[submodule \"foo\n    path = foo\n").unwrap_err();
        assert_eq!(err.to_string(), "expected closing \"]");
    }

    #[test]
    fn test_parse_eof_mid_section() {
        // A section whose last key/value line has no trailing newline still
        // parses (parse_until_eol takes the rest of input).
        let result = parse("[submodule \"a\"]\n    path = p").unwrap();
        assert_eq!(result.submodules.len(), 1);
        assert_eq!(result.submodules[0].path, "p");
        assert_eq!(result.submodules[0].url, "");

        // A section truncated before the '=' leaves the rest of the input
        // unconsumed; the parse loop then sees a chunk that is not a
        // `[submodule ` header and the whole parse fails (the swallowed `eof`
        // only applies inside the key/value loop).
        let err = parse("[submodule \"b\"]\n    path").unwrap_err();
        assert!(
            err.to_string().starts_with("expected [submodule ...], got"),
            "unexpected error: {err}"
        );
    }
}
