package gitmodules

import (
	"fmt"
	"os/exec"
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

// Parse uses Git's config syntax for quoted paths, comments and subsection names.
func Parse(input string) (*GitModules, error) {
	cmd := exec.Command("git", "config", "--no-includes", "--null", "--file", "-", "--list")
	cmd.Stdin = strings.NewReader(input)
	out, err := cmd.Output()
	if err != nil {
		return nil, fmt.Errorf("parsing .gitmodules: %w", err)
	}

	result := &GitModules{}
	indices := make(map[string]int)
	for record := range strings.SplitSeq(string(out), "\x00") {
		key, value, _ := strings.Cut(record, "\n")
		key, ok := strings.CutPrefix(key, "submodule.")
		dot := strings.LastIndexByte(key, '.')
		if !ok || dot <= 0 {
			continue
		}
		name := key[:dot]
		i, exists := indices[name]
		if !exists {
			i = len(result.Submodules)
			indices[name] = i
			result.Submodules = append(result.Submodules, GitSubmodule{Name: name})
		}
		switch key[dot+1:] {
		case "path":
			result.Submodules[i].Path = value
		case "url":
			result.Submodules[i].URL = value
		case "branch":
			result.Submodules[i].Branch = value
		}
	}
	return result, nil
}
