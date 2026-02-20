# Changelog

## v1.1.0 - 2026-02-20

Compared with `v1.0.0`.

### Added
- Added a `Download .md` action in the note actions menu.
- Added a server download endpoint: `GET /api/note/{slug}/download`.

### Changed
- Switched markdown download flow to server-driven delivery for native mobile handoff.
- Updated frontend download trigger to use an anchor `download` flow instead of popup navigation.
- Added UTF-8-aware filename handling in `Content-Disposition` for markdown downloads.

### Fixed
- Improved iOS/Safari file handoff behavior for `.md` downloads by using attachment headers.
- Added and updated frontend/backend tests for the download path and response headers.
