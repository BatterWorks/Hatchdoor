import { describe, expect, it, vi } from "vitest";

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
