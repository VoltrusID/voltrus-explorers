// A photo or screenshot of a manual page, scaled and re-encoded so it fits the console's limit
// (about 1 MB) and so camera metadata (location, device) never leaves the phone.

const MAX_SIDE = 1600;
/** The console accepts up to 1.4 million base64 characters; stay under it. */
const MAX_BASE64 = 1_300_000;

export async function fileToJpeg(file: File): Promise<{ mime: "image/jpeg"; data: string }> {
  const bitmap = await createImageBitmap(file).catch(() => null);
  if (!bitmap) throw new Error("That file could not be read as an image.");
  let scale = Math.min(1, MAX_SIDE / Math.max(bitmap.width, bitmap.height));
  for (let attempt = 0; attempt < 5; attempt++) {
    const canvas = document.createElement("canvas");
    canvas.width = Math.max(1, Math.round(bitmap.width * scale));
    canvas.height = Math.max(1, Math.round(bitmap.height * scale));
    const ctx = canvas.getContext("2d");
    if (!ctx) throw new Error("Could not prepare the photo.");
    // White behind transparent screenshots, so text stays readable as JPEG.
    ctx.fillStyle = "#fff";
    ctx.fillRect(0, 0, canvas.width, canvas.height);
    ctx.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
    const data = canvas.toDataURL("image/jpeg", attempt < 2 ? 0.85 : 0.7).split(",")[1] ?? "";
    if (data.length <= MAX_BASE64) {
      bitmap.close();
      return { mime: "image/jpeg", data };
    }
    scale *= 0.75;
  }
  bitmap.close();
  throw new Error("That photo is too large to send. Crop it to the table.");
}
