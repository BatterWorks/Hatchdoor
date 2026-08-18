---
tags: [type/reference, topic/vault-organization]
---

# The Zettelkasten method (external reference)

Zettelkasten ("slip-box") is Niklas Luhmann's note-taking method, popularized for a modern audience by Sönke Ahrens. This page is a dictionary of the method itself, for anyone deciding how to lay out a new Vault, with links to the primary sources.

## Primary sources

| Source | Link |
| --- | --- |
| *How to Take Smart Notes* (book) — Sönke Ahrens | [takesmartnotes.com](https://takesmartnotes.com/) |
| Niklas Luhmann's Zettelkasten (digitized archive) — Bielefeld University | [niklas-luhmann-archiv.de](https://niklas-luhmann-archiv.de/) |
| "Zettelkasten Method" overview — zettelkasten.de | [zettelkasten.de/introduction](https://zettelkasten.de/introduction/) |

## The core idea

Luhmann kept roughly 90,000 index cards over his career, cross-referenced by hand, and credited the slip-box itself — not just his own thinking — with the volume of work he produced. The method treats writing notes as thinking, not transcription: a note is only useful once it's rewritten in your own words, atomic, and linked to what it relates to. The slip-box's value compounds because notes accumulate connections over time, not because it stores more.

## Three kinds of notes

Ahrens distinguishes notes by role, not by where they're filed:

| Kind | Role |
| --- | --- |
| **Fleeting notes** | A quick capture of a thought as it occurs — disposable, meant to be processed within a day or two, not kept long-term. |
| **Literature notes** | What a source actually said, in your own words, tied to the source it came from. |
| **Permanent notes** | One idea, fully written out, in a form that makes sense without its original context — this is what actually goes into the slip-box and gets linked. |

## What makes a note "atomic"

A permanent note holds exactly one idea, written so it stands on its own months or years later without needing the surrounding material that produced it. This is the property that makes dense linking possible: a note that bundles several ideas can only be linked as a whole, while an atomic note can be linked precisely, from every other note that actually relates to that one idea.

## Links over hierarchy

Luhmann's slip-box had almost no folder structure — cards were filed near a related card and connected by explicit reference numbers, so a note's context came from its links, not its location. This is the method's sharpest contrast with [[The PARA method (external reference)|PARA]]: PARA sorts by where a note belongs (which folder, based on actionability); Zettelkasten deliberately avoids that question and lets structure emerge from links accumulated over time.

## How this shows up in a Hatchdoor Vault

A Vault's `[[wikilinks]]` are the direct mechanical equivalent of Luhmann's card references — see [[Supported Markdown reference]] for the link syntax and [[How indexing and search work]] for how they're indexed and surfaced as backlinks. A note's slug plays the role Luhmann's ID numbers played: a stable handle other notes can point at regardless of where the file lives or gets moved. Nothing about the method requires folders at all, so a Zettelkasten-style Vault can be as flat as a single directory of permanent notes — though nothing stops combining it with [[The PARA method (external reference)|PARA]] folders or [[The layer system|layers]] for fleeting and literature notes, keeping only permanent notes on the default surface.

---

Related: [[The PARA method (external reference)]] · [[The Second Brain method (external reference)]] · [[Supported Markdown reference]] · [[The layer system]]
