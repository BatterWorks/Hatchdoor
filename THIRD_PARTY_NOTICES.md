# Third-party notices

Hatchdoor is licensed under the GNU Affero General Public License v3.0 only
(see [LICENSE](LICENSE)). This file records third-party material that is either
**bundled in this repository** or **downloaded at runtime by the user**. The
distinction matters: only the first is redistributed by Hatchdoor.

---

## Bundled in this repository

### Material Symbols (Sharp)

Copyright Google LLC. Licensed under the Apache License, Version 2.0.

- Source: <https://github.com/google/material-design-icons>
- Licence: <https://www.apache.org/licenses/LICENSE-2.0>

Individual icons are inlined as React components in
`frontend/src/components/icons.tsx`. **Modification notice (Apache 2.0 §4b):**
the SVG path data is copied verbatim; only the surrounding markup differs (the
`<svg>` element is rewritten as a component that sizes to `1em` and paints with
`currentColor`). No icon artwork has been altered.

Apache 2.0 is compatible with AGPL-3.0 in this direction, so bundling is clean.

### Fonts

Four font families ship inside the web app, under `frontend/src/assets/fonts/`.
Each is licensed under the SIL Open Font License, Version 1.1, and none of them
declares a Reserved Font Name. The full licence text sits beside each family's
files as `OFL.txt`.

- **Bricolage Grotesque.** Copyright 2022 The Bricolage Grotesque Project
  Authors. Source: <https://github.com/ateliertriay/bricolage>
- **Newsreader.** Copyright 2020 The Newsreader Project Authors. Source:
  <https://github.com/productiontype/Newsreader>
- **Inter Tight.** Copyright 2022 The Inter Project Authors. Source:
  <https://github.com/rsms/inter-tight>
- **JetBrains Mono.** Copyright 2020 The JetBrains Mono Project Authors. Source:
  <https://github.com/JetBrains/JetBrainsMono>

The files are the `woff2` subsets Google Fonts serves for these families, one
per script, unmodified. The OFL allows bundling and redistribution with other
software provided the copyright notice and licence travel with the fonts, which
the `OFL.txt` files do. It is compatible with distribution alongside
AGPL-3.0 software.

---

## Downloaded at runtime

**These are not distributed with Hatchdoor.** No model weights ship in the
repository or the container image. The user selects a model during setup and it
is fetched from Hugging Face on their own machine, under the licence below.
Hatchdoor's obligation is to surface those terms, not to relicense them.

### Nomic Embed Text v1.5

Licensed under the Apache License, Version 2.0. The default model; downloaded
without any additional acceptance step.

### EmbeddingGemma 300M

Governed by the **Gemma Terms of Use**, which is *not* an open-source licence
and carries a prohibited-use policy. It is opt-in: Hatchdoor requires explicit
acceptance before downloading, and records a receipt of that acceptance.

- Terms: <https://ai.google.dev/gemma/terms>
- Prohibited use policy: <https://ai.google.dev/gemma/prohibited_use_policy>

Anyone deploying Hatchdoor with this model is bound by those terms. The
authoritative URLs, terms version, and source repository are the constants in
`src/model_setup.rs`; if they disagree with this file, the code wins.
