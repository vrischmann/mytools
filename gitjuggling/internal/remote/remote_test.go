package remote

import "testing"

func TestHttpsToSSH(t *testing.T) {
	tests := []struct {
		input string
		want  string
	}{
		{
			input: "https://git.example.com/user/project.git",
			want:  "git@git.example.com:user/project.git",
		},
		{
			input: "https://git.example.com/user/project",
			want:  "git@git.example.com:user/project.git",
		},
		{
			input: "https://git.example.com/org/deep/nested/repo.git",
			want:  "git@git.example.com:org/deep/nested/repo.git",
		},
		{
			input: "git@git.example.com:user/project.git",
			want:  "git@git.example.com:user/project.git", // already SSH, unchanged
		},
		{
			input: "https://git.example.com/user/project/",
			want:  "git@git.example.com:user/project.git",
		},
	}

	for _, tt := range tests {
		got := httpsToSSH(tt.input)
		if got != tt.want {
			t.Errorf("httpsToSSH(%q) = %q, want %q", tt.input, got, tt.want)
		}
	}
}
