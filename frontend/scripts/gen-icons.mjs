import { Resvg } from "@resvg/resvg-js";
import pngToIco from "png-to-ico";
import { writeFileSync, mkdirSync } from "fs";
import { resolve, dirname } from "path";
import { fileURLToPath } from "url";

const __dirname = dirname(fileURLToPath(import.meta.url));
const publicDir = resolve(__dirname, "../public");
const iconsDir = resolve(__dirname, "../public/icons");

mkdirSync(iconsDir, { recursive: true });

// Icon SVG with light-theme background baked in (for all raster outputs)
const ICON_SVG = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 60 60">
  <rect width="60" height="60" fill="#f4f1e8"/>
  <rect x="4" y="4" width="9" height="52" fill="#0c0c0a"/>
  <rect x="4" y="4" width="16" height="9" fill="#0c0c0a"/>
  <rect x="4" y="47" width="16" height="9" fill="#0c0c0a"/>
  <rect x="47" y="4" width="9" height="52" fill="#0c0c0a"/>
  <rect x="40" y="4" width="16" height="9" fill="#0c0c0a"/>
  <rect x="40" y="47" width="16" height="9" fill="#0c0c0a"/>
  <rect x="24" y="24" width="12" height="12" fill="#ff4d1c"/>
</svg>`;

function renderPng(size) {
  const resvg = new Resvg(ICON_SVG, {
    fitTo: { mode: "width", value: size },
  });
  return resvg.render().asPng();
}

const sizes = [16, 32, 48, 64, 120, 152, 167, 180, 192, 512];

console.log("Generating icons...");

// favicon-*.png in /public
for (const size of [16, 32, 64]) {
  const png = renderPng(size);
  writeFileSync(resolve(publicDir, `favicon-${size}x${size}.png`), png);
  console.log(`  favicon-${size}x${size}.png`);
}

// apple-touch-icon (180x180)
writeFileSync(resolve(publicDir, "apple-touch-icon.png"), renderPng(180));
console.log("  apple-touch-icon.png");

// android-chrome
writeFileSync(
  resolve(publicDir, "android-chrome-192x192.png"),
  renderPng(192)
);
console.log("  android-chrome-192x192.png");

writeFileSync(
  resolve(publicDir, "android-chrome-512x512.png"),
  renderPng(512)
);
console.log("  android-chrome-512x512.png");

// /public/icons/*
for (const size of sizes) {
  const png = renderPng(size);
  writeFileSync(resolve(iconsDir, `icon-${size}.png`), png);
  console.log(`  icons/icon-${size}.png`);
}

// favicon.ico — bundle 16, 32, 48
const icoBuffers = [renderPng(16), renderPng(32), renderPng(48)];
const ico = await pngToIco(icoBuffers);
writeFileSync(resolve(publicDir, "favicon.ico"), ico);
console.log("  favicon.ico");

console.log("Done.");
