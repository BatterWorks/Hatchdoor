use std::collections::HashMap;
use std::path::Path;

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use super::types::NoteEntry;

pub fn unique_slug(base: &str, by_slug: &HashMap<String, NoteEntry>) -> String {
    if !by_slug.contains_key(base) {
        return base.to_string();
    }

    let mut idx = 2usize;
    loop {
        let candidate = format!("{base}-{idx}");
        if !by_slug.contains_key(&candidate) {
            return candidate;
        }
        idx += 1;
    }
}

pub fn normalize_title(input: &str) -> String {
    input.trim().to_lowercase()
}

pub fn strip_md_extension(input: &str) -> &str {
    input.strip_suffix(".md").unwrap_or(input)
}

pub fn normalize_link_target(input: &str) -> String {
    strip_md_extension(input.trim()).replace('\\', "/")
}

/// Split a note wikilink body into its target and everything after it.
///
/// The target runs to the first `|`, `#` or `^`; the suffix keeps the
/// delimiter and the rest, so a rewriter can swap the target and hand back
/// the author's own alias and anchor untouched.
///
/// See [`split_wikilink_asset_body`] for the embed form, and
/// [`normalize_link_target`] for what the returned target is measured
/// against.
pub fn split_wikilink_note_body(body: &str) -> (&str, &str) {
    split_at_target_end(body, |c| matches!(c, '|' | '#' | '^'))
}

/// Split an asset embed body into its target and everything after it.
///
/// Only `|` closes the target here: an embed's suffix is a display size
/// (`![[diagram.png|200]]`), and a `#page=3` on a PDF addresses the viewer
/// rather than naming a different file, so it stays part of the target.
pub fn split_wikilink_asset_body(body: &str) -> (&str, &str) {
    split_at_target_end(body, |c| c == '|')
}

/// The one home for how a wikilink body ends.
///
/// A backslash directly before the alias pipe is the escape Markdown needs to
/// keep the pipe from closing a table cell (`[[Note\|alias]]`), so it is part
/// of the syntax and never the last character of the target. Left on the
/// target it reaches `normalize_link_target`, which turns a backslash into a
/// path separator; the lookup then asks for a note named `Note/`, finds
/// nothing, and every reader silently treats the link as pointing at a note
/// that does not exist (#252).
fn split_at_target_end(body: &str, is_delimiter: impl Fn(char) -> bool) -> (&str, &str) {
    let end = body
        .char_indices()
        .find(|(_, c)| is_delimiter(*c))
        .map(|(idx, c)| {
            if c == '|' && body[..idx].ends_with('\\') {
                idx - 1
            } else {
                idx
            }
        })
        .unwrap_or(body.len());
    (body[..end].trim(), &body[end..])
}

/// Fold a note's name into the slug that addresses it.
///
/// European languages give a plain ASCII address and every other script keeps
/// its own letters (ADR-24). An accented Latin letter decomposes to its base
/// letter and loses its marks, so `Veá` and `Vea` are addressed alike and a
/// combining accent spells the same slug as a precomposed one (#306). The
/// European letters that decompose to nothing useful go through
/// [`fold_european_letter`]. A letter or digit from any other writing system
/// is kept as it is, lowercased, and never romanised, and so is a mark that
/// belongs to one. What gets dropped is punctuation, symbols and the marks
/// that sit on ASCII; whitespace, `-` and `_` separate words.
///
/// `frontend/src/lib/noteHeadings.ts` carries the same rule for the heading
/// anchors the browser clicks. The two have to change together, or a note's
/// address and its own headings' addresses split apart.
pub fn slugify(input: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;

    for c in input.trim().chars() {
        if let Some(folded) = fold_european_letter(c) {
            out.push_str(folded);
            prev_dash = false;
            continue;
        }

        if c.is_whitespace() || c == '-' || c == '_' {
            if !prev_dash && !out.is_empty() {
                out.push('-');
            }
            prev_dash = true;
            continue;
        }

        // A mark that arrived on its own belongs to whatever it follows. On an
        // ASCII letter it is an accent this rule exists to remove; on a letter
        // kept in its own script it is part of the word, and dropping it
        // rewrites that word — a Devanagari virama is the difference between
        // हिन्दी and हिनदी.
        if is_combining_mark(c) {
            if out.chars().last().is_some_and(|last| !last.is_ascii()) {
                out.push(c);
                prev_dash = false;
            }
            continue;
        }

        // The first character of the decomposition is the base letter; the
        // marks that follow it are what an accent is made of. A base that is
        // already ASCII is the whole answer, and anything else keeps the
        // character it arrived as, so Devanagari and Hangul are not taken
        // apart by a rule written for European accents.
        let base = c.nfd().next().unwrap_or(c);
        if base.is_ascii_alphanumeric() {
            out.push(base.to_ascii_lowercase());
            prev_dash = false;
        } else if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
            prev_dash = false;
        }
    }

    while out.ends_with('-') {
        out.pop();
    }

    out
}

/// The European letters that no decomposition reaches, each with the spelling
/// its own language already uses when it has to write ASCII (ADR-24).
///
/// Stripping marks turns `é` into `e` because `é` is `e` plus a mark, but `ß`,
/// `ø` and `æ` are letters in their own right and decompose to themselves. A
/// German keyboard-less spelling of `Straße` is `Strasse`, not `Strae`, and
/// that convention is the whole content of this table.
fn fold_european_letter(c: char) -> Option<&'static str> {
    Some(match c {
        'ß' | 'ẞ' => "ss",
        'ø' | 'Ø' => "o",
        'æ' | 'Æ' => "ae",
        'œ' | 'Œ' => "oe",
        'ł' | 'Ł' => "l",
        'đ' | 'Đ' | 'ð' | 'Ð' => "d",
        'þ' | 'Þ' => "th",
        _ => return None,
    })
}

/// Which rung of [`resolve_path_ladder`] a path target resolved on. A rewriter
/// uses it to keep the form the author wrote when it has to write a new path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkForm {
    /// A leading `/`: from the Vault root.
    Root,
    /// Relative to the linking note's own folder.
    NoteRelative,
    /// As written from the Vault root, without the leading `/`.
    VaultRelative,
    /// A bare filename, found by name anywhere in the Vault.
    ByName,
}

/// Resolve a path-shaped link target, the one order attachment embeds and
/// Markdown note links share (ADR-28).
///
/// `note_dir` is the `/`-joined Vault-relative directory of the note the
/// target was written in, `""` at the Vault root. Order, from most to least
/// explicit:
///
/// 1. A leading `/` means Vault-root-relative — the one form that is stable
///    from any note at any depth.
/// 2. Relative to the note's own folder, which is what Hatchdoor resolved
///    exclusively before (#158) and what `../` forms rely on.
/// 3. As written from the Vault root, so `98_Attachments/x.png` works from
///    a nested note without counting `../`.
/// 4. By filename anywhere in the Vault, which is Obsidian's "shortest path
///    when possible" default and the reason a single attachments folder
///    works there. `by_name` picks among namesakes, nearest first.
///
/// `exact` answers for one normalised Vault-relative path. Only a bare
/// filename reaches `by_name`: a target that names a folder is an explicit
/// path, and answering it with a namesake somewhere else would resolve to a
/// file the author did not write.
pub fn resolve_path_ladder<T>(
    target: &str,
    note_dir: &str,
    exact: impl Fn(&str) -> Option<T>,
    by_name: impl FnOnce(&str) -> Option<T>,
) -> Option<(T, LinkForm)> {
    let target = target.trim().replace('\\', "/");
    if target.is_empty() {
        return None;
    }

    if let Some(absolute) = target.strip_prefix('/') {
        return exact(&join_vault_path("", absolute)?).map(|hit| (hit, LinkForm::Root));
    }

    if let Some(hit) = join_vault_path(note_dir, &target).and_then(|candidate| exact(&candidate)) {
        return Some((hit, LinkForm::NoteRelative));
    }

    if let Some(hit) = join_vault_path("", &target).and_then(|candidate| exact(&candidate)) {
        return Some((hit, LinkForm::VaultRelative));
    }

    if target.contains('/') {
        return None;
    }

    by_name(&target).map(|hit| (hit, LinkForm::ByName))
}

/// Join `target` onto `base_dir` and resolve `.`/`..`, returning `None` when the
/// result would escape the Vault root. Escaping is rejected rather than clamped:
/// a clamped `../../secret.png` would silently resolve to a different file than
/// the author wrote.
fn join_vault_path(base_dir: &str, target: &str) -> Option<String> {
    let mut stack: Vec<&str> = base_dir
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                stack.pop()?;
            }
            other => stack.push(other),
        }
    }
    if stack.is_empty() {
        return None;
    }
    Some(stack.join("/"))
}

/// Folder hops between the note and a candidate file, so the nearest namesake
/// wins a name collision.
pub fn folder_distance(note_dir: &str, candidate: &str) -> usize {
    let note: Vec<&str> = note_dir
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    let mut file: Vec<&str> = candidate
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    file.pop();

    let shared = note
        .iter()
        .zip(file.iter())
        .take_while(|(left, right)| left == right)
        .count();
    (note.len() - shared) + (file.len() - shared)
}

/// Extensions the Vault asset route will serve. The asset index and
/// `handlers::assets` share this list so wikilink resolution can never name a
/// path the route would then refuse: a resolved embed that 404s is worse than
/// an unresolved one, because it looks like a broken file rather than a
/// broken link.
pub fn servable_asset_extensions() -> &'static [&'static str] {
    &[
        "png", "jpg", "jpeg", "gif", "webp", "svg", "avif", "bmp", "pdf",
    ]
}

pub fn is_servable_asset(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            servable_asset_extensions().contains(&extension.to_ascii_lowercase().as_str())
        })
}

pub fn relative_note_path_without_ext(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let as_string = relative.to_str()?.replace('\\', "/");
    Some(strip_md_extension(&as_string).to_string())
}

pub fn content_snippet(content: &str, normalized_query: &str) -> Option<String> {
    content
        .lines()
        .find(|line| normalize_title(line).contains(normalized_query))
        .map(|line| {
            let trimmed = line.trim();
            if trimmed.chars().count() > 180 {
                let shortened: String = trimmed.chars().take(177).collect();
                format!("{shortened}...")
            } else {
                trimmed.to_string()
            }
        })
}
