# git-journal

Summarize git commits for journal entries.

## Description

`git-journal` scans git repositories in configured directories (work and personal) and collects commits by a specific author within a date range. The output is formatted for journal entries, making it easy to track daily work and personal coding activities.

## Usage

```bash
# Show commits for today
git-journal

# Show commits for a specific date
git-journal --date 2026-03-24

# Show commits for a date range
git-journal --date "2026-03-20..2026-03-24"

# Use explicit since/until flags
git-journal --since 2026-03-20 --until 2026-03-24

# Filter by author email
git-journal --author vincent@rischmann.fr

# Plain output format
git-journal --format plain
```

## Configuration

By default, git-journal looks for repositories in:
- `~/dev/Batch` (work)
- `~/dev/perso` (personal)

These paths can be modified in the source code's `Config` implementation.

## Output Format

### Journal format (default)
Markdown-style output with sections for work and personal commits, grouped by date.

### Plain format
Simple text output with commits grouped by date and category.

## Building

```bash
cargo build --release
```

The binary will be available at `target/release/git-journal`.
