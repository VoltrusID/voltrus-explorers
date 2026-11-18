import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { writeTextFile } from "@tauri-apps/plugin-fs";
import { decodeText } from "./textDecode";

/**
 * File access that works in the sandboxed App Store build.
 *
 * `<a download>` blobs and `<input type=file>` never reach the filesystem there. Instead the user
 * picks the file in a native panel (which grants the sandbox access to exactly that path) and the
 * fs plugin reads/writes it. Both apps need the dialog + fs plugins registered, the
 * `dialog:allow-open|save` and `fs:allow-read|write-text-file` permissions in their capabilities and
 * `com.apple.security.files.user-selected.read-write` in their entitlements.
 */

export interface FileFilter {
  name: string;
  extensions: string[];
}

export const CSV_FILTER: FileFilter = { name: "CSV", extensions: ["csv"] };

/**
 * Ask the user where to save `contents`, then write it.
 * Resolves with the path that was written, or `null` if the user cancelled the panel.
 * Rejects if the file could not be written — callers must not report success in that case.
 */
export async function saveTextFile(
  defaultName: string,
  contents: string,
  filter: FileFilter = CSV_FILTER,
): Promise<string | null> {
  const path = await save({ defaultPath: defaultName, filters: [filter] });
  if (!path) return null;
  await writeTextFile(path, contents);
  return path;
}

/**
 * Ask the user for a file and read it as text: UTF-8 when the bytes are valid UTF-8, Windows-1252
 * otherwise (DBC files from CANdb++ and other Windows tools), so no character turns into U+FFFD.
 * Resolves with `null` if the user cancelled the panel; rejects if the file could not be read.
 */
export async function openTextFile(
  filter: FileFilter = CSV_FILTER,
): Promise<{ path: string; text: string } | null> {
  const path = await open({ multiple: false, directory: false, filters: [filter] });
  if (!path) return null;
  // The fs plugin's `read_text_file` command returns the raw bytes (its JS wrapper decodes them as
  // UTF-8, replacing invalid sequences); calling it directly keeps the existing
  // `fs:allow-read-text-file` permission and lets us pick the encoding.
  const raw = await invoke<ArrayBuffer | number[]>("plugin:fs|read_text_file", { path });
  return { path, text: decodeText(raw instanceof ArrayBuffer ? new Uint8Array(raw) : Uint8Array.from(raw)) };
}

/**
 * Ask the user where to save a file, without writing anything.
 * Resolves with the chosen path, or `null` if the user cancelled the panel. Use it when the
 * writing happens later or in pieces (logging, capture); the panel grants the sandbox access.
 */
export async function pickSavePath(
  defaultName: string,
  filter: FileFilter = CSV_FILTER,
): Promise<string | null> {
  return (await save({ defaultPath: defaultName, filters: [filter] })) ?? null;
}

/** Append `text` to the file at `path`, creating it if needed. Rejects if the file can not be written. */
export async function appendTextFile(path: string, text: string): Promise<void> {
  await writeTextFile(path, text, { append: true });
}
