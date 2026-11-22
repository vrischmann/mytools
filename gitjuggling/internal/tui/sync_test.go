package tui

import (
	"context"
	"path/filepath"
	"testing"

	"dev.rischmann.fr/mytools/gitjuggling/internal/config"
	"dev.rischmann.fr/mytools/gitjuggling/internal/discover"
	"dev.rischmann.fr/mytools/gitjuggling/internal/execute"
	"dev.rischmann.fr/mytools/gitjuggling/internal/git"
	"dev.rischmann.fr/mytools/gitjuggling/internal/gittest"
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

func TestSyncPrunesAgainstAllFetchedRepos(t *testing.T) {
	root := t.TempDir()
	ws := &config.Workspace{Root: root, Rules: config.Rules{Base: root}}
	github := &remote.RemoteRepo{Owner: "alice", Name: "tool", Source: remote.SourceGitHub, CloneURL: "https://github.com/alice/tool.git"}
	forgejo := &remote.RemoteRepo{Owner: "alice", Name: "tool", Source: remote.SourceForgejo, CloneURL: "git@forgejo.example:alice/tool.git"}
	mirror := &remote.RemoteRepo{Owner: "alice", Name: "mirror", Source: remote.SourceForgejo, IsMirror: true, CloneURL: "git@forgejo.example:alice/mirror.git"}
	local := discover.NewLocalRepos([]discover.LocalRepo{
		{Path: filepath.Join(root, "tool"), RemoteURLs: map[string]string{"origin": github.CloneURL}},
		{Path: filepath.Join(root, "forgejo-tool"), RemoteURLs: map[string]string{"origin": "https://forgejo.example/alice/tool.git"}},
		{Path: filepath.Join(root, "mirror"), RemoteURLs: map[string]string{"origin": mirror.CloneURL}},
		{Path: filepath.Join(root, "orphan"), RemoteURLs: map[string]string{"origin": "https://forgejo.example/alice/deleted.git"}},
	})

	m := NewSyncModel(testPlanStore(t, root), ws, true, true, true, true, 1, nil)
	defer m.cancel()
	model, _ := m.Update(syncGithubReposMsg{repos: []*remote.RemoteRepo{github}})
	model, _ = model.Update(syncForgejoReposMsg{repos: []*remote.RemoteRepo{forgejo, mirror}})
	model, _ = model.Update(syncLocalReposMsg{local: local})
	planned := model.(SyncModel)
	require.Len(t, planned.actions, 1)
	require.Same(t, github, planned.actions[0].Repo)

	model, _ = planned.Update(tea.KeyMsg{Type: tea.KeyEnter})
	pruning := model.(SyncModel)
	require.Equal(t, syncPhasePruneList, pruning.phase)
	require.Len(t, pruning.orphans, 1)
	require.Equal(t, filepath.Join(root, "orphan"), pruning.orphans[0].Path)
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

func TestResumePlanUsesSavedActions(t *testing.T) {
	repo := &remote.RemoteRepo{Owner: "vincent", Name: "demo", Source: remote.SourceGitHub}
	m := NewSyncModel(testPlanStore(t, t.TempDir()), nil, false, true, false, false, 1, &syncstate.Plan{Actions: []syncplan.Action{{
		Type:         syncplan.ActionClone,
		Repo:         repo,
		ExpectedPath: "/tmp/demo",
	}}})

	model, _ := m.handleKey(tea.KeyMsg{Type: tea.KeyEnter})
	resumed := model.(SyncModel)
	require.Equal(t, syncPhasePlan, resumed.phase)
	require.Len(t, resumed.actions, 1)
	require.Equal(t, 1, resumed.clones)
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
		},
	}

	require.Equal(t, []int{1}, m.dirtyUpdateIndices())
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
		localDirty:  map[string]bool{first: true, second: true},
		actions: []syncplan.Action{
			{Type: syncplan.ActionUpdate, Repo: repo, LocalPath: first},
			{Type: syncplan.ActionUpdate, Repo: repo, LocalPath: second},
		},
		ws:          &config.Workspace{},
		ctx:         ctx,
		cancel:      cancel,
		concurrency: 1,
	}
	m.phase = syncPhasePlan

	// Enter plan -> enters dirty confirm over both dirty updates.
	entered, _ := m.handleKey(tea.KeyMsg{Type: tea.KeyEnter})
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
				WorktreeState: git.WorktreeState{Branch: "iter-seq"},
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
