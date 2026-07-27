# EmbeddingGemma licensing and default-model options

> Research record.
>
> Researched: 2026-07-26
> Scope: Using EmbeddingGemma 300M Q4 as Hatchdoor's default embedding model
> Status: Product-oriented licensing research, not formal legal advice

## Executive summary

Hatchdoor can use EmbeddingGemma as its default embedding model, including in a
commercial product. However, EmbeddingGemma is subject to Google's custom Gemma
Terms of Use and the incorporated Gemma Prohibited Use Policy. Those obligations
cannot be made to disappear.

The recommended architecture is:

1. Configure EmbeddingGemma as Hatchdoor's preferred default.
2. Do not include its weights in Hatchdoor releases or published container
   images.
3. On first run, present a short model-specific notice and record the user's
   acceptance of the applicable Gemma terms.
4. Download the model directly from its upstream host into a persistent cache on
   the user's machine.
5. Pin the upstream revision and verify the downloaded files.

This preserves local inference after the initial download while minimizing the
argument that Hatchdoor itself distributes the model weights.

## Model identification

The model evaluated in Hatchdoor is **EmbeddingGemma 300M Q4**:

- `Q4` means a 4-bit quantized model; it does not mean Gemma 4.
- Hatchdoor uses FastEmbed's `EmbeddingModel::EmbeddingGemma300MQ4`.
- FastEmbed 5.17.3 obtains the model from
  `onnx-community/embeddinggemma-300m-ONNX`.
- The relevant artifacts are:
  - `onnx/model_q4.onnx`
  - `onnx/model_q4.onnx_data`

This distinction matters because Gemma 4 uses the Apache License 2.0, while
EmbeddingGemma remains listed in the Appendix governed by the custom Gemma Terms
of Use.

The Hugging Face metadata for both the official model and ONNX conversion
identifies the license as `gemma`. The official Google repository requires
manual acknowledgement of the license. The ONNX conversion is not gated, but
that does not remove or change the license attached to the model.

## Exact licensing position

The applicable Gemma Terms of Use were last modified on April 1, 2026.

### Permitted activity

The terms permit use, reproduction, modification and distribution, provided
their conditions are followed. Google's official EmbeddingGemma documentation
expressly describes the model as licensed for responsible commercial use and
allows developers to fine-tune and deploy it in their applications.

The terms also say:

- Google claims no rights in outputs generated using Gemma.
- Outputs are not Model Derivatives.
- Users remain responsible for their outputs and subsequent uses.

For Hatchdoor, embedding vectors and vector indexes produced by the model should
therefore be treated as user output rather than as Gemma-licensed model
derivatives.

### Use restrictions

Use must comply with applicable laws and the incorporated Gemma Prohibited Use
Policy. The policy broadly prohibits using or allowing others to use Gemma for
activities including:

- Rights-infringing content or activities.
- Dangerous, illegal or malicious activities.
- Harmful, abusive or misleading uses.
- Certain privacy violations and non-consensual tracking.
- Certain automated decisions affecting people's rights.
- Sexually explicit content.
- Attempts to bypass safety measures to enable prohibited conduct.

These are use-based restrictions, which is why the license is not equivalent to
an OSI-approved open-source license such as Apache 2.0 or MIT.

### What counts as distribution

The terms define Distribution broadly. It includes transmitting, publishing or
otherwise sharing Gemma or a Model Derivative with a third party. It also
expressly includes making Gemma or its functionality available through a hosted
service, including via an API or web interface.

Consequently:

- Publishing a Docker image containing the model weights is Distribution.
- Publishing model files in a release is Distribution.
- Operating a Hatchdoor-hosted service that exposes EmbeddingGemma
  functionality is Distribution, even if users never receive the weights.
- A user's private, self-hosted Hatchdoor instance does not by itself make the
  Hatchdoor project operator a hosted-service provider.

### Redistribution obligations

A party distributing Gemma or a Model Derivative must satisfy all of the
following:

1. Include the Section 3.2 use restrictions as an enforceable provision in an
   agreement governing use or distribution of the model.
2. Notify subsequent users that the model is subject to those restrictions.
3. Give every third-party recipient a copy of the complete Gemma Terms of Use.
   A link alone is not enough.
4. Cause modified files to carry prominent notices stating that they were
   modified.
5. Accompany non-hosted distributions with a `NOTICE` text file containing:

   > Gemma is provided under and subject to the Gemma Terms of Use found at ai.google.dev/gemma/terms

Additional terms may be applied to modifications or a Model Derivative as a
whole, but they must not conflict with the Gemma Terms.

The terms do not explicitly require a click-through acceptance mechanism.
Nevertheless, the requirement for an enforceable downstream restriction makes
an installation or first-run acknowledgement safer than relying only on passive
documentation.

### Other material provisions

- The model and outputs are provided as-is, without warranties.
- Google broadly disclaims liability to the extent permitted by law.
- Google may terminate the agreement for breach.
- After termination, the recipient must stop using and distributing Gemma and
  delete copies in their possession or control.
- California law and courts in Santa Clara County govern the agreement.
- The license does not grant rights to Google's trademarks or permit implying
  endorsement.

## Hatchdoor's current distribution shape

Hatchdoor currently:

- Uses Nomic Embed Text v1.5 in production.
- Prefetches that model during the Docker build.
- Copies the populated FastEmbed cache into the final runtime image.
- Publishes the Hatchdoor application under the AGPL-3.0.

If the Docker prefetch step were changed from Nomic to EmbeddingGemma without
other architectural changes, every published Hatchdoor image would contain the
Gemma weights. Hatchdoor would then be a model redistributor and would need to
meet all the obligations above.

The AGPL application and Gemma model can be distributed as separate components
in an aggregate, but their licensing must be clearly separated:

- Hatchdoor source and binaries remain AGPL-3.0.
- EmbeddingGemma weights remain under the Gemma Terms.
- Gemma's additional restrictions must not be presented as restrictions on the
  AGPL-covered Hatchdoor code.

## Option 1: Default model with direct runtime acquisition

**Recommendation**

Make EmbeddingGemma the logical default but do not ship its weights.

Suggested flow:

1. Hatchdoor starts and detects that the default model is not installed.
2. It shows the model name, source, terms version and links to the complete
   terms and prohibited-use policy.
3. The administrator accepts the model-specific terms.
4. Hatchdoor downloads the model directly from Hugging Face to a persistent
   model-cache volume.
5. Hatchdoor records the accepted terms version, model revision and timestamp.
6. Subsequent use is fully local and offline.

The acceptance record belongs next to the model files, rather than in logs or
an environment variable. This keeps the model and the record of the terms that
govern it together when the persistent model directory is moved or restored.

Important boundaries:

- The download must go directly from the user's instance to the upstream host.
- Hatchdoor-operated infrastructure should not proxy, mirror or cache the
  weights.
- The model repository revision should be pinned.
- File hashes should be checked before the model is loaded.
- The official image must not contain a build-cache residue or model layer.

This approach offers the best balance of user convenience, offline operation
after installation and reduced maintainer exposure. It is not a guarantee that
a court would never characterize Hatchdoor as participating in distribution,
but it gives the project a much stronger separation than bundling or mirroring
the weights.

## Option 2: Bundle the model with a compliance package

Hatchdoor can retain its existing fully-offline-on-first-boot behavior and ship
the model in its container image. The image must then contain a model-specific
compliance package, for example:

```text
licenses/embeddinggemma/
├── GEMMA_TERMS
├── PROHIBITED_USE_POLICY
├── NOTICE
├── MODIFICATIONS
└── SOURCE
```

The package should:

- Contain a complete snapshot of the applicable Gemma Terms.
- Contain the required `NOTICE` sentence verbatim.
- Identify Google as the upstream model provider.
- Identify `onnx-community/embeddinggemma-300m-ONNX` as the ONNX/Q4 conversion
  source.
- State prominently that the artifacts are converted and quantized versions.
- Identify the exact upstream revision and artifact hashes.
- Explain that the model is separate from the AGPL-covered Hatchdoor
  application.

The installer or first-run experience should also obtain model-specific
acceptance and incorporate the Section 3.2 restrictions into the terms
governing the weights.

This is workable but creates continuing release-management obligations. Every
published image and alternative distribution must retain the compliance
package, and Google's terms should be checked before each release that updates
the model.

## Option 3: User-provided model or external provider

Hatchdoor can prefer EmbeddingGemma whenever an administrator supplies an
external model cache, while shipping no model and performing no automatic
download.

The administrator would:

1. Obtain the model directly from Google or Hugging Face.
2. Accept the provider's license flow.
3. Mount the resulting cache into Hatchdoor.

This provides the clearest separation for the Hatchdoor project but adds more
installation friction.

A managed EmbeddingGemma endpoint is another variation. In that arrangement,
the model provider is responsible for distributing or hosting the model, while
Hatchdoor sends text and receives embeddings. This conflicts with Hatchdoor's
private, offline design and introduces credentials, cost and data-processing
concerns. If Hatchdoor operates the endpoint itself, the Gemma hosted-service
obligations apply to Hatchdoor again.

## Recommendation for Hatchdoor

Adopt Option 1:

- EmbeddingGemma 300M Q4 is the preferred default.
- Public Hatchdoor images contain no Gemma weights.
- First run includes one lightweight, model-specific acknowledgement.
- The user's instance downloads directly from the upstream repository.
- The model is cached persistently and inference remains local afterward.
- The repository revision and artifact hashes are pinned.

If the project cannot accept even a one-time model-specific acknowledgement,
the incorporated prohibited-use policy or the possibility of future compliance
review, then EmbeddingGemma cannot be the default. In that case the default
must remain a permissively licensed alternative, with EmbeddingGemma offered as
a user-installed option.

## Primary sources

- [Gemma Terms of Use](https://ai.google.dev/gemma/terms)
- [Gemma Prohibited Use Policy](https://ai.google.dev/gemma/prohibited_use_policy)
- [EmbeddingGemma model overview](https://ai.google.dev/gemma/docs/embeddinggemma)
- [Gemma Apache License 2.0 page](https://ai.google.dev/gemma/apache_2)
- [Official EmbeddingGemma Hugging Face repository](https://huggingface.co/google/embeddinggemma-300m)
- [Official repository metadata](https://huggingface.co/api/models/google/embeddinggemma-300m)
- [ONNX/Q4 conversion repository](https://huggingface.co/onnx-community/embeddinggemma-300m-ONNX)
- [ONNX/Q4 repository metadata](https://huggingface.co/api/models/onnx-community/embeddinggemma-300m-ONNX)
- [FastEmbed-rs documentation](https://docs.rs/fastembed/latest/fastembed/enum.EmbeddingModel.html)

## Relevant Hatchdoor files

- `src/embed/fastembed_embedder.rs`
- `src/server.rs`
- `src/main.rs`
- `Dockerfile`
- `Cargo.toml`
- `Cargo.lock`
- `LICENSE`

## Approved implementation decisions

This section records the agreed product contract for implementation. It takes
precedence over the options above where they differ.

### Distribution and storage

- Hatchdoor's public images ship **neither** EmbeddingGemma nor the Nomic v1.5
  fallback weights. The Docker build must not prefetch either model or leave a
  model-cache layer behind.
- Models live only in a persistent `models/` directory. In Docker this is the
  fixed path `/models`, with the supplied Compose configuration bind-mounting
  `./models` to it. A direct local run uses `./models`. There is no model-path
  environment setting.
- Model versions use separate subdirectories, so a complete, verified version
  is never overwritten by a partial replacement.
- EmbeddingGemma is fetched directly, without a Hugging Face account or token,
  from the benchmarked Q4 ONNX source:
  `onnx-community/embeddinggemma-300m-ONNX`. Hatchdoor pins its exact revision
  and verifies every required artifact against recorded hashes before loading
  it.

### First startup and fallback

EmbeddingGemma 300M Q4 is the default model and has no manual “start setup”
button. On a fresh installation, the startup flow is:

```text
Show Gemma terms → accept → download Gemma → scan vault → build index → ready
```

Acceptance must precede the download. Downloading creates a local copy of the
model, so it is a reproduction governed by the Gemma terms—not merely a later
inference/use concern.

If the user declines, Hatchdoor deletes any partial Gemma model files and any
Gemma cache/index, then automatically downloads **Nomic Embed Text v1.5** and
indexes with it. Nomic is presented as the fallback: it has no Gemma acceptance
step, but is English-only and lower quality than multilingual Gemma.

If the user accepts Gemma but its download fails, Hatchdoor must not silently
fall back to Nomic. It stops at a clear retry error, since an automatic switch
would unexpectedly change search quality and language coverage.

There is no ongoing model-settings screen and no model-switching flow after
initial setup in this implementation. A user who declines Gemma stays on Nomic
until a later feature deliberately adds a change path.

### Terms acknowledgement and privacy copy

The single Hatchdoor user may accept the terms either in the Web UI or through
MCP. In MCP, a setup-required response presents the same notice and links; an
explicit acceptance call starts the automatic download and indexing process.

The Web UI notice is intentionally short and links to the complete Gemma Terms
and Gemma Prohibited Use Policy. It must include this substance:

> Accepting these terms only allows Hatchdoor to download and use the Gemma
> model. It does not change ownership of your vault or its contents. Hatchdoor
> does not send your notes to Google when indexing or searching. Once installed,
> the model runs locally on this machine. Downloading the model contacts its
> upstream host, but does not upload your vault contents.

The acknowledgement is written only to an `acceptance.json` file beside the
selected Gemma version. It records the terms version, terms URLs, acceptance
time, pinned model revision, artifact manifest/hash set, and Hatchdoor version.
It contains no vault data and is never sent to Hatchdoor, Google, or any other
service.

When a future Hatchdoor release knows of newer Gemma terms, it compares that
version with the receipt. A mismatch requires fresh acceptance before Gemma is
used again.

### Startup progress and failure safety

Hatchdoor already has a whole-app startup gate: the Web UI polls
`/api/startup-status` once per second and blocks vault features until the index
is ready. It already displays indexing percentage, ETA, notes, and chunks; logs
emit the same indexing progress after ten seconds and then every minute.

Gemma setup extends that existing state machine rather than creating a second
setup interface:

```text
terms required → model downloading → existing scanning → existing indexing → ready
```

The existing indexing view and logs continue to provide their current detailed
progress. The new terms and download states add only their state-specific copy
and download progress. Until startup reaches `ready`, all vault features remain
unavailable.

Before downloading, Hatchdoor checks that there is enough free space for the
model and a fresh index. Interrupted downloads resume and are hash-verified;
if verification fails, Hatchdoor deletes the bad copy and retries once. A
partially built index is never searched: it is discarded and rebuilt safely.

Future model upgrades are not automatic. If an upgrade flow is later added, it
must download and verify the new version, rebuild the index, and only then
remove the previous model and index. A changed terms version requires fresh
acceptance before that switch.
