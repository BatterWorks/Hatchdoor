#!/usr/bin/env bash
#
# Provision disposable dev-server Vault fixtures under .dev/vaults and write a
# matching Vault registry at .dev/state/vaults.json.
#
# Everything here is generated rather than committed: the pathological fixture
# deliberately contains filenames git and rsync handle badly (trailing spaces,
# NFC/NFD near-duplicates, symlinks), so keeping it out of the tree is what
# lets the repo stay clonable on any platform.
#
# Usage: scripts/dev-vaults.sh <clean|messy|broken>

set -euo pipefail

profile="${1:-clean}"
repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
vaults_dir="$repo_root/.dev/vaults"
state_dir="$repo_root/.dev/state"
registry="$state_dir/vaults.json"

# A read-only fixture is left at 0555, which defeats a plain rm -rf. Always
# restore write permission before removing anything.
reset_tree() {
    if [ -d "$vaults_dir" ]; then
        chmod -R u+w "$vaults_dir" 2>/dev/null || true
        rm -rf "$vaults_dir"
    fi
    mkdir -p "$vaults_dir" "$state_dir"
}

note() {
    local path="$1"
    mkdir -p "$(dirname "$path")"
    cat > "$path"
}

# ---------------------------------------------------------------------------
# healthy: the control. Prefer the local demo vault (richer, ~50 notes) and
# fall back to the committed starter vault so a fresh clone still works.
# ---------------------------------------------------------------------------
build_healthy() {
    local dest="$vaults_dir/healthy"
    mkdir -p "$dest"
    if [ -d "$repo_root/demo-vault" ]; then
        cp -a "$repo_root/demo-vault/." "$dest/"
    else
        cp -a "$repo_root/docs/starter-vault/." "$dest/"
    fi
}

# ---------------------------------------------------------------------------
# readonly: healthy content with the write bit removed, to exercise
# LocalContentStatus::ReadOnly and the disabled-mutation surfaces.
# ---------------------------------------------------------------------------
build_readonly() {
    local dest="$vaults_dir/readonly"
    mkdir -p "$dest"
    cp -a "$repo_root/docs/starter-vault/." "$dest/"
    chmod -R a-w "$dest"
}

# ---------------------------------------------------------------------------
# empty: a registered Vault with no notes at all.
# ---------------------------------------------------------------------------
build_empty() {
    mkdir -p "$vaults_dir/empty"
}

# ---------------------------------------------------------------------------
# disabled: registered with enabled=false. Needs its own directory because the
# registry rejects two definitions whose canonical paths overlap.
# ---------------------------------------------------------------------------
build_disabled() {
    local dest="$vaults_dir/disabled"
    mkdir -p "$dest"
    cp -a "$repo_root/docs/starter-vault/." "$dest/"
}

# ---------------------------------------------------------------------------
# pathological: the point of this whole exercise.
# ---------------------------------------------------------------------------
build_pathological() {
    local dest="$vaults_dir/pathological"
    mkdir -p "$dest"

    build_scripts "$dest"
    build_filename_edge_cases "$dest"
    build_encoding_edge_cases "$dest"
    build_frontmatter_edge_cases "$dest"
    build_link_edge_cases "$dest"
    build_structure_edge_cases "$dest"
}

# Non-Latin scripts across writing systems: each note is a real note with real
# links, so search, the graph, and the explorer all have to render them.
build_scripts() {
    local dest="$1/scripts"

    note "$dest/日本語のノート.md" <<'EOF'
---
title: 日本語のノート
tags: [多言語, テスト]
---

# 日本語のノート

ひらがな、カタカナ、漢字が混ざった本文です。全角スペース（　）も含みます。

関連: [[中文笔记]] / [[한국어 노트]]
EOF

    note "$dest/中文笔记.md" <<'EOF'
---
title: 中文笔记
tags: [多语言]
---

# 中文笔记

简体中文正文，包含标点符号：、。！？「」《》

参见 [[日本語のノート]] 和 [[Русская заметка]]。
EOF

    note "$dest/한국어 노트.md" <<'EOF'
---
title: 한국어 노트
---

# 한국어 노트

한글 자모가 조합된 본문입니다. 파일 이름에 공백도 있습니다.

[[日本語のノート]]으로 돌아가기.
EOF

    note "$dest/Русская заметка.md" <<'EOF'
---
title: Русская заметка
tags: [кириллица]
---

# Русская заметка

Кириллица, включая ё и щ. Ссылка на [[Ελληνική σημείωση]].
EOF

    note "$dest/Ελληνική σημείωση.md" <<'EOF'
---
title: Ελληνική σημείωση
---

# Ελληνική σημείωση

Ελληνικά με τόνους: ά έ ή ί ό ύ ώ, και τελικό σίγμα ς.

Το τελικό σίγμα πέφτει σε case-folding: ΣΙΓΜΑ vs σιγμα vs ςιγμα.
EOF

    note "$dest/ملاحظة عربية.md" <<'EOF'
---
title: ملاحظة عربية
tags: [عربي, RTL]
---

# ملاحظة عربية

نص من اليمين إلى اليسار مع أرقام ١٢٣ و 123 مختلطة في نفس السطر.

رابط إلى [[הערה בעברית]] داخل فقرة عربية.
EOF

    note "$dest/הערה בעברית.md" <<'EOF'
---
title: הערה בעברית
tags: [עברית, RTL]
---

# הערה בעברית

טקסט מימין לשמאל עם ניקוד: שָׁלוֹם עוֹלָם.

קישור אל [[ملاحظة عربية]] בתוך פסקה עברית.
EOF

    note "$dest/บันทึกภาษาไทย.md" <<'EOF'
---
title: บันทึกภาษาไทย
---

# บันทึกภาษาไทย

ภาษาไทยไม่มีการเว้นวรรคระหว่างคำซึ่งทำให้การตัดคำในดัชนีค้นหายากขึ้น

สระและวรรณยุกต์ซ้อนกัน: เก้ ก้ ก๊ ก๋
EOF

    note "$dest/हिन्दी टिप्पणी.md" <<'EOF'
---
title: हिन्दी टिप्पणी
---

# हिन्दी टिप्पणी

देवनागरी में संयुक्ताक्षर: क्ष त्र ज्ञ श्र

मात्राएँ शीर्षरेखा के ऊपर और नीचे दोनों लगती हैं: कि की कु कू कृ कै कौ
EOF

    note "$dest/Ghi chú tiếng Việt.md" <<'EOF'
---
title: Ghi chú tiếng Việt
---

# Ghi chú tiếng Việt

Dấu chồng dấu: ế ề ể ễ ệ, ườ ượ, và chữ đ.

Đây là trường hợp tốt để kiểm tra chuẩn hoá Unicode khi tìm kiếm.
EOF

    note "$dest/Türkçe not — ısı İIıi.md" <<'EOF'
---
title: Türkçe not
---

# Türkçe not

Türkçe'de küçük harf i'nin büyüğü İ, ı'nın büyüğü I.

Bu, ASCII case-folding kullanan bir aramanın "ISI" ile "ısı"yı eşleştirip
eşleştirmediğini ortaya çıkarır.
EOF

    note "$dest/Straße Grüße ẞ.md" <<'EOF'
---
title: Straße
---

# Straße, Grüße, ẞ

Das große Eszett (ẞ) faltet auf "ss" — STRASSE vs Straße vs STRAẞE.

Umlaute: ä ö ü Ä Ö Ü.
EOF

    note "$dest/ქართული შენიშვნა.md" <<'EOF'
---
title: ქართული შენიშვნა
---

# ქართული შენიშვნა

ქართული ანბანი უასოებოა — არ აქვს დიდი და პატარა ასოები.
EOF

    note "$dest/Հայերեն նշում.md" <<'EOF'
---
title: Հայերեն նշում
---

# Հայերեն նշում

Հայերեն տառեր և կետադրություն՝ ։ ՞ ՛ ֊
EOF

    note "$dest/emoji 🎉 note 👩‍👩‍👧‍👦 zwj.md" <<'EOF'
---
title: emoji note
tags: [🏷️, emoji]
---

# emoji 🎉 note

The filename holds a ZWJ family sequence (👩‍👩‍👧‍👦), which is one grapheme
cluster but many code points — good for testing truncation and cursor logic.

Skin-tone modifiers: 👋🏻 👋🏼 👋🏽 👋🏾 👋🏿

Flags are regional-indicator pairs: 🇫🇷 🇯🇵 🇺🇦
EOF
}

# Names that are legal on disk but awkward for URLs, wikilinks, shells, and
# path normalisation.
build_filename_edge_cases() {
    local dest="$1/filenames"
    mkdir -p "$dest"

    note "$dest/note with #hash and [brackets].md" <<'EOF'
# note with #hash and [brackets]

The filename contains characters that mean something in both Markdown link
syntax and URLs. A naive href builder truncates this at the `#`.
EOF

    note "$dest/note with %20 literal and + plus.md" <<'EOF'
# note with %20 literal and + plus

A double-encoding bug turns the literal `%20` in this filename into a space.
EOF

    note "$dest/note?with=query&params.md" <<'EOF'
# note?with=query&params

`?` and `&` in a filename break URL construction that forgets to encode.
EOF

    note "$dest/.hidden leading dot.md" <<'EOF'
# .hidden leading dot

Dotfiles are commonly excluded by scanners. This one is a real note.
EOF

    note "$dest/trailing space .md" <<'EOF'
# trailing space

The filename ends with a space before the extension. Git checkouts on Windows
cannot represent this, which is why the fixture is generated, not committed.
EOF

    note "$dest/double..dots...everywhere.md" <<'EOF'
# double..dots...everywhere

Not a traversal attempt, just a name that looks like one to a sloppy filter.
EOF

    note "$dest/$(printf 'a%.0s' {1..180}).md" <<'EOF'
# very long filename

180 characters before the extension: within the 255-byte ext4 limit, but wide
enough to overflow most sidebar and breadcrumb layouts.
EOF

    note "$dest/UPPER lower MiXeD.md" <<'EOF'
# UPPER lower MiXeD

Sits next to `upper LOWER mixed.md` — the same name under case-insensitive
comparison. Both exist here; a case-insensitive index collapses them.
EOF

    note "$dest/upper LOWER mixed.md" <<'EOF'
# upper LOWER mixed

The case-folding twin of `UPPER lower MiXeD.md`.
EOF

    # NFC vs NFD: identical when rendered, different byte sequences on disk, so
    # both files coexist and a byte-keyed index reports two distinct notes.
    printf '# cafe (NFC)\n\nThe filename uses the precomposed e-acute (U+00E9).\nIts twin uses e + combining acute (U+0065 U+0301). They render identically.\n' \
        > "$dest/$(printf 'caf\xc3\xa9 note (NFC)').md"
    printf '# cafe (NFD)\n\nThe filename uses e + combining acute (U+0065 U+0301).\nIts twin uses the precomposed U+00E9. They render identically.\n' \
        > "$dest/$(printf 'cafe\xcc\x81 note (NFD)').md"

    # A name that is right-to-left in the middle of an otherwise LTR string.
    note "$dest/report عن الميزانية final.md" <<'EOF'
# mixed-direction filename

An Arabic run inside an ASCII filename: the visual order in a file listing is
not the byte order, which makes "which file did I click" genuinely ambiguous.
EOF
}

# Bytes that are legal-ish Markdown but hostile to a naive reader.
build_encoding_edge_cases() {
    local dest="$1/encoding"
    mkdir -p "$dest"

    printf '\xef\xbb\xbf# BOM at the start\n\nThis file begins with a UTF-8 byte order mark.\nA parser that does not strip it sees a stray character before the heading.\n' \
        > "$dest/bom-prefixed.md"

    printf '# CRLF line endings\r\n\r\nEvery line here ends with a carriage return.\r\nDiffing or hashing this against an LF copy reports a change on every line.\r\n' \
        > "$dest/crlf-line-endings.md"

    printf '# mixed line endings\r\n\nFirst line was CRLF, this one is LF.\r\nBack to CRLF.\n' \
        > "$dest/mixed-line-endings.md"

    : > "$dest/completely empty.md"

    printf '\n\n\n\n' > "$dest/only blank lines.md"

    printf '# invalid UTF-8 ahead\n\nThe next line contains a lone continuation byte: \xff\xfe and a truncated\nsequence: \xe2\x82 — a strict UTF-8 read fails here.\n' \
        > "$dest/invalid-utf8-bytes.md"

    printf '# zero-width and bidi controls\n\nZero-width space between the words: hello\xe2\x80\x8bworld\nZero-width joiner: a\xe2\x80\x8db\nRight-to-left override (a classic spoofing character): \xe2\x80\xaegnp.txt\nNon-breaking space between these:\xc2\xa0two words.\n' \
        > "$dest/invisible-characters.md"

    # A note large enough to make streaming vs buffering visible.
    {
        printf '# very large note\n\n'
        for i in $(seq 1 4000); do
            printf 'Paragraph %d. Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore magna aliqua.\n\n' "$i"
        done
    } > "$dest/very-large-note.md"

    # A .md file that is actually binary.
    head -c 65536 /dev/urandom > "$dest/actually-binary.md"

    # A very long single line with no newline at end of file.
    printf '# unterminated single line\n\n'"$(printf 'word %.0s' {1..8000})" > "$dest/no-trailing-newline.md"
}

build_frontmatter_edge_cases() {
    local dest="$1/frontmatter"
    mkdir -p "$dest"

    note "$dest/malformed yaml.md" <<'EOF'
---
title: unterminated "quote
tags: [unclosed, list
created: not-a-date
---

# malformed yaml

The frontmatter block is not valid YAML. The note body should still be
readable and searchable.
EOF

    note "$dest/duplicate keys.md" <<'EOF'
---
title: first value
title: second value
tags: [a]
tags: [b]
---

# duplicate keys

Which value wins is parser-defined. The UI should not crash either way.
EOF

    note "$dest/unclosed frontmatter.md" <<'EOF'
---
title: never closed
tags: [orphan]

# unclosed frontmatter

There is no closing delimiter, so this whole file is arguably frontmatter.
EOF

    note "$dest/frontmatter only.md" <<'EOF'
---
title: no body at all
tags: [empty-body]
---
EOF

    note "$dest/deeply nested frontmatter.md" <<'EOF'
---
title: deeply nested
meta:
  source:
    origin:
      system:
        name: legacy-importer
        version: 3
        options:
          - flatten: false
          - retain:
              - links
              - tags
aliases: ["別名", "псевдоним", "alias with spaces"]
tags: [nested, "tag with space", "tag/with/slash", "🏷️"]
---

# deeply nested frontmatter

Exercises property extraction beyond flat string values.
EOF

    note "$dest/wrong delimiter.md" <<'EOF'
+++
title = "TOML frontmatter"
tags = ["toml"]
+++

# wrong delimiter

TOML frontmatter, which this vault format does not claim to support. It should
degrade to being part of the body rather than being silently dropped.
EOF

    note "$dest/dashes in body.md" <<'EOF'
# dashes in body

This note has no frontmatter, but the body contains a horizontal rule that
looks like a delimiter:

---

Text after the rule must not be treated as a frontmatter boundary.
EOF
}

build_link_edge_cases() {
    local dest="$1/links"
    mkdir -p "$dest"

    note "$dest/broken links.md" <<'EOF'
# broken links

- [[This Note Does Not Exist]]
- [[Nor Does This One|with an alias]]
- [[folder/that/is/not/here/note]]
- [[]]
- [[   ]]
- [Markdown link to nowhere](./missing-target.md)
- ![[missing-attachment.png]]
EOF

    note "$dest/ambiguous links.md" <<'EOF'
# ambiguous links

`[[Duplicate Name]]` matches two notes in different folders. Resolution has to
pick one and the UI should say which.

- [[Duplicate Name]]
- [[a/Duplicate Name]]
- [[b/Duplicate Name]]
EOF

    note "$dest/a/Duplicate Name.md" <<'EOF'
# Duplicate Name (a)

The copy in folder `a`.
EOF

    note "$dest/b/Duplicate Name.md" <<'EOF'
# Duplicate Name (b)

The copy in folder `b`.
EOF

    note "$dest/self referential.md" <<'EOF'
# self referential

This note links to [[self referential]], itself, which a naive graph walk turns
into an infinite loop.
EOF

    note "$dest/cycle one.md" <<'EOF'
# cycle one

Goes to [[cycle two]].
EOF

    note "$dest/cycle two.md" <<'EOF'
# cycle two

Goes to [[cycle three]].
EOF

    note "$dest/cycle three.md" <<'EOF'
# cycle three

Goes back to [[cycle one]], closing the loop.
EOF

    note "$dest/links to weird names.md" <<'EOF'
# links to weird names

Wikilinks whose targets contain characters that fight the link syntax:

- [[日本語のノート]]
- [[ملاحظة عربية]]
- [[note with #hash and [brackets]]]
- [[trailing space ]]
- [[emoji 🎉 note 👩‍👩‍👧‍👦 zwj]]
- [[UPPER lower MiXeD]] and [[upper LOWER mixed]]
EOF

    note "$dest/heading and block refs.md" <<'EOF'
# heading and block refs

- [[links to weird names#links to weird names]]
- [[links to weird names#missing heading]]
- [[broken links^missing-block]]
- [[#local heading]]

## local heading

Target of the local reference.
EOF

    note "$dest/orphan.md" <<'EOF'
# orphan

Nothing links here and this note links nowhere. It should still appear in the
graph as an isolated node rather than vanishing.
EOF
}

build_structure_edge_cases() {
    local dest="$1"

    # Deep nesting.
    local deep="$dest/structure/deep"
    local path="$deep"
    for level in $(seq 1 12); do
        path="$path/level-$level"
    done
    note "$path/deeply nested note.md" <<'EOF'
# deeply nested note

Twelve directories down, to see what the breadcrumb and tree do.
EOF

    # A folder whose name is awkward in its own right.
    note "$dest/structure/folder with .md in name.md/not actually a folder.md" <<'EOF'
# folder named like a note

The parent directory ends in `.md`, so a scanner keying on the extension may
try to read the directory as a file.
EOF

    # Empty directories.
    mkdir -p "$dest/structure/empty folder" "$dest/structure/nested/empty/deeper"

    # Symlinks: one in-vault, one escaping the vault, one dangling.
    mkdir -p "$dest/structure/symlinks"
    ln -sfn "../../links/orphan.md" "$dest/structure/symlinks/link to orphan.md"
    ln -sfn "/etc/hostname" "$dest/structure/symlinks/escapes the vault.md"
    ln -sfn "./nowhere.md" "$dest/structure/symlinks/dangling.md"
    ln -sfn "../deep" "$dest/structure/symlinks/link to deep folder"

    # Attachments with awkward names.
    local media="$dest/Media"
    mkdir -p "$media"
    if [ -f "$repo_root/docs/starter-vault/40-reference/pdf-preview-sample.pdf" ]; then
        cp "$repo_root/docs/starter-vault/40-reference/pdf-preview-sample.pdf" \
            "$media/dossier — résumé (final) v2.pdf"
    fi
    if [ -d "$repo_root/demo-vault/Media" ]; then
        cp "$repo_root/demo-vault/Media/demo-dashboard.png" "$media/スクリーンショット 2026-08-16 🎉.png" 2>/dev/null || true
    fi
    printf 'not really a png\n' > "$media/lying-extension.png"
    head -c 12 /dev/urandom > "$media/no-extension"

    note "$dest/Media/attachment index.md" <<'EOF'
# attachment index

- ![[dossier — résumé (final) v2.pdf]]
- ![[スクリーンショット 2026-08-16 🎉.png]]
- ![[lying-extension.png]]
- ![[no-extension]]
- ![[Media/does-not-exist.png]]
EOF

    # A file the scanner should skip, next to one it should not.
    mkdir -p "$dest/.obsidian"
    printf '{ "note": "config the scanner should ignore" }\n' > "$dest/.obsidian/app.json"
    printf 'plain text, not markdown\n' > "$dest/notes.txt"
}

# ---------------------------------------------------------------------------
# Registry
# ---------------------------------------------------------------------------
vault_entry() {
    # $1 uuid, $2 name, $3 absolute path, $4 enabled
    cat <<EOF
    "$1": {
      "name": $2,
      "enabled": $4,
      "source": { "type": "local", "path": "$3" },
      "exclude_patterns": []
    }
EOF
}

git_vault_entry() {
    # $1 uuid, $2 name, $3 repository url, $4 enabled
    cat <<EOF
    "$1": {
      "name": $2,
      "enabled": $4,
      "source": {
        "type": "managed_git",
        "repository_url": "$3",
        "branch": "main",
        "vault_subdirectory": null,
        "mode": "pull_only",
        "poll_interval_secs": 3600
      },
      "exclude_patterns": []
    }
EOF
}

write_registry() {
    local entries=("$@")
    {
        printf '{\n  "schema_version": 1,\n  "revision": 1,\n  "vaults": {\n'
        local index=0
        for entry in "${entries[@]}"; do
            index=$((index + 1))
            printf '%s' "$entry"
            if [ "$index" -lt "${#entries[@]}" ]; then
                printf ',\n'
            else
                printf '\n'
            fi
        done
        printf '  }\n}\n'
    } > "$registry"
    chmod 600 "$registry"
}

# Fixed UUIDs so a reprovision keeps the same Vault identities — bookmarked
# URLs and cached frontend state survive a profile switch.
ID_HEALTHY="11111111-1111-4111-8111-111111111111"
ID_PATHOLOGICAL="22222222-2222-4222-8222-222222222222"
ID_READONLY="33333333-3333-4333-8333-333333333333"
ID_EMPTY="44444444-4444-4444-8444-444444444444"
ID_MISSING="55555555-5555-4555-8555-555555555555"
ID_DISABLED="66666666-6666-4666-8666-666666666666"
ID_GIT_BROKEN="77777777-7777-4777-8777-777777777777"
ID_GIT_OK="88888888-8888-4888-8888-888888888888"
ID_GIT_AUTH="99999999-9999-4999-8999-999999999999"

# Real repositories on the lab Forgejo. The three Git fixtures fail (or not) in
# genuinely different code paths: a bad hostname never resolves, a private repo
# resolves and is refused, and the public one must actually clone and index.
FORGEJO_BASE="https://forgejo.batterlan.cc/battermanz"

reset_tree

case "$profile" in
    clean)
        build_healthy
        write_registry \
            "$(vault_entry "$ID_HEALTHY" '"Healthy"' "$vaults_dir/healthy" true)"
        ;;
    messy)
        build_healthy
        build_pathological
        build_readonly
        build_empty
        build_disabled
        write_registry \
            "$(vault_entry "$ID_HEALTHY" '"Healthy"' "$vaults_dir/healthy" true)" \
            "$(vault_entry "$ID_PATHOLOGICAL" '"Pathological — 病理的 — مرضي"' "$vaults_dir/pathological" true)" \
            "$(vault_entry "$ID_READONLY" '"Read-only"' "$vaults_dir/readonly" true)" \
            "$(vault_entry "$ID_EMPTY" '"Empty"' "$vaults_dir/empty" true)" \
            "$(vault_entry "$ID_MISSING" '"Missing path"' "$vaults_dir/does-not-exist" true)" \
            "$(vault_entry "$ID_DISABLED" '"Disabled"' "$vaults_dir/disabled" false)" \
            "$(git_vault_entry "$ID_GIT_BROKEN" '"Git — unreachable remote"' "https://forgejo.invalid/hatchdoor-dev/nope.git" true)" \
            "$(git_vault_entry "$ID_GIT_OK" '"Git — clones cleanly"' "$FORGEJO_BASE/hatchdoor-dev-vault-ok.git" true)" \
            "$(git_vault_entry "$ID_GIT_AUTH" '"Git — auth refused"' "$FORGEJO_BASE/hatchdoor-dev-vault-private.git" true)"
        ;;
    broken)
        build_readonly
        build_empty
        build_disabled
        write_registry \
            "$(vault_entry "$ID_READONLY" '"Read-only"' "$vaults_dir/readonly" true)" \
            "$(vault_entry "$ID_EMPTY" '"Empty"' "$vaults_dir/empty" true)" \
            "$(vault_entry "$ID_MISSING" '"Missing path"' "$vaults_dir/does-not-exist" true)" \
            "$(vault_entry "$ID_DISABLED" '"Disabled"' "$vaults_dir/disabled" false)" \
            "$(git_vault_entry "$ID_GIT_BROKEN" '"Git — unreachable remote"' "https://forgejo.invalid/hatchdoor-dev/nope.git" true)" \
            "$(git_vault_entry "$ID_GIT_OK" '"Git — clones cleanly"' "$FORGEJO_BASE/hatchdoor-dev-vault-ok.git" true)" \
            "$(git_vault_entry "$ID_GIT_AUTH" '"Git — auth refused"' "$FORGEJO_BASE/hatchdoor-dev-vault-private.git" true)"
        ;;
    *)
        echo "unknown profile '$profile' (expected: clean, messy, broken)" >&2
        exit 1
        ;;
esac

echo "$profile" > "$repo_root/.dev/vaults-profile"
echo "provisioned '$profile' profile"
echo "  vaults:   $vaults_dir"
echo "  registry: $registry"
