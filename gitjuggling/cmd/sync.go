package cmd

import (
	"fmt"
	"os"

	"dev.rischmann.fr/mytools/gitjuggling/internal/tui"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/spf13/cobra"
)

var (
	syncConfigPath  string
	syncDryRun      bool
	syncInteractive bool
	syncPrune       bool
	syncConcurrency int
)

var syncCmd = &cobra.Command{
	Use:   "sync [workspace]",
	Short: "Sync local repos with upstream GitHub/Forgejo remotes",
	Args:  cobra.MaximumNArgs(1),
	RunE:  runSync,
}

func init() {
	syncCmd.Flags().StringVar(&syncConfigPath, "config", "", "path to config file")
	syncCmd.Flags().BoolVar(&syncDryRun, "dry-run", false, "show what would be done without making changes")
	syncCmd.Flags().BoolVar(&syncInteractive, "interactive", true, "prompt before destructive actions")
	syncCmd.Flags().BoolVar(&syncPrune, "prune", false, "prune local repos that have no upstream match")
	syncCmd.Flags().IntVarP(&syncConcurrency, "concurrency", "c", 2, "concurrency limit for parallel operations")

	rootCmd.AddCommand(syncCmd)
}

func runSync(cmd *cobra.Command, args []string) error {
	workspaceName := ""
	if len(args) > 0 {
		workspaceName = args[0]
	}

	model := tui.NewSyncModel(workspaceName, syncConfigPath, syncDryRun, syncInteractive, syncPrune, syncConcurrency)

	p := tea.NewProgram(model)
	finalModel, err := p.Run()
	if err != nil {
		return fmt.Errorf("running TUI: %w", err)
	}

	if m, ok := finalModel.(tui.SyncModel); ok && m.HasFailures() {
		os.Exit(1)
	}

	return nil
}
