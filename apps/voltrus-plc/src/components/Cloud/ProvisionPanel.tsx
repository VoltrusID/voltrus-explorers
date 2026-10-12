// WebSerial ESP32 provisioning UI (PLC-26). Flashes the runtime image from
// the release manifest over USB and surfaces the pairing token issued by
// POST /api/devices. NVS provisioning is not possible with esptool-js (raw
// flash writes only), so that step shows honest manual instructions instead.
import { useEffect, useState } from "react";
import { Button, Callout } from "@mactools/ui";
import { useCloudStore } from "../../store/cloud";
import {
  APP_IMAGE_OFFSET,
  PROVISION_BASE_KEY,
  buildFlashPlan,
  cliFallbackCommand,
  downloadVerifiedImage,
  fetchManifest,
  isWebSerialAvailable,
  provisionBase,
  registerDevice,
  type RegisteredDevice,
  type RuntimeManifest,
} from "../../api/provision";
import "./cloud.css";

type FlashPhase = "idle" | "connecting" | "downloading" | "flashing" | "done";

const LOG_LIMIT = 200;

type Log = (line: string) => void;
type Progress = (loaded: number, total: number) => void;

// Dynamic import is required: esptool-js (and its pako payload) must land in
// a lazy chunk that non-Chromium bundles never download, so the specifier is
// deliberately not hoisted to a static import.
async function flashRuntimeImage(manifest: RuntimeManifest, log: Log, setPhase: (p: FlashPhase) => void, onProgress: Progress): Promise<void> {
  const { ESPLoader, Transport } = await import("esptool-js");
  const port = await navigator.serial.requestPort();
  const transport = new Transport(port, true);
  try {
    const loader = new ESPLoader({
      transport,
      baudrate: 115200,
      romBaudrate: 115200,
      terminal: {
        clean: () => {},
        write: (data: string) => log(data.replace(/\n+$/, "")),
        writeLine: (line: string) => log(line),
      },
    });

    const chipName = await loader.main();
    log(`Connected: ${chipName}`);

    const entry = buildFlashPlan(manifest)[0];
    setPhase("downloading");
    const image = await downloadVerifiedImage(entry, onProgress);
    log(`Image verified: ${image.length} bytes, sha256 ${entry.sha256.slice(0, 12)}…`);

    setPhase("flashing");
    await loader.writeFlash({
      fileArray: [{ data: image, address: entry.address }],
      flashSize: "keep",
      flashMode: "keep",
      flashFreq: "keep",
      eraseAll: false,
      compress: true,
      reportProgress: (_fileIndex, written, total) => onProgress(written, total),
    });
    await loader.after();
  } finally {
    await transport.disconnect().catch(() => {});
  }
}

export default function ProvisionPanel() {
  const token = useCloudStore((s) => s.token);

  const [serialOk] = useState(isWebSerialAvailable);
  const [manifest, setManifest] = useState<RuntimeManifest | null>(null);
  const [manifestError, setManifestError] = useState<string | null>(null);

  const [deviceName, setDeviceName] = useState("");
  const [registering, setRegistering] = useState(false);
  const [device, setDevice] = useState<RegisteredDevice | null>(null);

  const [phase, setPhase] = useState<FlashPhase>("idle");
  const [progress, setProgress] = useState<{ loaded: number; total: number } | null>(null);
  const [log, setLog] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    fetchManifest()
      .then((m) => {
        if (!cancelled) setManifest(m);
      })
      .catch((e) => {
        if (!cancelled) setManifestError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const busy = phase !== "idle" && phase !== "done";

  const appendLog: Log = (line) => setLog((prev) => [...prev.slice(-(LOG_LIMIT - 1)), line]);

  async function register() {
    if (!token || deviceName.trim().length === 0) return;
    setRegistering(true);
    setError(null);
    try {
      setDevice(await registerDevice(token, deviceName.trim()));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setRegistering(false);
    }
  }

  async function connectAndFlash() {
    if (!manifest || busy) return;
    setPhase("connecting");
    setError(null);
    setProgress(null);
    setLog([]);
    try {
      await flashRuntimeImage(manifest, appendLog, setPhase, (loaded, total) => setProgress({ loaded, total }));
      setPhase("done");
    } catch (e) {
      setPhase("idle");
      setError(e instanceof Error ? e.message : String(e));
    }
  }

  const pct =
    progress && progress.total > 0 ? Math.min(100, Math.round((progress.loaded / progress.total) * 100)) : 0;

  return (
    <div className="cld-panel">
      <div className="cld-panel__title">Device provisioning</div>

      {/* Step 1: browser + manifest */}
      <div className="cld-step">
        <div className="cld-step__title">1. Browser and manifest</div>
        <div className="cld-hint">
          Browser: {serialOk ? "Web Serial available" : "Web Serial NOT available"} · Manifest:{" "}
          {manifest ? (
            <span className="cld-mono">runtime v{manifest.version} ({manifest.chip})</span>
          ) : manifestError ? (
            "failed"
          ) : (
            "loading…"
          )}
        </div>
        {manifestError && (
          <Callout tone="danger">
            Manifest failed: {manifestError}
            <div className="cld-hint" style={{ marginTop: 2 }}>
              Base: <span className="cld-mono">{provisionBase()}</span> — override with localStorage key{" "}
              <span className="cld-mono">{PROVISION_BASE_KEY}</span>
            </div>
          </Callout>
        )}
        {!serialOk && (
          <Callout tone="warn">
            <div>
              This browser has no Web Serial. Use Chrome/Edge/Chromium to flash over USB, or flash manually:
            </div>
            <div className="cld-mono" style={{ margin: "2px 0" }}>{cliFallbackCommand()}</div>
            {manifest && (
              <a href={manifest.url} download className="cld-link">
                Download runtime image v{manifest.version} ({Math.ceil(manifest.size / 1024)} KB)
              </a>
            )}
          </Callout>
        )}
      </div>

      {/* Step 2: register device, get pairing token */}
      <div className="cld-step">
        <div className="cld-step__title">2. Register device</div>
        {!token ? (
          <div className="cld-hint">Sign in to provision devices.</div>
        ) : device ? (
          <>
            <div className="cld-hint">
              Registered <span style={{ fontWeight: 600, color: "var(--text)" }}>{device.name}</span>{" "}
              (<span className="cld-mono">{device.id}</span>)
            </div>
            <div className="cld-token">{device.pairingToken}</div>
            <Callout tone="warn">Pairing token is shown only once. Store it now.</Callout>
          </>
        ) : (
          <div className="cld-form-row">
            <input
              type="text"
              className="vt-input vt-input--sm cld-grow"
              placeholder="Device name"
              aria-label="Device name"
              value={deviceName}
              onChange={(e) => setDeviceName(e.target.value)}
            />
            <Button
              size="sm"
              disabled={registering || deviceName.trim().length === 0}
              onClick={() => void register()}
            >
              {registering ? "…" : "Register"}
            </Button>
          </div>
        )}
      </div>

      {/* Step 3: connect + flash (Chromium only) */}
      {serialOk && (
        <div className="cld-step">
          <div className="cld-step__title">3. Flash runtime</div>
          <div className="cld-form-row">
            <Button
              size="sm"
              variant="primary"
              disabled={!manifest || !device || busy}
              onClick={() => void connectAndFlash()}
            >
              {phase === "idle" ? "Connect & flash" : phase === "done" ? "Reflash" : `${phase}…`}
            </Button>
            {progress && busy && (
              <span className="cld-progress">
                {pct}% ({progress.loaded}/{progress.total})
              </span>
            )}
          </div>
          <div className="cld-hint">
            App image at offset <span className="cld-mono">0x{APP_IMAGE_OFFSET.toString(16)}</span> · erase-all
            off, existing NVS keys survive.
          </div>
        </div>
      )}

      {/* Step 4: provisioning — esptool-js has no NVS writer, so this is manual */}
      <div className="cld-step">
        <div className="cld-step__title">4. Provision (manual)</div>
        <div className="cld-hint">
          esptool-js writes raw flash only — it cannot build or write NVS partitions in-browser. After
          flashing, provision the board out-of-band:
          <ol className="cld-ol">
            <li>
              Open the board's USB serial console (e.g. <span className="cld-mono">screen</span>,{" "}
              <span className="cld-mono">minicom</span>, or a Web Serial terminal) and enter the
              Wi-Fi credentials plus the pairing token from step 2.
            </li>
            <li>
              Alternatively, generate an NVS image offline with{" "}
              <span className="cld-mono">nvs_partition_gen.py</span> and flash it with{" "}
              <span className="cld-mono">esptool.py --chip esp32s3 write_flash &lt;nvs-offset&gt; nvs.bin</span>.
            </li>
          </ol>
        </div>
      </div>

      {device && (
        <div className="cld-hint">
          Once the device connects, it appears under Devices in the cloud dashboard (
          <span className="cld-mono">GET /api/devices</span>).
        </div>
      )}

      {phase === "done" && <Callout tone="success">Flash complete. Device reset.</Callout>}
      {error && <Callout tone="danger">{error}</Callout>}
      {log.length > 0 && (
        <details>
          <summary className="cld-hint" style={{ cursor: "pointer" }}>esptool log</summary>
          <pre
            className="cld-mono"
            style={{
              maxHeight: 120,
              overflow: "auto",
              margin: "2px 0 0",
              whiteSpace: "pre-wrap",
            }}
          >
            {log.join("\n")}
          </pre>
        </details>
      )}
    </div>
  );
}
