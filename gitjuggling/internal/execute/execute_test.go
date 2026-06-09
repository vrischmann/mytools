package execute

import "testing"

func TestIsNoTrackingError(t *testing.T) {
	tests := []struct {
		name string
		msg  string
		want bool
	}{
		{
			name: "git pull no tracking information",
			msg:  "git pull --rebase failed: There is no tracking information for the current branch.\nPlease specify which branch you want to rebase against.",
			want: true,
		},
		{
			name: "lowercase variant",
			msg:  "git pull --rebase failed: fatal: no tracking information for the current branch",
			want: true,
		},
		{
			name: "unrelated git error",
			msg:  "git pull --rebase failed: error: could not apply abc1234..def5678",
			want: false,
		},
		{
			name: "stash error",
			msg:  "git stash failed: error: could not stash",
			want: false,
		},
		{
			name: "empty message",
			msg:  "",
			want: false,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := IsNoTrackingError(tt.msg)
			if got != tt.want {
				t.Errorf("IsNoTrackingError(%q) = %v, want %v", tt.msg, got, tt.want)
			}
		})
	}
}
