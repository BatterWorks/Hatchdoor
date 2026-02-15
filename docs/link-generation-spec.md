# Hatchdoor Page URL Generation

## Base URL

Use this base URL for all note pages:

`http://192.168.31.174:42824`

## Page URL format

A note page URL is:

`http://192.168.31.174:42824/n/<slug>`

## How `<slug>` is generated

Generate `<slug>` from the markdown file name (without `.md`):

1. Lowercase.
2. Keep only ASCII letters and digits.
3. Convert spaces, `_`, and `-` to `-`.
4. Remove other symbols.
5. Collapse repeated `-`.
6. Remove trailing `-`.
7. If duplicate slug exists, append `-2`, `-3`, etc.

## Examples

- `Quotes Reinhold Niebuhr.md` -> `http://192.168.31.174:42824/n/quotes-reinhold-niebuhr`
- `My_Note!.md` -> `http://192.168.31.174:42824/n/my-note`
