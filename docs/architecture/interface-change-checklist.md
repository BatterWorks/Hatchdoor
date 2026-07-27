# Interface Change Checklist

Use this checklist whenever a supported contract crosses its producing module
boundary or is externally observable, even when one work packet owns both the
producer and every in-repository consumer. Internal refactors that preserve the
contract and its observable behavior do not need it.

Copy or record the applicable checklist items in the work packet, issue, pull
request, or handoff notes. Do not mark up this canonical file for an individual
change.

This checklist does not expand the work packet's authority. Declare consumer
edits as owned or coordination paths before changing them, follow the packet's
escalation rules, and obtain explicit user approval before materially broadening
the outcome or making a breaking change.

## Declare the change

- [ ] Name the producing module boundary.
- [ ] Name the contract: function or Rust type; serialized or persistent
      format; HTTP route, payload, status, error, header, auth, or origin
      behavior; MCP tool/schema; CLI argument, output, or exit behavior;
      frontend export or import path; externally observable UI behavior; CSS
      selector; event; configuration value; filesystem behavior; or ordering
      and timing behavior.
- [ ] Record the old and proposed contract shape or behavior.
- [ ] Explain why the existing contract cannot support the outcome.
- [ ] List every discovered consumer and its owning boundary, plus how
      consumers were searched for.
- [ ] State whether the change is additive, behavior-changing, or breaking.
- [ ] State compatibility expectations, including external or unknown
      consumers.
- [ ] Describe deployment or migration order and rollback/fallback behavior
      where relevant.

## Preserve architectural invariants

- [ ] Check the applicable records in `docs/adr/`; changing a binding invariant
      requires an explicitly scoped superseding ADR.
- [ ] Keep web and MCP writes routed through `vault/write/`.
- [ ] Keep Markdown authoritative and SQLite disposable.
- [ ] Keep auth/token/origin requirements at least as strict.
- [ ] Keep runtime retrieval behavior consistent with ADR-05 unless an
      explicitly scoped superseding ADR is part of the change.
- [ ] Avoid a new abstraction unless the change demonstrates its concrete
      value.

Record `N/A` with a reason for invariant checks that do not apply.

## Update consumers and evidence

- [ ] Declare each required consumer edit in the work packet before editing it;
      ask the user first if it materially broadens the outcome, risk, or
      authority.
- [ ] Update all declared in-repository consumers or document a compatible
      migration.
- [ ] Update Rust and TypeScript wire representations together where relevant.
- [ ] Add or update focused contract tests.
- [ ] Update the module map if ownership, dependencies, or integration points
      changed.
- [ ] Update user or agent documentation when observable behavior changed.

## Validate

- [ ] Run focused producer tests.
- [ ] Run focused consumer tests.
- [ ] Run the full checks for every affected backend/frontend surface.
- [ ] Record the exact validation commands, their starting directories, and
      their results.
- [ ] Run `node scripts/check-module-map.mjs` from the repository root when
      production source files or module-map classifications changed.
- [ ] Confirm the final working-tree status contains only owned paths, declared
      coordination paths, and the optional packet record, including any newly
      created files.

## Pull-request summary

Include a short block in the pull request:

```markdown
Interface change: <name>
Producer boundary: <module>
Kind: <additive | behavior-changing | breaking>
Old contract: <shape or behavior>
New contract: <shape or behavior>
Consumer boundaries: <list, including external or unknown consumers>
Compatibility/migration: <summary>
Rollout/rollback: <order and fallback, or N/A with reason>
Evidence: <exact validation commands/results and documentation>
```
