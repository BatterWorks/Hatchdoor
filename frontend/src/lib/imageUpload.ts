export const IMAGE_UPLOAD_MAX_EDGE = 2560;
export const IMAGE_UPLOAD_WEBP_QUALITY = 0.86;
export const IMAGE_UPLOAD_WEBP_TYPE = "image/webp";

type DecodedImage = {
  width: number;
  height: number;
  close?: () => void;
};

type EncoderCanvas = {
  width: number;
  height: number;
  getContext: HTMLCanvasElement["getContext"];
  toBlob: HTMLCanvasElement["toBlob"];
};

export type ImageUploadNormalizerDeps = {
  createImageBitmap: (file: File) => Promise<DecodedImage>;
  createCanvas: () => EncoderCanvas;
};

const browserImageDeps: ImageUploadNormalizerDeps = {
  createImageBitmap: async (file) => {
    try {
      // Bake EXIF orientation into pixels before the canvas re-encode, which
      // strips EXIF. WebKit does not auto-orient without this option, so
      // portrait phone photos would otherwise be stored sideways.
      return await window.createImageBitmap(file, {
        imageOrientation: "from-image",
      });
    } catch {
      // Fallback for engines that reject the imageOrientation option.
      return window.createImageBitmap(file);
    }
  },
  createCanvas: () => document.createElement("canvas"),
};

export async function normalizeImageForUpload(
  file: File,
  deps: ImageUploadNormalizerDeps = browserImageDeps,
): Promise<File> {
  if (!shouldConvertImage(file)) {
    return file;
  }

  let bitmap: DecodedImage | null = null;
  try {
    bitmap = await deps.createImageBitmap(file);
    const { width, height } = scaledDimensions(
      bitmap.width,
      bitmap.height,
      IMAGE_UPLOAD_MAX_EDGE,
    );
    const canvas = deps.createCanvas();
    canvas.width = width;
    canvas.height = height;

    const context = canvas.getContext("2d");
    if (!context) {
      return file;
    }
    context.drawImage(bitmap as CanvasImageSource, 0, 0, width, height);

    const blob = await canvasToBlob(
      canvas,
      IMAGE_UPLOAD_WEBP_TYPE,
      IMAGE_UPLOAD_WEBP_QUALITY,
    );
    const extension = extensionForImageType(blob.type);
    if (!extension) {
      return file;
    }
    return new File([blob], filenameWithExtension(file.name, extension), {
      type: blob.type,
      lastModified: file.lastModified,
    });
  } catch {
    return file;
  } finally {
    bitmap?.close?.();
  }
}

function shouldConvertImage(file: File): boolean {
  return (
    file.type.startsWith("image/") &&
    file.type !== "image/gif" &&
    file.type !== "image/svg+xml"
  );
}

function scaledDimensions(
  width: number,
  height: number,
  maxEdge: number,
): { width: number; height: number } {
  const longest = Math.max(width, height);
  if (longest <= maxEdge) {
    return { width, height };
  }
  const scale = maxEdge / longest;
  return {
    width: Math.max(1, Math.round(width * scale)),
    height: Math.max(1, Math.round(height * scale)),
  };
}

function canvasToBlob(
  canvas: EncoderCanvas,
  type: string,
  quality: number,
): Promise<Blob> {
  return new Promise((resolve, reject) => {
    canvas.toBlob(
      (blob) => {
        if (blob) {
          resolve(blob);
        } else {
          reject(new Error("image conversion failed"));
        }
      },
      type,
      quality,
    );
  });
}

function extensionForImageType(type: string): string | null {
  switch (type) {
    case "image/webp":
      return "webp";
    case "image/png":
      return "png";
    case "image/jpeg":
      return "jpg";
    default:
      return null;
  }
}

function filenameWithExtension(filename: string, extension: string): string {
  const trimmed = filename.trim() || "attachment";
  return /\.[^./\\]+$/.test(trimmed)
    ? trimmed.replace(/\.[^./\\]+$/, `.${extension}`)
    : `${trimmed}.${extension}`;
}
