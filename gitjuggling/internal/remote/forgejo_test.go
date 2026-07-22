package remote

import (
	"testing"

	"github.com/stretchr/testify/require"
)

func TestForgejoTokenReturnsPlaintextToken(t *testing.T) {
	token, err := forgejoToken("  forgejo_token_here  ")

	require.NoError(t, err)
	require.Equal(t, "forgejo_token_here", token)
}
