package remote

import (
	"net/http/httptest"
	"testing"

	"github.com/stretchr/testify/require"
)

func TestReadForgejoPageIncludesMirrors(t *testing.T) {
	response := httptest.NewRecorder()
	_, err := response.WriteString(`[{"name":"mirror","mirror":true,"clone_url":"https://forgejo.example/alice/mirror.git"}]`)
	require.NoError(t, err)

	repos, err := readForgejoPage(response.Result(), "alice")
	require.NoError(t, err)
	require.Len(t, repos, 1)
	require.True(t, repos[0].IsMirror)
	require.Equal(t, "git@forgejo.example:alice/mirror.git", repos[0].CloneURL)
}

func TestForgejoTokenReturnsPlaintextToken(t *testing.T) {
	token, err := forgejoToken("  forgejo_token_here  ")

	require.NoError(t, err)
	require.Equal(t, "forgejo_token_here", token)
}
