# Work Packet Template

A complete issue, task description, or agent working plan can serve as the
packet; a committed packet file is not required. Copy this template where the
work is being tracked, delete instructional comments, and replace every
placeholder before coding. Write `None` for sections that do not apply so small
tasks stay lightweight without leaving ambiguity.

````markdown
# <Task title>

## Outcome

<Observable result. Describe behavior, not an implementation preference.>

This packet narrows the user-requested outcome. It does not authorize broader
work or opportunistic cleanup. Owned paths are writable only as necessary to
produce the outcome above.

## Boundaries

<!-- Repeat for every module involved in a cross-module or full-stack task. -->

- Module: <exact module name from docs/architecture/module-map.md>
  Kind: <exact kind from the module map>

## Owned paths

The task may change these paths only as necessary for the stated outcome. This
is the maximum permitted area, not a suggestion to edit every listed file.
Include intended tests and module-owned documentation:

- `<exact path or narrowly scoped directory>`

## Public contract

Stable contract:

- `<functions, types, serialized fields, events, routes, or behavior consumers rely on>`

Declared contract changes:

- None.

Any supported contract that crosses its producing module boundary or is
externally observable triggers
`docs/architecture/interface-change-checklist.md`, even when this packet owns
the producer and all in-repository consumers. List every affected consumer when
the declared changes are not `None`.

## Coordination paths

The task may change these shared integration, test, documentation, tooling, or
configuration files for the stated reason only:

- `<path>` — <required integration change>

## Packet record

<!-- Use None for an issue/task/working-plan packet that creates no repo file. -->

- `<committed packet path, if any>` — may be updated only to reflect approved
  scope and evidence

## Consumed dependencies

The task may use, import, read, or call but does not own:

- `<module or contract>`

## Forbidden paths and invariants

The task must not change:

- `<path or behavior>`
- `<applicable ADR invariant>`

## Acceptance criteria

- <observable behavior or structural property>
- The diff stays within owned paths, declared coordination paths, and the
  packet record.
- Existing behavior outside the stated outcome is unchanged.

## Validation

Run commands from the repository root unless a command explicitly changes
directory.

Focused:

```bash
<exact commands>
```

Full:

```bash
<gates from CONTRIBUTING.md for every affected surface>
<node scripts/check-module-map.mjs when production files changed>
```

## Escalation

If an undeclared path or interface is needed, stop expanding the diff and
classify the change:

- If it is necessary for the existing outcome and does not materially increase
  risk or authority, declare the path/change in this packet before editing it.
- If it would materially broaden the outcome, risk, or required authority, stop
  and ask the user before proceeding.

List affected consumers and complete the interface-change checklist whenever a
supported contract crosses its producing module boundary or is externally
observable.
````

## Packet review questions

Before assigning the packet to a person or agent:

- Is the outcome testable without prescribing unnecessary internals?
- Does the packet stand alone in limiting work to the user-requested outcome?
- Are all involved boundaries named exactly as they appear in the module map?
- Are writable paths exact enough to prevent opportunistic cleanup?
- Does every shared/composition edit have a stated reason?
- Are tests, documentation, tooling/configuration, and any committed packet
  record accounted for?
- Are consumed dependencies distinguished from owned implementation?
- Are safety and ADR invariants expressed as concrete behavior?
- Do focused checks state their starting directory, and are full gates included
  for every affected surface?
- Does `git status --short` show only owned paths, coordination paths, and the
  optional packet record, including any newly created files?
- Could someone identify an out-of-scope change without knowing the whole
  repository?
