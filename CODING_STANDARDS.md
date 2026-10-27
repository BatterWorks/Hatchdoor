# Coding standards

Read at review, against a finished diff. Everything here is a judgement call that no check can make. A rule a machine could enforce belongs in `just check`, not in this file.

## The chosen prototype is the contract

When a ticket's comments record the choice of a prototype option, that option's page is part of the spec, on a par with the ticket's own words. The comment that records the choice names the page; on the machine that built it, the page is under `.dev/prototypes/<issue>-<slug>/`.

Compare the built screen with that page at the same size and theme, control by control: its shape, its size, where it sits, and what it says. Each difference is a finding, unless a later comment on the ticket records that the maintainer agreed to it.

Where the chosen page breaks a house rule from the design system (a radius, a token, a control size), the finding is the clash itself, to be put to the maintainer. Building the house rule's version in the page's place is the fault this rule exists for: it swaps what was chosen for what was not, and nobody is asked.

A review that cannot reach the prototype folder says so in its report and leaves the point open.

## Signals have two sides

A signal is a build argument, an environment variable, a compile-time value or a file whose presence or value another part of the system reads to decide something.

When a diff adds or changes a writer of a signal (who sets it, when, or what it holds), find every reader. When it adds a reader, find every writer: the `Dockerfile`, the `justfile`, `scripts/`, the release runbook and the container-build notes as well as the source. The review names each one it found and says whether the reader's assumption still holds.

The case this rule comes from: #485 made every image build pass `GIT_SHA`, for the image's revision label. The version string already read "a commit was compiled in" as "development build", and the usage report (#477) reused that reading for its channel. A release image would have shown its version as `2.8.0 (dev abc123)` and reported `dev`, and a build from source reported `stable`. Both changes passed review; #504 fixed it.
