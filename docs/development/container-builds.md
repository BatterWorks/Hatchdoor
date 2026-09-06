# Building containers with a persistent Cargo cache

The root Dockerfile requires BuildKit. With a current Docker installation,
`docker build` uses it by default; clients that report a missing Buildx plugin
need their Docker distribution's client plugin installed. The default target
remains the production image. No CI service, registry, remote cache, or particular
machine size is required.

```sh
docker build --target verification .
docker build -t hatchdoor:local .
```

`verification` runs `cargo test --locked` as an unprivileged build user, including
permission-sensitive tests. It runs the default-feature suite, just like the
same Cargo command outside Docker; feature-gated model/evaluation tests are not
implicitly enabled. Production builds do not depend on this target. Callers
that require tests before publication must run it first and propagate failure.

Cargo registry downloads, Git checkouts, and `/app/target` use BuildKit cache
mounts shared across Cargo stages, separated by target platform. The cache
namespace defaults to `hatchdoor`; `--build-arg CARGO_CACHE_NAMESPACE=<name>`
selects an isolated namespace. Keep it stable across source and lockfile changes
for reuse. Test/dev and release artifacts occupy different profile directories;
the first use of each profile still compiles its dependencies.

The chef recipe and release dependency cook precede the real source COPY.
An ordinary edit therefore retains the dependency step. Manifest, lockfile, and
Cargo target changes can invalidate it. The final Cargo build uses `--locked`
and validates its own inputs even when cache contents have been removed.
The binary is copied outside the target mount during the same RUN, so the
runtime image never depends on a mounted cache at runtime.

## Optional build controls

| Build argument | Default | Effect |
| --- | --- | --- |
| `CARGO_BUILD_JOBS` | Unset (Cargo default) | Limit simultaneous Cargo compilation jobs, including chef installation. |
| `CARGO_PROFILE_RELEASE_INCREMENTAL` | `false` | Opt into compiler reuse within changed release crates. |
| `CARGO_PROFILE_RELEASE_CODEGEN_UNITS` | `16` | Preserve Cargo's normal non-incremental release value when opting into incrementality. |
| `CARGO_CACHE_NAMESPACE` | `hatchdoor` | Isolate persistent caches on a shared builder. |
| `GIT_SHA` | Empty | Existing application build provenance value. |

For a constrained builder, a caller can select one Cargo job and prebuild the
Rust stage before the full image, preventing Rust/frontend build overlap:

```sh
docker build --target verification --build-arg CARGO_BUILD_JOBS=1 .
docker build --target rust-builder --build-arg CARGO_BUILD_JOBS=1 \
  --build-arg CARGO_PROFILE_RELEASE_INCREMENTAL=true .
docker build --build-arg CARGO_BUILD_JOBS=1 \
  --build-arg CARGO_PROFILE_RELEASE_INCREMENTAL=true -t hatchdoor:local .
```

Use identical context and build arguments between the last two commands, and
retain the same builder cache. The default build leaves BuildKit free to schedule
independent stages. Cargo jobs do not bound every rustc/linker thread or the
frontend. Incremental release builds consume additional disk and can still
recompile the changed crate and relink; they do not promise a fixed speedup or
identical runtime performance. Normal release optimization defaults remain in
effect unless a caller opts in.

## Cache lifetime and verification

Cache mounts belong to the selected builder. With Docker's default driver they
live under that daemon's data root, so retaining that storage permits reuse
between client/job containers. A persistent disk does not prevent BuildKit
pruning/garbage collection. Exporting ordinary image layers is not a guarantee
that mutable cache mounts follow to another builder. Correct builds must also
work with empty caches.

Use `--progress=plain` to inspect cached build steps. An unchanged-source build
can skip an entire Cargo RUN, including a cached verification RUN. To exercise
Cargo again, make a source edit in a disposable checkout. Confirm chef's cook
stays cached and unchanged dependencies do not compile again. Also verify a
real dependency/lockfile update and an isolated empty namespace. Never substitute
an unreviewed lockfile edit for `--locked` validation.

Report wall time with cache state, source, toolchain, profile, and jobs. Docker
layer hits, Cargo fresh crates, and rustc internal incremental reuse are different
metrics. Stable Cargo output does not expose a single compiler cache-hit rate.
For a remote Docker daemon, client RSS is not build memory; sample the build
cgroup or the daemon's enclosing cgroup and state whether page cache, swap, and
other jobs are included.

References: [Docker cache mount semantics](https://docs.docker.com/reference/dockerfile/#run---mounttypecache),
[Docker default driver](https://docs.docker.com/build/builders/drivers/docker/),
[Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html#incremental).
