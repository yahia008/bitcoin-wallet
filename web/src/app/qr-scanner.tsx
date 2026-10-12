"use client";

import { useEffect, useRef, useState } from "react";

import { CloseIcon, ImageIcon } from "./icons";

/** The browser's own QR reader, where there is one (Chrome on Android and macOS). Not in
 * TypeScript's DOM types yet. */
type BarcodeDetectorLike = { detect(source: CanvasImageSource): Promise<{ rawValue: string }[]> };
declare global {
  interface Window {
    BarcodeDetector?: new (options: { formats: string[] }) => BarcodeDetectorLike;
  }
}

/** Reads a QR code from a video frame or picture: the browser's reader if it has one, else
 * jsQR (Safari, Firefox), loaded only when first needed. */
async function readQr(source: HTMLVideoElement | ImageBitmap, canvas: HTMLCanvasElement) {
  if (window.BarcodeDetector) {
    const codes = await new window.BarcodeDetector({ formats: ["qr_code"] }).detect(source);
    return codes[0]?.rawValue;
  }
  const width = source instanceof HTMLVideoElement ? source.videoWidth : source.width;
  const height = source instanceof HTMLVideoElement ? source.videoHeight : source.height;
  if (!width || !height) return undefined;
  canvas.width = width;
  canvas.height = height;
  const ctx = canvas.getContext("2d", { willReadFrequently: true })!;
  ctx.drawImage(source, 0, 0);
  const { default: jsQR } = await import("jsqr");
  return jsQR(ctx.getImageData(0, 0, width, height).data, width, height)?.data;
}

function cameraError(e: unknown): string {
  const name = e instanceof DOMException ? e.name : "";
  if (name === "NotAllowedError") {
    return "Camera access is blocked. Allow it in your browser's settings, or choose a picture of the QR code.";
  }
  if (name === "NotFoundError" || name === "OverconstrainedError") {
    return "No camera found. Choose a picture of the QR code instead.";
  }
  return "Couldn't start the camera. Choose a picture of the QR code instead.";
}

/** Full-panel camera view that reads the first QR code it sees and hands over its text.
 * Without a camera (or permission), the user can pick a picture of a QR code instead, e.g. a
 * screenshot of an invoice. */
export function QrScanner({
  onScan,
  onClose,
}: {
  onScan: (text: string) => void;
  onClose: () => void;
}) {
  const videoRef = useRef<HTMLVideoElement>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [error, setError] = useState<string>();
  const [starting, setStarting] = useState(true);
  // The latest onScan, so the camera loop needn't restart when the parent re-renders.
  const onScanRef = useRef(onScan);
  useEffect(() => {
    onScanRef.current = onScan;
  });

  useEffect(() => {
    let stream: MediaStream | undefined;
    let timer: number | undefined;
    let stopped = false;

    async function scanFrame() {
      const video = videoRef.current;
      if (stopped || !video || !canvasRef.current) return;
      try {
        const text = video.readyState >= 2 ? await readQr(video, canvasRef.current) : undefined;
        if (text && !stopped) {
          stopped = true;
          onScanRef.current(text);
          return;
        }
      } catch {
        // A frame that couldn't be read; try the next one.
      }
      timer = window.setTimeout(scanFrame, 150);
    }

    (async () => {
      if (!navigator.mediaDevices?.getUserMedia) {
        // Browsers only offer the camera to HTTPS pages (and localhost).
        setError("The camera needs a secure (https) page. Choose a picture of the QR code instead.");
        setStarting(false);
        return;
      }
      try {
        stream = await navigator.mediaDevices.getUserMedia({
          video: { facingMode: "environment" }, // the back camera on phones
          audio: false,
        });
        if (stopped) return;
        const video = videoRef.current!;
        video.srcObject = stream;
        await video.play();
        setStarting(false);
        scanFrame();
      } catch (e) {
        if (!stopped) {
          setError(cameraError(e));
          setStarting(false);
        }
      }
    })();

    return () => {
      stopped = true;
      window.clearTimeout(timer);
      stream?.getTracks().forEach((t) => t.stop());
    };
  }, []);

  async function readPicture(file: File) {
    setError(undefined);
    try {
      const text = await readQr(await createImageBitmap(file), canvasRef.current!);
      if (text) onScan(text);
      else setError("No QR code found in that picture.");
    } catch {
      setError("Couldn't read that picture.");
    }
  }

  return (
    <div className="absolute inset-0 z-10 -mx-4 -my-6 flex flex-col bg-zinc-950 px-4 py-6 sm:-mx-6 sm:rounded-3xl sm:px-6">
      <div className="grid grid-cols-[2.5rem_1fr_2.5rem] items-center">
        <button
          aria-label="Close scanner"
          onClick={onClose}
          className="flex h-10 w-10 items-center justify-center rounded-full text-zinc-300 hover:bg-zinc-800/60"
        >
          <CloseIcon />
        </button>
        <h1 className="text-center text-lg font-semibold">Scan QR code</h1>
      </div>

      <div className="relative mt-6 aspect-square w-full overflow-hidden rounded-3xl bg-zinc-900">
        <video
          ref={videoRef}
          muted
          playsInline // iOS: play inline, not full screen
          className="h-full w-full object-cover"
        />
        {/* The viewfinder: a square with clear corners. */}
        <div className="pointer-events-none absolute inset-10 rounded-2xl border-2 border-white/70 shadow-[0_0_0_999px_rgba(0,0,0,0.35)]" />
        {(starting || error) && (
          <p className="absolute inset-0 flex items-center justify-center p-8 text-center text-sm text-zinc-300">
            {starting ? "Starting camera…" : error}
          </p>
        )}
      </div>
      <canvas ref={canvasRef} hidden />

      <p className="mt-4 text-center text-sm text-zinc-400">
        Point the camera at a bitcoin address or payment QR code.
      </p>
      <div className="flex-1" />
      <label className="flex h-12 w-full cursor-pointer items-center justify-center gap-2 rounded-full border border-zinc-800 text-sm font-semibold hover:border-zinc-600">
        <ImageIcon /> Choose a picture
        <input
          type="file"
          accept="image/*"
          aria-label="QR code picture"
          className="sr-only"
          onChange={(e) => {
            const file = e.target.files?.[0];
            e.target.value = "";
            if (file) readPicture(file);
          }}
        />
      </label>
    </div>
  );
}
