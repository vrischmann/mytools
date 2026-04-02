# ansible-password-agent fish integration
#
# Creates a per-shell kernel session keyring when inside a directory tree
# containing an .ansible-keyring marker file. This isolates cached passwords
# to the current shell and its children.
#
# When leaving the directory, a new session keyring is created to invalidate
# any cached passwords. When the shell exits, the kernel destroys the keyring.

set -g __ansible_keyring_active 0

function __ansible_keyring_check --on-variable PWD
    # Walk up from $PWD looking for .ansible-keyring marker file.
    set -l dir $PWD
    set -l found 0
    while test "$dir" != "/"
        if test -f "$dir/.ansible-keyring"
            set found 1
            break
        end
        set dir (dirname "$dir")
    end

    if test $found -eq 1
        if test $__ansible_keyring_active -eq 0
            keyctl new_session >/dev/null 2>&1
            set -g __ansible_keyring_active 1
        end
    else
        if test $__ansible_keyring_active -eq 1
            keyctl new_session >/dev/null 2>&1
            set -g __ansible_keyring_active 0
        end
    end
end

# Initial check in case fish starts inside a marked directory.
__ansible_keyring_check
