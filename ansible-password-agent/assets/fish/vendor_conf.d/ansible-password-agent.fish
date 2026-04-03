# ansible-password-agent fish integration
#
# Provides the `init-ansible-password-agent` function which starts a new fish
# shell inside an isolated kernel session keyring named "ansible_vault".
# This ensures cached vault/become passwords are scoped to that shell and
# its children, and are automatically destroyed when the shell exits.
#
# Usage:
#   init-ansible-password-agent
#
# This replaces the previous approach of using `keyctl new_session` inside a
# PWD event handler, which is not permitted on modern Fedora kernels.

function apa
    if not command -q keyctl
        echo "apa: keyctl not found" >&2
        return 1
    end
    exec keyctl session ansible_password_agent fish
end
