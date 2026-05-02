package gitmodules

import (
	"fmt"
	"strings"
)

// GitSubmodule represents a single submodule entry in a .gitmodules file.
type GitSubmodule struct {
	Name   string
	Path   string
	URL    string
	Branch string
}

// GitModules holds all parsed submodules from a .gitmodules file.
type GitModules struct {
	Submodules []GitSubmodule
}

// Contains checks if any submodule has the given path.
func (gm *GitModules) Contains(path string) bool {
	for _, sub := range gm.Submodules {
		if sub.Path == path {
			return true
		}
	}
	return false
}

// Parse parses the content of a .gitmodules file.
func Parse(input string) (*GitModules, error) {
	p := &parser{input: input}
	result, err := p.parse()
	if err != nil {
		return nil, err
	}
	return result, nil
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

type parser struct {
	input string
}

func (p *parser) parse() (*GitModules, error) {
	result := &GitModules{}

	for len(p.input) > 0 {
		p.eatWhitespace()

		if len(p.input) == 0 {
			break
		}

		submodule, err := p.parseSubmodule()
		if err != nil {
			if err.Error() == "eof" {
				break
			}
			return nil, err
		}

		result.Submodules = append(result.Submodules, submodule)
	}

	return result, nil
}

func (p *parser) parseSubmodule() (GitSubmodule, error) {
	p.eatWhitespace()

	// Parse [submodule "<foo>"]
	if !strings.HasPrefix(p.input, "[submodule ") {
		return GitSubmodule{}, fmt.Errorf("expected [submodule ...], got %q", p.peek(20))
	}
	p.input = p.input[len("[submodule "):]

	name, err := p.parseQuotedString()
	if err != nil {
		return GitSubmodule{}, err
	}

	// Parse key=value pairs until next section or EOF
	kvs := make(map[string]string)

	for {
		p.eatWhitespace()

		if len(p.input) == 0 || p.input[0] == '[' {
			break
		}

		key, err := p.parseUntil('=')
		if err != nil {
			break
		}
		value := p.parseUntilEOL()

		kvs[key] = value
	}

	submodule := GitSubmodule{
		Name:   name,
		Path:   kvs["path"],
		URL:    kvs["url"],
		Branch: kvs["branch"],
	}

	return submodule, nil
}

func (p *parser) parseQuotedString() (string, error) {
	if len(p.input) == 0 || p.input[0] != '"' {
		return "", fmt.Errorf("expected opening quote, got %q", p.peek(5))
	}
	p.input = p.input[1:]

	// Find the closing "]
	idx := strings.Index(p.input, "\"]")
	if idx < 0 {
		return "", fmt.Errorf("expected closing \"]")
	}

	result := p.input[:idx]
	p.input = p.input[idx+2:]

	return result, nil
}

func (p *parser) parseUntil(ch byte) (string, error) {
	for i := 0; i < len(p.input); i++ {
		if p.input[i] == ch {
			result := strings.TrimSpace(p.input[:i])
			p.input = p.input[i+1:]
			return result, nil
		}
	}
	return "", fmt.Errorf("eof")
}

func (p *parser) parseUntilEOL() string {
	for i := 0; i < len(p.input); i++ {
		if p.input[i] == '\n' {
			result := strings.TrimSpace(p.input[:i])
			p.input = p.input[i+1:]
			return result
		}
	}
	// No newline found — rest of input is the value
	result := strings.TrimSpace(p.input)
	p.input = ""
	return result
}

func (p *parser) eatWhitespace() {
	for len(p.input) > 0 {
		switch p.input[0] {
		case ' ', '\t', '\n', '\r':
			p.input = p.input[1:]
		default:
			return
		}
	}
}

func (p *parser) peek(n int) string {
	if len(p.input) < n {
		n = len(p.input)
	}
	return p.input[:n]
}
