# Coding standards

Read at review, against a finished diff. Everything here is a judgement call that no check can make. A rule a machine could enforce belongs in `just check`, not in this file.

## Signals have two sides

A signal is a build argument, an environment variable, a compile-time value or a file whose presence or value another part of the system reads to decide something.

When a diff adds or changes a writer of a signal (who sets it, when, or what it holds), find every reader. When it adds a reader, find every writer: the `Dockerfile`, the `justfile`, `scripts/`, the release runbook and the container-build notes as well as the source. The review names each one it found and says whether the reader's assumption still holds.

The case this rule comes from: #485 made every image build pass `GIT_SHA`, for the image's revision label. The version string already read "a commit was compiled in" as "development build", and the usage report (#477) reused that reading for its channel. A release image would have shown its version as `2.8.0 (dev abc123)` and reported `dev`, and a build from source reported `stable`. Both changes passed review; #504 fixed it.
