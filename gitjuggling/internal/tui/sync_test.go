package tui

import (
	"testing"

	"dev.rischmann.fr/mytools/gitjuggling/internal/execute"
	"dev.rischmann.fr/mytools/gitjuggling/internal/remote"
	"dev.rischmann.fr/mytools/gitjuggling/internal/syncplan"
)

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

func TestStartExecutionWithOnlySkippedMovesFinishesImmediately(t *testing.T) {
	m := SyncModel{}

	model, _ := m.startExecution(nil, []execute.ActionResult{{Message: "skipped (user declined)", Success: true}})
	finalModel, ok := model.(SyncModel)
	if !ok {
		t.Fatalf("expected SyncModel, got %T", model)
	}
	if finalModel.phase != syncPhaseSummary {
		t.Fatalf("expected summary phase, got %v", finalModel.phase)
	}
}
