package cmd

import (
	"errors"
	"fmt"
	"os"
	"path/filepath"

	"dev.rischmann.fr/mytools/gitjuggling/internal/config"
	"github.com/spf13/cobra"
	"gopkg.in/yaml.v3"
)

var initConfigPath string

var initCmd = &cobra.Command{
	Use:   "init",
	Short: "Initialize a configuration file with placeholder values",
	RunE:  runInit,
}

func init() {
	initCmd.Flags().StringVarP(&initConfigPath, "config", "c", "", "path to config file (default: <UserConfigDir>/gitjuggling/config.yaml)")
	rootCmd.AddCommand(initCmd)
}

func runInit(cmd *cobra.Command, args []string) error {
	path := initConfigPath
	if path == "" {
		defaultPath, err := config.DefaultPath()
		if err != nil {
			return err
		}
		path = defaultPath
	}

	dir := filepath.Dir(path)
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return fmt.Errorf("failed to create config directory %s: %w", dir, err)
	}

	home, _ := os.UserHomeDir()
	placeholder := struct {
		DefaultWorkspace string `yaml:"default_workspace"`
		Workspaces       map[string]struct {
			Root         string   `yaml:"root"`
			GitHubOwners []string `yaml:"github_owners"`
			GitHubToken  string   `yaml:"github_token,omitempty"`
			ForgejoURL   string   `yaml:"forgejo_url,omitempty"`
			ForgejoUser  string   `yaml:"forgejo_user,omitempty"`
			ForgejoToken string   `yaml:"forgejo_token,omitempty"`
			Rules        struct {
				Base     string `yaml:"base"`
				Forks    string `yaml:"forks,omitempty"`
				Archived string `yaml:"archived,omitempty"`
			} `yaml:"rules"`
		} `yaml:"workspace"`
	}{
		DefaultWorkspace: "personal",
		Workspaces: map[string]struct {
			Root         string   `yaml:"root"`
			GitHubOwners []string `yaml:"github_owners"`
			GitHubToken  string   `yaml:"github_token,omitempty"`
			ForgejoURL   string   `yaml:"forgejo_url,omitempty"`
			ForgejoUser  string   `yaml:"forgejo_user,omitempty"`
			ForgejoToken string   `yaml:"forgejo_token,omitempty"`
			Rules        struct {
				Base     string `yaml:"base"`
				Forks    string `yaml:"forks,omitempty"`
				Archived string `yaml:"archived,omitempty"`
			} `yaml:"rules"`
		}{
			"personal": {
				Root:         home + "/dev",
				GitHubOwners: []string{"YOUR_GITHUB_USERNAME"},
				Rules: struct {
					Base     string `yaml:"base"`
					Forks    string `yaml:"forks,omitempty"`
					Archived string `yaml:"archived,omitempty"`
				}{
					Base:     home + "/dev/repos",
					Forks:    home + "/dev/forks",
					Archived: home + "/dev/archived",
				},
			},
		},
	}

	data, err := yaml.Marshal(&placeholder)
	if err != nil {
		return fmt.Errorf("failed to marshal config: %w", err)
	}

	file, err := os.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if errors.Is(err, os.ErrExist) {
		return fmt.Errorf("config file already exists at %s", path)
	}
	if err != nil {
		return fmt.Errorf("failed to create config file: %w", err)
	}
	_, writeErr := file.Write(data)
	if err := errors.Join(writeErr, file.Close()); err != nil {
		return fmt.Errorf("failed to write config file: %w", errors.Join(err, os.Remove(path)))
	}

	fmt.Printf("Config file created at %s\n", path)
	return nil
}
