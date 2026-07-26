# EmbeddingGemma licensing and default-model options

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

For unattended installations, an explicit environment variable could be
supported, for example:

```text
HATCHDOOR_ACCEPT_GEMMA_TERMS=1
```

The acceptance record should still include the terms version in application
state or logs.

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
