import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import {
  IMAGE_UPLOAD_MAX_EDGE,
  IMAGE_UPLOAD_WEBP_QUALITY,
  IMAGE_UPLOAD_WEBP_TYPE,
  normalizeImageForUpload,
  type ImageUploadNormalizerDeps,
} from "./imageUpload";

function normalizerDeps({
  bitmap = { width: 4000, height: 3000 },
  blob = new Blob(["webp-bytes"], { type: IMAGE_UPLOAD_WEBP_TYPE }),
}: {
  bitmap?: { width: number; height: number; close?: () => void };
  blob?: Blob | null;
} = {}) {
  const drawImage = vi.fn();
  const toBlob = vi.fn((callback: BlobCallback) => {
    callback(blob);
  });
  const canvas = {
    width: 0,
    height: 0,
    getContext: vi.fn(() => ({ drawImage })),
    toBlob,
  } as unknown as HTMLCanvasElement;
  const deps: ImageUploadNormalizerDeps = {
    createImageBitmap: vi.fn().mockResolvedValue(bitmap),
    createCanvas: vi.fn(() => canvas),
  };

  return { deps, canvas, drawImage, toBlob };
}

describe("normalizeImageForUpload", () => {
  it("resizes and converts still images to webp", async () => {
    const { deps, canvas, drawImage, toBlob } = normalizerDeps();
    const file = new File(["jpg-bytes"], "Vacation Photo.JPG", {
      type: "image/jpeg",
      lastModified: 1782000000000,
    });

    const normalized = await normalizeImageForUpload(file, deps);

    expect(deps.createImageBitmap).toHaveBeenCalledWith(file);
    expect(canvas.width).toBe(IMAGE_UPLOAD_MAX_EDGE);
    expect(canvas.height).toBe(1920);
    expect(drawImage).toHaveBeenCalledWith(expect.anything(), 0, 0, 2560, 1920);
    expect(toBlob).toHaveBeenCalledWith(
      expect.any(Function),
      IMAGE_UPLOAD_WEBP_TYPE,
      IMAGE_UPLOAD_WEBP_QUALITY,
    );
    expect(normalized.name).toBe("Vacation Photo.webp");
    expect(normalized.type).toBe(IMAGE_UPLOAD_WEBP_TYPE);
    expect(normalized.lastModified).toBe(file.lastModified);
    expect(normalized.size).toBe(10);
  });

  it("uses the actual canvas output type when webp is unsupported", async () => {
    const { deps } = normalizerDeps({
      blob: new Blob(["png-bytes"], { type: "image/png" }),
    });
    const file = new File(["jpg-bytes"], "photo.jpeg", {
      type: "image/jpeg",
    });

    const normalized = await normalizeImageForUpload(file, deps);

    expect(normalized.name).toBe("photo.png");
    expect(normalized.type).toBe("image/png");
    expect(normalized.size).toBe(9);
  });

  it("returns the original file when canvas returns an unsupported type", async () => {
    const { deps } = normalizerDeps({
      blob: new Blob(["png-bytes"], { type: "" }),
    });
    const file = new File(["jpg-bytes"], "photo.jpeg", {
      type: "image/jpeg",
    });

    await expect(normalizeImageForUpload(file, deps)).resolves.toBe(file);
  });

  it("keeps animated gifs on the original upload path", async () => {
    const { deps } = normalizerDeps();
    const file = new File(["gif-bytes"], "animation.gif", {
      type: "image/gif",
    });

    await expect(normalizeImageForUpload(file, deps)).resolves.toBe(file);
    expect(deps.createImageBitmap).not.toHaveBeenCalled();
  });

  it("keeps svg uploads on the backend validation path", async () => {
    const { deps } = normalizerDeps();
    const file = new File(["<svg />"], "diagram.svg", {
      type: "image/svg+xml",
    });

    await expect(normalizeImageForUpload(file, deps)).resolves.toBe(file);
    expect(deps.createImageBitmap).not.toHaveBeenCalled();
  });

  describe("default browser decode deps", () => {
    const originalCreateImageBitmap = (
      window as unknown as { createImageBitmap?: unknown }
    ).createImageBitmap;

    beforeEach(() => {
      (window as unknown as { createImageBitmap: unknown }).createImageBitmap =
        vi.fn().mockResolvedValue({ width: 3000, height: 4000 });
    });

    afterEach(() => {
      (window as unknown as { createImageBitmap?: unknown }).createImageBitmap =
        originalCreateImageBitmap;
    });

    it("applies EXIF orientation when decoding the source bitmap", async () => {
      const file = new File(["jpg-bytes"], "portrait.jpg", {
        type: "image/jpeg",
      });

      // Default deps route through window.createImageBitmap. Without
      // { imageOrientation: "from-image" }, WebKit decodes ignoring the EXIF
      // orientation tag, and the subsequent canvas re-encode strips EXIF, so
      // portrait phone photos are baked sideways.
      await normalizeImageForUpload(file);

      expect(window.createImageBitmap).toHaveBeenCalledWith(file, {
        imageOrientation: "from-image",
      });
    });
  });

  it("returns the original file if browser conversion fails", async () => {
    const deps: ImageUploadNormalizerDeps = {
      createImageBitmap: vi.fn().mockRejectedValue(new Error("decode failed")),
      createCanvas: vi.fn(() => document.createElement("canvas")),
    };
    const file = new File(["heic-bytes"], "photo.heic", {
      type: "image/heic",
    });

    await expect(normalizeImageForUpload(file, deps)).resolves.toBe(file);
  });
});
