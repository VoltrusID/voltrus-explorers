// Drift detection (PLC-6): compare the CRC32 of the bundle the project last
// deployed against the CRC32 the running target reports for its loaded
// bundle. Pure logic, kept separate from TargetPanel so it can be unit
// tested without React.

export type DriftStatus = "sync" | "drift" | "unknown";

export interface DriftInfo {
  status: DriftStatus;
  projectCrc: number | null;
  runningCrc: number | null;
}

/**
 * Compare the project-side CRC with the running CRC.
 *
 * A CRC of 0 counts as "unknown" (proto3 default / nothing loaded), as does
 * a missing value on either side.
 */
export function compareCrc(
  projectCrc: number | null | undefined,
  runningCrc: number | null | undefined,
): DriftStatus {
  if (!projectCrc || !runningCrc) return "unknown";
  return projectCrc === runningCrc ? "sync" : "drift";
}

/** Short hex form for badge display, e.g. 0xA1B2C3D4. */
export function formatCrcHex(crc: number): string {
  return "0x" + crc.toString(16).toUpperCase().padStart(8, "0");
}
