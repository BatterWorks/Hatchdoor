# Hatchdoor Documentation

Use this page as the entry point for project documentation. Documents are
grouped by purpose so active guidance is easy to distinguish from research and
historical records.

## Architecture and collaboration

- [`adr/`](adr/README.md) — binding architecture decisions and their history.
- [`architecture/module-map.md`](architecture/module-map.md) — current module
  ownership, contracts, dependencies, and validation.
- [`architecture/work-packet-template.md`](architecture/work-packet-template.md)
  — scope template for contributor and agent work.
- [`architecture/interface-change-checklist.md`](architecture/interface-change-checklist.md)
  — safety checklist for cross-boundary or externally observable contracts.
- [`architecture/domain-collaboration-plan.md`](architecture/domain-collaboration-plan.md)
  — implemented collaboration-boundary plan.
- [`architecture/collaboration-pilot-assessment.md`](architecture/collaboration-pilot-assessment.md)
  — evidence and limitations from the initial boundary pilots.

## Product direction

- [`roadmap/product-roadmap.md`](roadmap/product-roadmap.md) — overall product
  direction and workstreams.
- [`roadmap/vault-lifecycle.md`](roadmap/vault-lifecycle.md) — detailed vault
  lifecycle and multi-vault workstream.

## Design

- [`design/design-system.html`](design/design-system.html) — frontend visual
  tokens, component patterns, layouts, and interaction states.

## Maintenance

- [`maintenance/dependency-update-plan.md`](maintenance/dependency-update-plan.md)
  — completed dependency-update plan and retained upgrade context.
- [`maintenance/release-runbook.md`](maintenance/release-runbook.md) — release
  PR merge procedure and pre-merge checklist.

## Research

- [`research/embeddings/`](research/embeddings/) — embedding-model,
  licensing, evaluation, and FastEmbed feasibility records.

Research records inform decisions but are not binding architecture policy.
Accepted decisions belong in `adr/`.

## Historical reviews and implementation records

- [`reviews/`](reviews/) — point-in-time pull-request and implementation
  reviews.
- [`superpowers/`](superpowers/) — task-specific design, handoff, and
  implementation records retained for context.

## Runtime-coupled documentation

- [`starter-vault/`](starter-vault/) — documentation and example content
  compiled into Hatchdoor's seeded starter vault. Moving these files requires
  updating their `include_str!` or `include_bytes!` paths in `src/vault/seed.rs`.
