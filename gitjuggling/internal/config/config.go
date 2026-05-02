package config

import (
	"fmt"
	"os"
	"path/filepath"

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
	ForgejoURL   string   `yaml:"forgejo_url"`
	ForgejoUser  string   `yaml:"forgejo_user"`
	ForgejoToken string   `yaml:"forgejo_token"`
	Rules        Rules    `yaml:"rules"`
}

// Rules defines where different categories of repos should be placed.
type Rules struct {
	Base     string `yaml:"base"`
	Forks    string `yaml:"forks"`
	Archived string `yaml:"archived"`
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
