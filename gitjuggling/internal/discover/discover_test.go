package discover

import "testing"

func TestNormalizeSSHURL(t *testing.T) {
	got := NormalizeURL("git@github.com:owner/repo.git")
	want := "github.com/owner/repo"
	if got != want {
		t.Errorf("NormalizeURL(%q) = %q, want %q", "git@github.com:owner/repo.git", got, want)
	}
}

func TestNormalizeHTTPSURL(t *testing.T) {
	tests := []struct {
		input string
		want  string
	}{
		{
			input: "https://github.com/owner/repo.git",
			want:  "github.com/owner/repo",
		},
		{
			input: "https://github.com/owner/repo",
			want:  "github.com/owner/repo",
		},
	}
	for _, tt := range tests {
		got := NormalizeURL(tt.input)
		if got != tt.want {
			t.Errorf("NormalizeURL(%q) = %q, want %q", tt.input, got, tt.want)
		}
	}
}

func TestNormalizeForgejoURL(t *testing.T) {
	got := NormalizeURL("https://git.example.com/user/project.git")
	want := "git.example.com/user/project"
	if got != want {
		t.Errorf("NormalizeURL(%q) = %q, want %q", "https://git.example.com/user/project.git", got, want)
	}
}

func TestNormalizeTrailingSlash(t *testing.T) {
	got := NormalizeURL("https://github.com/owner/repo/")
	want := "github.com/owner/repo"
	if got != want {
		t.Errorf("NormalizeURL(%q) = %q, want %q", "https://github.com/owner/repo/", got, want)
	}
}

func TestNormalizeIdempotent(t *testing.T) {
	url := "github.com/owner/repo"
	got := NormalizeURL(url)
	if got != url {
		t.Errorf("NormalizeURL(%q) = %q, want %q", url, got, url)
	}
}
