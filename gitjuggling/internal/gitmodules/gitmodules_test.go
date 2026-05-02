package gitmodules

import "testing"

func TestParseGitModules(t *testing.T) {
	input := `
[submodule "foobar"]
    path = foo
    url = git@github.com:foo/bar.git
[submodule "cpc"]
    path = cpclol
    url = git@github.com:foo/cpc.git

[submodule "yep"]
    path = yop
    url = git@github.com:foo/yop.git
    branch = master
    foo = bar
`

	result, err := Parse(input)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if len(result.Submodules) != 3 {
		t.Fatalf("expected 3 submodules, got %d", len(result.Submodules))
	}

	// First submodule
	sub1 := result.Submodules[0]
	if sub1.Name != "foobar" {
		t.Errorf("submodule[0].Name = %q, want %q", sub1.Name, "foobar")
	}
	if sub1.Path != "foo" {
		t.Errorf("submodule[0].Path = %q, want %q", sub1.Path, "foo")
	}
	if sub1.URL != "git@github.com:foo/bar.git" {
		t.Errorf("submodule[0].URL = %q, want %q", sub1.URL, "git@github.com:foo/bar.git")
	}

	// Second submodule
	sub2 := result.Submodules[1]
	if sub2.Name != "cpc" {
		t.Errorf("submodule[1].Name = %q, want %q", sub2.Name, "cpc")
	}
	if sub2.Path != "cpclol" {
		t.Errorf("submodule[1].Path = %q, want %q", sub2.Path, "cpclol")
	}
	if sub2.URL != "git@github.com:foo/cpc.git" {
		t.Errorf("submodule[1].URL = %q, want %q", sub2.URL, "git@github.com:foo/cpc.git")
	}

	// Third submodule (with branch and extra key)
	sub3 := result.Submodules[2]
	if sub3.Name != "yep" {
		t.Errorf("submodule[2].Name = %q, want %q", sub3.Name, "yep")
	}
	if sub3.Path != "yop" {
		t.Errorf("submodule[2].Path = %q, want %q", sub3.Path, "yop")
	}
	if sub3.URL != "git@github.com:foo/yop.git" {
		t.Errorf("submodule[2].URL = %q, want %q", sub3.URL, "git@github.com:foo/yop.git")
	}
	if sub3.Branch != "master" {
		t.Errorf("submodule[2].Branch = %q, want %q", sub3.Branch, "master")
	}
}

func TestContains(t *testing.T) {
	input := `
[submodule "foo"]
    path = mypath
    url = git@github.com:foo/bar.git
`
	result, err := Parse(input)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if !result.Contains("mypath") {
		t.Error("expected Contains(mypath) to be true")
	}
	if result.Contains("otherpath") {
		t.Error("expected Contains(otherpath) to be false")
	}
}
