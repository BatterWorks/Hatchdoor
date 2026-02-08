# Hatchdoor

Read-only web frontend for an Obsidian vault. The app runs in Rust, serves HTML directly, and supports Obsidian wikilinks:

- `[[Note Name]]`
- `[[Note Name|Alias]]`

## Configuration

Copy `.env.example` to `.env` and set values as needed.

Default values are:

- `VAULT_PATH=./vault`
- `HOST=0.0.0.0`
- `PORT=42824`

## Run

```bash
cargo run
```

Then open `http://localhost:42824`.

## Routes

- `/` renders the explorer with no note selected
- `/n/:slug` renders a note page
- `/assets/style.css` serves static styles
- `/health` returns `ok`

## Quality checks

```bash
cargo fmt --all --check
cargo check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```
