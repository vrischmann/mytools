package cmd

import (
	"errors"
	"fmt"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"strings"

	"dev.rischmann.fr/mytools/gitjuggling/internal/gitmodules"
	"dev.rischmann.fr/mytools/gitjuggling/internal/tui"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/spf13/cobra"
)

var (
	execDepth       int
	execConcurrency int
	execVerbose     bool
)

var execCmd = &cobra.Command{
	Use:   "exec -- <git args...>",
	Short: "Run a git command in all local repositories",
	Args:  cobra.MinimumNArgs(1),
	RunE:  runExec,
}

func init() {
	execCmd.Flags().IntVarP(&execDepth, "depth", "d", 3, "search depth for repository discovery")
	execCmd.Flags().IntVarP(&execConcurrency, "concurrency", "c", 2, "concurrency limit")
	execCmd.Flags().BoolVarP(&execVerbose, "verbose", "v", false, "show output from all repositories, not just failures")

	rootCmd.AddCommand(execCmd)
}

func runExec(cmd *cobra.Command, args []string) error {
	if execConcurrency < 1 {
		return fmt.Errorf("--concurrency must be at least 1 (got %d)", execConcurrency)
	}

	// Discover repos first
	repoPaths, err := getRepositoryPaths(execDepth)
	if err != nil {
		return fmt.Errorf("discovering repos: %w", err)
	}

	if len(repoPaths) == 0 {
		fmt.Println("No repositories found.")
		return nil
	}

	// Create results channel
	resultsCh := make(chan tui.ExecResultMsg, len(repoPaths))

	// Create the Bubbletea model
	model := tui.NewExecModel(args, execVerbose, resultsCh)

	// Start the TUI program
	p := tea.NewProgram(model)

	// Run git commands in goroutines
	go func() {
		// Send discovered paths
		p.Send(tui.ExecDiscoverMsg{Paths: repoPaths})

		sem := make(chan struct{}, execConcurrency)

		for _, path := range repoPaths {
			sem <- struct{}{}

			go func(repoPath string) {
				defer func() { <-sem }()

				c := exec.Command("git", args...)
				c.Dir = repoPath
				output, err := c.CombinedOutput()

				result := tui.ExecResultMsg{
					Path: repoPath,
				}
				if err != nil {
					result.Success = false
					result.Stderr = strings.TrimSpace(string(output))
					result.Err = err
				} else {
					result.Success = true
					result.Stdout = strings.TrimSpace(string(output))
				}

				resultsCh <- result
			}(path)
		}
	}()

	// Run the TUI
	finalModel, err := p.Run()
	if err != nil {
		return fmt.Errorf("running TUI: %w", err)
	}

	// Check for failures and set exit code
	if m, ok := finalModel.(tui.ExecModel); ok && m.HasFailures() {
		os.Exit(1)
	}

	return nil
}

func getRepositoryPaths(depth int) ([]string, error) {
	var paths []string
	submodules := make(map[string]bool)

	err := filepath.WalkDir(".", func(path string, d fs.DirEntry, err error) error {
		if err != nil {
			return nil
		}

		// Limit depth
		if strings.Count(path, string(filepath.Separator)) > depth {
			if d.IsDir() {
				return fs.SkipDir
			}
			return nil
		}

		if !d.IsDir() {
			return nil
		}
		if d.Name() == ".git" {
			repoPath := filepath.Dir(path)
			if !submodules[repoPath] {
				paths = append(paths, repoPath)
			}
			return fs.SkipDir
		}
		// Read declarations before walking the children of this checkout.
		data, err := os.ReadFile(filepath.Join(path, ".gitmodules"))
		if errors.Is(err, os.ErrNotExist) {
			return nil
		}
		if _, statErr := os.Stat(filepath.Join(path, ".git")); statErr != nil {
			return nil
		}
		if err != nil {
			return err
		}
		modules, err := gitmodules.Parse(string(data))
		if err != nil {
			return fmt.Errorf("reading submodules in %s: %w", path, err)
		}
		for _, sub := range modules.Submodules {
			if filepath.IsLocal(sub.Path) && filepath.Clean(sub.Path) != "." {
				submodules[filepath.Join(path, sub.Path)] = true
			}
		}
		return nil
	})

	return paths, err
}
