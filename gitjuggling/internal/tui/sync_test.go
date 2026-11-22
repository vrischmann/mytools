package tui

import (
	"context"
	"errors"
	"os"
	"path/filepath"
	"testing"

	"dev.rischmann.fr/mytools/gitjuggling/internal/config"
	"dev.rischmann.fr/mytools/gitjuggling/internal/execute"
	"dev.rischmann.fr/mytools/gitjuggling/internal/git"
	"dev.rischmann.fr/mytools/gitjuggling/internal/gittest"
	"dev.rischmann.fr/mytools/gitjuggling/internal/prune"
	"dev.rischmann.fr/mytools/gitjuggling/internal/remote"
	"dev.rischmann.fr/mytools/gitjuggling/internal/syncplan"
	"dev.rischmann.fr/mytools/gitjuggling/internal/syncstate"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/stretchr/testify/require"
)

func testPlanStore(t *testing.T, root string) syncstate.Store {
	t.Helper()
	dir := t.TempDir()
	plans, err := syncstate.NewStore(filepath.Join(dir, "plans"), "personal", filepath.Join(dir, "config.yaml"), root)
	require.NoError(t, err)
	return plans
}

// Run the loading command from a Bubble Tea batch, ignoring its spinner tick.
func loadingResult(t *testing.T, cmd tea.Cmd) tea.Msg {
	t.Helper()
	require.NotNil(t, cmd)
	batch := cmd().(tea.BatchMsg)
	for _, run := range batch {
		switch msg := run().(type) {
		case syncGithubReposMsg, syncForgejoReposMsg, syncLocalReposMsg, syncDirtyReposMsg:
			return msg
		}
	}
	t.Fatal("missing loading command")
	return nil
}

func TestSyncPrunesAgainstAllFetchedRepos(t *testing.T) {
	root := t.TempDir()
	ws := &config.Workspace{Root: root, Rules: config.Rules{Base: root}}
	github := &remote.RemoteRepo{Owner: "alice", Name: "tool", Source: remote.SourceGitHub, CloneURL: "https://github.com/alice/tool.git"}
	forgejo := &remote.RemoteRepo{Owner: "alice", Name: "tool", Source: remote.SourceForgejo, CloneURL: "git@forgejo.example:alice/tool.git"}
	mirror := &remote.RemoteRepo{Owner: "alice", Name: "mirror", Source: remote.SourceForgejo, IsMirror: true, CloneURL: "git@forgejo.example:alice/mirror.git"}
	gittest.Repo(t, filepath.Join(root, "tool"), github.CloneURL)
	gittest.Repo(t, filepath.Join(root, "forgejo-tool"), "https://forgejo.example/alice/tool.git")
	gittest.Repo(t, filepath.Join(root, "mirror"), mirror.CloneURL)
	gittest.Repo(t, filepath.Join(root, "orphan"), "https://forgejo.example/alice/deleted.git")

	saved := &syncstate.Plan{Actions: []syncplan.Action{{
		Type: syncplan.ActionUpdate, Repo: mirror, LocalPath: filepath.Join(root, "mirror"), AlreadyInPlace: true,
	}}}
	for _, tc := range []struct {
		name string
		plan *syncstate.Plan
	}{{"fresh", nil}, {"resumed", saved}} {
		t.Run(tc.name, func(t *testing.T) {
			m := NewSyncModel(testPlanStore(t, root), ws, true, true, true, true, 1, tc.plan)
			defer m.cancel()
			var model tea.Model = m
			cmd := m.Init()
			if tc.plan != nil {
				model, cmd = model.Update(tea.KeyMsg{Type: tea.KeyEnter})
			}
			githubMsg := loadingResult(t, cmd).(syncGithubReposMsg)
			githubMsg.repos = []*remote.RemoteRepo{github}
			model, cmd = model.Update(githubMsg)
			forgejoMsg := loadingResult(t, cmd).(syncForgejoReposMsg)
			forgejoMsg.repos = []*remote.RemoteRepo{forgejo, mirror}
			model, cmd = model.Update(forgejoMsg)
			model, _ = model.Update(loadingResult(t, cmd))
			planned := model.(SyncModel)
			require.Len(t, planned.actions, 1)
			if tc.plan == nil {
				require.Same(t, github, planned.actions[0].Repo)
			} else {
				require.Equal(t, tc.plan.Actions, planned.actions)
			}
			model, _ = planned.Update(tea.KeyMsg{Type: tea.KeyEnter})
			pruning := model.(SyncModel)
			require.Equal(t, syncPhasePruneList, pruning.phase)
			require.Len(t, pruning.orphans, 1)
			require.Equal(t, filepath.Join(root, "orphan"), pruning.orphans[0].Path)
		})
	}
}

func TestSyncReportsLoadAndPruneFailures(t *testing.T) {
	model, _ := (SyncModel{}).Update(syncGithubReposMsg{err: errors.New("authentication failed")})
	require.True(t, model.(SyncModel).HasFailures())

	m := SyncModel{phase: syncPhasePruneList, dryRun: true, orphans: []*prune.OrphanRepo{
		{Path: t.TempDir()}, {Path: filepath.Join(t.TempDir(), "missing")},
	}}
	model, _ = m.handleKey(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'y'}})
	pruned := model.(SyncModel)
	require.True(t, pruned.HasFailures())
	require.True(t, pruned.pruneResults[0].Success)
	pruned.pruneResults = pruned.pruneResults[:1]
	require.False(t, pruned.HasFailures())
}

func TestBuildExecutableActionsSkipsDeclinedMoves(t *testing.T) {
	repo := &remote.RemoteRepo{Owner: "vincent", Name: "demo", Source: remote.SourceGitHub}

	m := SyncModel{
		actions: []syncplan.Action{
			{Type: syncplan.ActionUpdate, Repo: repo, LocalPath: "/tmp/demo"},
			{Type: syncplan.ActionMove, Repo: repo, CurrentPath: "/tmp/old", ExpectedPath: "/tmp/new"},
			{Type: syncplan.ActionClone, Repo: repo, ExpectedPath: "/tmp/clone"},
		},
		skippedMoveIndices: map[int]bool{1: true},
	}

	actions := m.buildExecutableActions()
	if len(actions) != 2 {
		t.Fatalf("expected 2 executable actions, got %d", len(actions))
	}
	if actions[0].Type != syncplan.ActionUpdate {
		t.Fatalf("expected first action to be update, got %v", actions[0].Type)
	}
	if actions[1].Type != syncplan.ActionClone {
		t.Fatalf("expected second action to be clone, got %v", actions[1].Type)
	}

	results := m.buildSkippedMoveResults()
	if len(results) != 1 {
		t.Fatalf("expected 1 skipped result, got %d", len(results))
	}
	if results[0].Path != "/tmp/old" {
		t.Fatalf("expected skipped path /tmp/old, got %q", results[0].Path)
	}
	if results[0].Message != "skipped (user declined)" {
		t.Fatalf("unexpected skipped message %q", results[0].Message)
	}
}

func TestResumeChecksSavedUpdatesForNewChanges(t *testing.T) {
	root := t.TempDir()
	checkout := t.TempDir() // A configured destination can be outside the discovery root.
	repo := &remote.RemoteRepo{Owner: "vincent", Name: "demo", Source: remote.SourceGitHub, CloneURL: "https://github.com/vincent/demo.git"}
	gittest.Repo(t, checkout, repo.CloneURL)
	plans := testPlanStore(t, root)
	plan := syncstate.Plan{Actions: []syncplan.Action{{Type: syncplan.ActionUpdate, Repo: repo, LocalPath: checkout}}}
	require.NoError(t, plans.Save(plan))
	m := NewSyncModel(plans, &config.Workspace{Root: root}, false, true, false, false, 1, &plan)
	defer m.cancel()
	model, cmd := m.Update(tea.KeyMsg{Type: tea.KeyEnter})
	require.Nil(t, cmd, "resuming without prune does not fetch the inventories")
	resumed := model.(SyncModel)
	require.Equal(t, syncPhasePlan, resumed.phase)
	require.Equal(t, plan.Actions, resumed.actions)
	require.NoError(t, os.WriteFile(filepath.Join(checkout, "draft"), []byte("written while viewing the saved plan"), 0o600))
	model, cmd = resumed.Update(tea.KeyMsg{Type: tea.KeyEnter})
	model, _ = model.Update(loadingResult(t, cmd))
	require.Equal(t, syncPhaseDirtyConfirm, model.(SyncModel).phase)
	require.Contains(t, model.View(), checkout)
	model, _ = model.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'n'}})
	require.Equal(t, syncPhaseSummary, model.(SyncModel).phase)
	require.Equal(t, "?? draft", gittest.Run(t, checkout, "status", "--porcelain"))
	require.Empty(t, gittest.Run(t, checkout, "stash", "list"))
}

func TestResumeWithPruneKeepsSavedPlanWhenRefreshFails(t *testing.T) {
	root := t.TempDir()
	plans := testPlanStore(t, root)
	plan := syncstate.Plan{Actions: []syncplan.Action{{Type: syncplan.ActionClone, ExpectedPath: filepath.Join(root, "pending")}}}
	require.NoError(t, plans.Save(plan))
	m := NewSyncModel(plans, &config.Workspace{Root: root}, false, true, true, false, 1, &plan)
	defer m.cancel()
	model, cmd := m.Update(tea.KeyMsg{Type: tea.KeyEnter})
	require.IsType(t, syncGithubReposMsg{}, loadingResult(t, cmd))
	model, _ = model.Update(syncGithubReposMsg{err: errors.New("inventory unavailable")})
	require.True(t, model.(SyncModel).HasFailures())
	require.Equal(t, syncPhaseSummary, model.(SyncModel).phase)
	saved, err := plans.Load()
	require.NoError(t, err)
	require.Equal(t, &plan, saved)
}

func TestRebuildPreservesTheSavedPlanDuringDryRun(t *testing.T) {
	root := t.TempDir()
	plans := testPlanStore(t, root)
	plan := syncstate.Plan{Actions: []syncplan.Action{{Type: syncplan.ActionClone, ExpectedPath: filepath.Join(root, "repo")}}}
	for _, dryRun := range []bool{true, false} {
		require.NoError(t, plans.Save(plan))
		m := NewSyncModel(plans, &config.Workspace{Root: root}, dryRun, true, false, false, 1, &plan)
		model, cmd := m.handleKey(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'n'}})
		rebuilding := model.(SyncModel)
		require.Equal(t, syncPhaseLoading, rebuilding.phase)
		require.Nil(t, rebuilding.savedPlan)
		for rebuilding.phase == syncPhaseLoading {
			model, cmd = rebuilding.Update(loadingResult(t, cmd))
			rebuilding = model.(SyncModel)
		}
		require.Equal(t, syncPhasePlan, rebuilding.phase)
		model, _ = rebuilding.Update(tea.KeyMsg{Type: tea.KeyEnter})
		require.Equal(t, syncPhaseSummary, model.(SyncModel).phase)
		onDisk, err := plans.Load()
		require.NoError(t, err)
		if dryRun {
			require.Equal(t, &plan, onDisk)
		} else {
			require.Nil(t, onDisk)
		}
		m.cancel()
	}
}

func TestBuildExecutableActionsSkipsPullForReposAlreadyInPlace(t *testing.T) {
	repo := &remote.RemoteRepo{Owner: "vincent", Name: "demo", Source: remote.SourceGitHub}

	m := SyncModel{
		skipPull: true,
		actions: []syncplan.Action{
			{Type: syncplan.ActionUpdate, Repo: repo, LocalPath: "/tmp/in-place", ExpectedPath: "/tmp/in-place", AlreadyInPlace: true},
			{Type: syncplan.ActionUpdate, Repo: repo, LocalPath: "/tmp/misplaced", ExpectedPath: "/tmp/expected"},
			{Type: syncplan.ActionClone, Repo: repo, ExpectedPath: "/tmp/clone"},
		},
		skippedPulls: 1,
	}

	actions := m.buildExecutableActions()
	if len(actions) != 2 {
		t.Fatalf("expected 2 executable actions, got %d", len(actions))
	}
	if actions[0].LocalPath != "/tmp/misplaced" {
		t.Fatalf("expected misplaced repo update to remain executable, got %q", actions[0].LocalPath)
	}
	if actions[1].Type != syncplan.ActionClone {
		t.Fatalf("expected clone action to remain executable, got %v", actions[1].Type)
	}

	results := m.buildSkippedPullResults()
	if len(results) != 1 {
		t.Fatalf("expected 1 skipped pull result, got %d", len(results))
	}
	if results[0].Path != "/tmp/in-place" {
		t.Fatalf("expected skipped pull path /tmp/in-place, got %q", results[0].Path)
	}
	if results[0].Message != "skipped (pull disabled)" {
		t.Fatalf("unexpected skipped pull message %q", results[0].Message)
	}
}

func TestStartExecutionWithOnlySkippedMovesFinishesImmediately(t *testing.T) {
	m := SyncModel{plans: testPlanStore(t, t.TempDir())}

	model, _ := m.startExecution(nil, []execute.ActionResult{{Message: "skipped (user declined)", Success: true}})
	finalModel, ok := model.(SyncModel)
	if !ok {
		t.Fatalf("expected SyncModel, got %T", model)
	}
	if finalModel.phase != syncPhaseSummary {
		t.Fatalf("expected summary phase, got %v", finalModel.phase)
	}
}

func TestBuildExecutableActionsSkipsDeclinedDirtyUpdates(t *testing.T) {
	repo := &remote.RemoteRepo{Owner: "vincent", Name: "demo", Source: remote.SourceGitHub}

	m := SyncModel{
		actions: []syncplan.Action{
			{Type: syncplan.ActionUpdate, Repo: repo, LocalPath: "/tmp/clean"},
			{Type: syncplan.ActionUpdate, Repo: repo, LocalPath: "/tmp/dirty"},
			{Type: syncplan.ActionClone, Repo: repo, ExpectedPath: "/tmp/clone"},
		},
		skippedDirtyIndices: map[int]bool{1: true},
	}

	actions := m.buildExecutableActions()
	require.Len(t, actions, 2)
	require.Equal(t, "/tmp/clean", actions[0].LocalPath)
	require.Equal(t, syncplan.ActionClone, actions[1].Type)

	results := m.buildSkippedDirtyResults()
	require.Len(t, results, 1)
	require.Equal(t, "/tmp/dirty", results[0].Path)
	require.Contains(t, results[0].Message, "uncommitted changes")
}

func TestDirtyUpdateIndicesFlagsOnlyExecutableDirtyUpdates(t *testing.T) {
	repo := &remote.RemoteRepo{Owner: "vincent", Name: "demo", Source: remote.SourceGitHub}

	m := SyncModel{
		skipPull:   true,
		localDirty: map[string]bool{"/tmp/clean": false, "/tmp/dirty": true, "/tmp/skip": true},
		actions: []syncplan.Action{
			{Type: syncplan.ActionUpdate, Repo: repo, LocalPath: "/tmp/clean"},
			{Type: syncplan.ActionUpdate, Repo: repo, LocalPath: "/tmp/dirty"},
			// Already in place with --skip-pull: would not execute, must be excluded.
			{Type: syncplan.ActionUpdate, Repo: repo, LocalPath: "/tmp/skip", AlreadyInPlace: true},
			{Type: syncplan.ActionClone, Repo: repo, ExpectedPath: "/tmp/clone"},
			{Type: syncplan.ActionUpdate, Repo: repo, LocalPath: "/tmp/status-unavailable"},
		},
	}

	require.Equal(t, []int{1, 4}, m.dirtyUpdateIndices())
}

func TestDirtyConfirmKeysSkipAndProceed(t *testing.T) {
	repo := &remote.RemoteRepo{Owner: "vincent", Name: "demo", Source: remote.SourceGitHub}
	ctx, cancel := context.WithCancel(context.Background())
	defer cancel()
	root := t.TempDir()
	first, second := filepath.Join(root, "first"), filepath.Join(root, "second")
	gittest.Repo(t, second, gittest.BareRemote(t))
	gittest.Run(t, second, "push", "-u", "origin", "master")

	m := SyncModel{
		plans:       testPlanStore(t, root),
		interactive: true,
		actions: []syncplan.Action{
			{Type: syncplan.ActionUpdate, Repo: repo, LocalPath: first},
			{Type: syncplan.ActionUpdate, Repo: repo, LocalPath: second},
		},
		ws:          &config.Workspace{},
		ctx:         ctx,
		cancel:      cancel,
		concurrency: 1,
	}
	m.setPlan(m.actions)

	// Two checkouts require confirmation.
	entered, _ := m.Update(syncDirtyReposMsg{first: true, second: true})
	m = entered.(SyncModel)
	require.Equal(t, syncPhaseDirtyConfirm, m.phase)
	require.Equal(t, []int{0, 1}, m.dirtyConfirmIndices)

	// Skip the first: it drops out of the executable set; the second stays.
	skipped, _ := m.handleKey(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'n'}})
	m = skipped.(SyncModel)
	require.True(t, m.skippedDirtyIndices[0])
	require.Equal(t, []string{second}, updatePaths(m.buildExecutableActions()))

	// Confirm the second: no more dirty items, so it proceeds to execution.
	confirmed, _ := m.handleKey(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{'y'}})
	m = confirmed.(SyncModel)
	require.Equal(t, syncPhaseExecuting, m.phase)
	result := <-m.execCh
	require.True(t, result.Success, result.Message)
}

func updatePaths(actions []syncplan.Action) []string {
	var paths []string
	for _, a := range actions {
		if a.Type == syncplan.ActionUpdate {
			paths = append(paths, a.LocalPath)
		}
	}
	return paths
}

func TestBuildExecutableActionsSkipsDeclinedWorktreePrunes(t *testing.T) {
	repo := &remote.RemoteRepo{Owner: "vincent", Name: "ppst", Source: remote.SourceForgejo}

	m := SyncModel{
		actions: []syncplan.Action{
			{Type: syncplan.ActionUpdate, Repo: repo, LocalPath: "/tmp/ppst"},
			{
				Type:          syncplan.ActionPruneWorktree,
				Repo:          repo,
				WorktreePath:  "/tmp/ppst-iter-seq",
				MainRepoPath:  "/tmp/ppst",
				WorktreeState: git.WorktreeState{BranchState: git.BranchState{Branch: "iter-seq"}},
			},
		},
		skippedWorktreePrunes: map[int]bool{1: true},
	}

	actions := m.buildExecutableActions()
	if len(actions) != 1 {
		t.Fatalf("expected 1 executable action, got %d", len(actions))
	}
	if actions[0].Type != syncplan.ActionUpdate {
		t.Fatalf("expected update action to remain, got %v", actions[0].Type)
	}

	results := m.buildSkippedWorktreeResults()
	if len(results) != 1 {
		t.Fatalf("expected 1 skipped result, got %d", len(results))
	}
	if results[0].Path != "/tmp/ppst-iter-seq" {
		t.Fatalf("expected skipped path /tmp/ppst-iter-seq, got %q", results[0].Path)
	}
	if results[0].Message != "skipped (user declined)" {
		t.Fatalf("unexpected skipped message %q", results[0].Message)
	}
}

func TestCompletedMoveSavesRelocatedWorktreePaths(t *testing.T) {
	root := t.TempDir()
	oldMain, newMain := filepath.Join(root, "demo"), filepath.Join(root, "repos", "demo")
	sibling := filepath.Join(root, "demo-topic")
	m := SyncModel{plans: testPlanStore(t, root), actions: []syncplan.Action{
		{Type: syncplan.ActionMove, CurrentPath: oldMain, ExpectedPath: newMain},
		{Type: syncplan.ActionPruneWorktree, MainRepoPath: oldMain, WorktreePath: filepath.Join(oldMain, "nested")},
		{Type: syncplan.ActionPruneWorktree, MainRepoPath: oldMain, WorktreePath: sibling},
	}}
	require.NoError(t, m.completeAction(execute.ActionResult{Path: newMain, MovedFrom: oldMain, Success: true}))
	plan, err := m.plans.Load()
	require.NoError(t, err)
	require.NotNil(t, plan)
	require.Len(t, plan.Actions, 2)
	require.Equal(t, newMain, plan.Actions[0].MainRepoPath)
	require.Equal(t, filepath.Join(newMain, "nested"), plan.Actions[0].WorktreePath)
	require.Equal(t, newMain, plan.Actions[1].MainRepoPath)
	require.Equal(t, sibling, plan.Actions[1].WorktreePath)
	require.NoError(t, m.completeAction(execute.ActionResult{Path: plan.Actions[0].WorktreePath, Success: true}))
	require.NoError(t, m.completeAction(execute.ActionResult{Path: sibling, Success: true}))
	plan, err = m.plans.Load()
	require.NoError(t, err)
	require.Nil(t, plan)
}

func TestTrackingRepairRetiresOnlySuccessfulUpdates(t *testing.T) {
	for _, decision := range []string{"repair", "fail", "decline"} {
		t.Run(decision, func(t *testing.T) {
			root := t.TempDir()
			path := filepath.Join(root, "repo")
			origin := gittest.BareRemote(t)
			gittest.Repo(t, path, origin)
			gittest.Run(t, path, "push", "origin", "master")
			repo := &remote.RemoteRepo{Name: "repo", CloneURL: origin}
			actions := []syncplan.Action{
				{Type: syncplan.ActionUpdate, Repo: repo, LocalPath: path},
				{Type: syncplan.ActionClone, Repo: repo, ExpectedPath: filepath.Join(root, "other")},
			}
			plans := testPlanStore(t, root)
			require.NoError(t, plans.Save(syncstate.Plan{Actions: actions}))
			m := NewSyncModel(plans, &config.Workspace{Root: root}, false, true, false, false, 1, nil)
			defer m.cancel()
			m.setPlan(actions)
			m.execResults = []execute.ActionResult{{Path: path, Message: "no tracking information"}}
			model, _ := m.finishExecution()
			require.Equal(t, syncPhaseTrackingFix, model.(SyncModel).phase)
			if decision == "fail" {
				require.NoError(t, os.Rename(origin, filepath.Join(root, "offline")))
			}
			key := 'y'
			if decision == "decline" {
				key = 'n'
			}
			model, _ = model.Update(tea.KeyMsg{Type: tea.KeyRunes, Runes: []rune{key}})
			require.Equal(t, decision != "repair", model.(SyncModel).HasFailures())
			pending, err := plans.Load()
			require.NoError(t, err)
			require.NotNil(t, pending)
			if decision == "repair" {
				require.Len(t, pending.Actions, 1)
				require.Equal(t, syncplan.ActionClone, pending.Actions[0].Type)
			} else {
				require.Equal(t, actions, pending.Actions)
			}
		})
	}
}

func TestSyncPersistsAndRetriesOnlyBranchCleanup(t *testing.T) {
	root := t.TempDir()
	mainPath, worktree := filepath.Join(root, "repo"), filepath.Join(root, "checkout")
	origin := gittest.BareRemote(t)
	gittest.Repo(t, mainPath, origin)
	gittest.Run(t, mainPath, "push", "-u", "origin", "master")
	gittest.Worktree(t, mainPath, worktree, "topic", false)
	gittest.Run(t, worktree, "push", "-u", "origin", "HEAD:refs/heads/published")
	gittest.Run(t, origin, "update-ref", "-d", "refs/heads/published")
	gittest.Run(t, mainPath, "fetch", "--prune", "origin")
	state, err := git.ReadWorktreeState(mainPath, worktree)
	require.NoError(t, err)
	plan := &syncstate.Plan{Actions: []syncplan.Action{{
		Type: syncplan.ActionPruneWorktree, Repo: &remote.RemoteRepo{Name: "repo", CloneURL: origin},
		MainRepoPath: mainPath, WorktreePath: worktree, WorktreeState: state, Merged: true,
	}}}
	plans := testPlanStore(t, root)
	require.NoError(t, plans.Save(*plan))
	start := func(saved *syncstate.Plan) (tea.Model, tea.Cmd) {
		m := NewSyncModel(plans, &config.Workspace{Root: root}, false, false, false, false, 1, saved)
		t.Cleanup(m.cancel)
		model, _ := m.Update(tea.KeyMsg{Type: tea.KeyEnter})
		return model.Update(tea.KeyMsg{Type: tea.KeyEnter})
	}
	model, cmd := start(plan)
	removed := cmd().(syncActionResultMsg)
	require.True(t, removed.result.Success, removed.result.Message)
	require.NotNil(t, removed.result.BranchCleanup)
	require.NoDirExists(t, worktree)
	// A replacement checkout makes Git refuse branch deletion.
	gittest.Run(t, mainPath, "worktree", "add", worktree, "topic")
	model, cmd = model.Update(removed)
	saved, err := plans.Load()
	require.NoError(t, err)
	require.NotNil(t, saved)
	require.Len(t, saved.Actions, 1)
	require.True(t, saved.Actions[0].BranchCleanupPending)
	model, _ = model.Update(cmd())
	require.True(t, model.(SyncModel).HasFailures())
	saved, err = plans.Load()
	require.NoError(t, err)
	require.NotNil(t, saved)
	gittest.Run(t, worktree, "switch", "--detach")
	model, cmd = start(saved)
	model, _ = model.Update(cmd())
	require.False(t, model.(SyncModel).HasFailures())
	require.Equal(t, state.Commit, gittest.Run(t, worktree, "rev-parse", "HEAD"))
	require.Empty(t, gittest.Run(t, mainPath, "branch", "--list", "topic"))
	saved, err = plans.Load()
	require.NoError(t, err)
	require.Nil(t, saved)
}
