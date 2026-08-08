package config

import (
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"strings"
	"unicode"
	"unicode/utf8"

	"gopkg.in/yaml.v3"
)

// Config represents the top-level gitjuggling configuration.
type Config struct {
	DefaultWorkspace string                `yaml:"default_workspace"`
	Workspaces       map[string]*Workspace `yaml:"workspace"`
}

// Workspace defines the settings for a single workspace.
type Workspace struct {
	Root         string   `yaml:"root"`
	GitHubOwners []string `yaml:"github_owners"`
	GitHubToken  string   `yaml:"github_token"`
	ForgejoURL   string   `yaml:"forgejo_url"`
	ForgejoUser  string   `yaml:"forgejo_user"`
	ForgejoToken string   `yaml:"forgejo_token"`
	Rules        Rules    `yaml:"rules"`
	Ignore       []string `yaml:"ignore"`
}

// IsIgnored checks whether a repo name matches any entry in the ignore list.
// Each entry is matched as a glob pattern against the repo name only.
// If the ignore list is empty, this always returns false.
func (ws *Workspace) IsIgnored(name string) bool {
	for _, pattern := range ws.Ignore {
		matched, err := filepath.Match(strings.ToLower(pattern), strings.ToLower(name))
		if err == nil && matched {
			return true
		}
	}
	return false
}

// Rules defines where different categories of repos should be placed.
type Rules struct {
	Base     string        `yaml:"base"`
	Forks    string        `yaml:"forks"`
	Archived string        `yaml:"archived"`
	Patterns []PatternRule `yaml:"patterns"`
}

// PatternRule routes repos whose name matches Pattern into the directory
// produced by expanding To. To is the full final path of the repo and may
// reference capture groups from Pattern using $1, ${1}, ${name}, or $0 for
// the whole match. Pattern rules take precedence over the fork/archived/base
// categories and are evaluated in declared order; the first match wins.
type PatternRule struct {
	Pattern string `yaml:"pattern"`
	To      string `yaml:"to"`
}

// Validate checks the pattern rules for correctness:
//   - every pattern compiles as a regex,
//   - every To template references at least one capture, so that distinct
//     repos never collapse onto a single directory,
//   - every reference in To resolves to an existing group of the pattern.
func (r *Rules) Validate() error {
	for i := range r.Patterns {
		rule := &r.Patterns[i]
		if rule.Pattern == "" {
			return fmt.Errorf("rules.patterns[%d]: pattern is empty", i)
		}
		re, err := regexp.Compile(rule.Pattern)
		if err != nil {
			return fmt.Errorf("rules.patterns[%d]: invalid regex %q: %w", i, rule.Pattern, err)
		}
		if rule.To == "" {
			return fmt.Errorf("rules.patterns[%d]: to is empty", i)
		}
		if err := validatePatternTemplate(rule.To, re); err != nil {
			return fmt.Errorf("rules.patterns[%d]: %w", i, err)
		}
	}
	return nil
}

// validatePatternTemplate checks that tmpl references at least one capture
// group of re and that every reference resolves to an existing group. The
// reference syntax mirrors regexp.ExpandString: $1, ${1}, ${name}, and $0
// (whole match) are references; $$ is an escaped dollar sign.
func validatePatternTemplate(tmpl string, re *regexp.Regexp) error {
	numSub := re.NumSubexp()
	names := re.SubexpNames() // len == numSub+1; names[0] is "" for the whole match
	refs := 0
	s := tmpl
	for {
		_, after, ok := strings.Cut(s, "$")
		if !ok {
			break
		}
		s = after
		if s != "" && s[0] == '$' {
			// $$ is an escaped dollar sign, not a reference.
			s = s[1:]
			continue
		}
		name, num, rest, extOK := extractRef(s)
		if !extOK {
			// Malformed reference: Go treats the $ as literal text, so this
			// does not count as a reference. Keep scanning the rest.
			continue
		}
		switch {
		case num >= 0:
			if num > numSub {
				return fmt.Errorf("to %q: $%s refers to a non-existent group (pattern has %d capture group(s))", tmpl, name, numSub)
			}
		default:
			found := false
			for _, n := range names {
				if n == name {
					found = true
					break
				}
			}
			if !found {
				return fmt.Errorf("to %q: $%s is not a named group in the pattern", tmpl, name)
			}
		}
		refs++
		s = rest
	}
	if refs == 0 {
		return fmt.Errorf("to %q: must reference at least one capture (e.g. $1 or $0)", tmpl)
	}
	return nil
}

// extractRef mirrors the reference parsing of regexp's internal extract: it
// reads a leading "name" or "{name}" from str (the leading $ already consumed
// by the caller) and returns the name, its numeric value (-1 for named
// references), the unconsumed rest, and whether a well-formed reference was
// found.
func extractRef(str string) (name string, num int, rest string, ok bool) {
	if str == "" {
		return "", -1, str, false
	}
	brace := false
	if str[0] == '{' {
		brace = true
		str = str[1:]
	}
	i := 0
	for i < len(str) {
		r, size := utf8.DecodeRuneInString(str[i:])
		if !unicode.IsLetter(r) && !unicode.IsDigit(r) && r != '_' {
			break
		}
		i += size
	}
	if i == 0 {
		// empty name is not okay
		return "", -1, str, false
	}
	name = str[:i]
	if brace {
		if i >= len(str) || str[i] != '}' {
			return "", -1, str, false
		}
		i++ // consume '}'
	}
	// Parse as a number, matching Go's extract: non-digits or values >= 1e8
	// and leading zeros (other than "0") are treated as named references.
	num = 0
	for k := 0; k < len(name); k++ {
		if name[k] < '0' || name[k] > '9' || num >= 1e8 {
			num = -1
			break
		}
		num = num*10 + int(name[k]-'0')
	}
	if name[0] == '0' && len(name) > 1 {
		num = -1
	}
	rest = str[i:]
	ok = true
	return name, num, rest, ok
}

// LoadFrom reads and parses a config file from the given path.
func LoadFrom(path string) (*Config, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("failed to read config file %s: %w", path, err)
	}

	var cfg Config
	if err := yaml.Unmarshal(data, &cfg); err != nil {
		return nil, fmt.Errorf("failed to parse config file %s: %w", path, err)
	}

	for name, ws := range cfg.Workspaces {
		if ws == nil {
			continue
		}
		if err := ws.Rules.Validate(); err != nil {
			return nil, fmt.Errorf("workspace %q: %w", name, err)
		}
	}

	return &cfg, nil
}

// LoadDefault loads config from the default path using os.UserConfigDir.
// The default config file is <UserConfigDir>/gitjuggling/config.yaml.
func LoadDefault() (*Config, error) {
	configDir, err := os.UserConfigDir()
	if err != nil {
		return nil, fmt.Errorf("failed to determine config directory: %w", err)
	}

	path := filepath.Join(configDir, "gitjuggling", "config.yaml")
	return LoadFrom(path)
}

// GetWorkspace resolves a workspace by name. If name is empty, it falls back
// to the DefaultWorkspace. Returns an error if no match is found.
func (c *Config) GetWorkspace(name string) (*Workspace, error) {
	if name == "" {
		name = c.DefaultWorkspace
	}

	if name == "" {
		return nil, fmt.Errorf("no workspace specified and no default_workspace configured")
	}

	ws, ok := c.Workspaces[name]
	if !ok {
		return nil, fmt.Errorf("workspace %q not found in config", name)
	}

	return ws, nil
}
