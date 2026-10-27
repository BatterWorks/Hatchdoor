// What `just proto-new` and `just proto-serve` share (#602): where prototypes
// live, the sizes and themes a screen is captured at, and how a tool reports a
// mistake the person running it can put right.
//
// A prototype is a static copy of the real screens with the options for a
// `needs-a-choice` ticket drawn on top. It lives under `.dev/prototypes/`,
// which Git ignores, and is thrown away once the ticket's pull request merges.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);

export const PROTOTYPES_DIR = path.join(ROOT, ".dev", "prototypes");

// `[width, height]` in CSS pixels. Phone and tablet both fall under the app's
// 920px mobile layout, which is chosen in code rather than by CSS, so a phone
// page must be captured at phone width: a desktop page squeezed to 390px shows
// a top bar that does not exist (#417). A name is part of a page's file name.
export const SIZES = {
  phone: [390, 844],
  tablet: [834, 1194],
  desktop: [1440, 900],
};

export const THEMES = ["light", "dark"];

// Something the person running the recipe can put right. Printed without a
// stack.
export class ToolError extends Error {}

export async function run(main) {
  try {
    await main();
  } catch (error) {
    if (!(error instanceof ToolError)) {
      throw error;
    }
    console.error(`error: ${error.message}`);
    process.exitCode = 1;
  }
}

// The folder a prototype's name means, refusing anything else: `proto-serve`
// answers the whole LAN, and the repository root holds `.env`.
export function folderOf(name, prototypes = PROTOTYPES_DIR) {
  const found = path.join(prototypes, name ?? "");
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]*$/.test(name ?? "") || !fs.existsSync(found)) {
    throw new ToolError(
      `there is no prototype named ${name} under .dev/prototypes/`,
    );
  }
  return found;
}
