import { ListPlus, Play, Plug, Square } from "lucide-react";
import { Button, Callout, Field, PageTitle, ProBadge, Section, SheetSelect, StatusDot } from "@mactools/ui";
import { TEMPLATES } from "../simTemplates";

interface Props {
  template: string | null;
  running: boolean;
  port: number | null;
  portInput: number;
  starting: boolean;
  error: string | null;
  onPort: (n: number) => void;
  onTemplate: (t: string | null) => void;
  onStart: () => void;
  onStop: () => void;
  /** Points the active PLC at the running simulator. */
  onUse: () => void;
  /** Copies the template's points into the active PLC as tags. */
  onLoadTags: () => void;
}

/** The Simulator page: a real S7 server on this phone that other S7 clients (or this app) can connect to. */
export default function PhoneSimulator({ template, running, port, portInput, starting, error, onPort, onTemplate, onStart, onStop, onUse, onLoadTags }: Props) {
  return (
    <div className="vm-body">
      <PageTitle title="Simulator" caption={running ? `Listening on port ${port ?? portInput}` : "Test without a PLC"} />
      {error && <Callout tone="danger">{error}</Callout>}
      <Section
        title="Server"
        meta={
          <span className="s7-meta">
            <ProBadge />
            <StatusDot tone={running ? "success" : "neutral"} label={running ? "Simulator running" : "Simulator stopped"} />
            {running ? "Running" : "Stopped"}
          </span>
        }
      >
        <div className="vt-stack vt-rows">
          <Field label="Template" hint={template ? TEMPLATES[template].description : undefined}>
            <SheetSelect
              sheetTitle="Template"
              value={template ?? ""}
              placeholder="Choose"
              disabled={running}
              onChange={(t) => onTemplate(t || null)}
              options={Object.entries(TEMPLATES).map(([id, t]) => ({ value: id, label: t.name }))}
            />
          </Field>
          <Field label="TCP port">
            {(id) => (
              <input id={id} className="vt-input vt-input--mono" type="number" inputMode="numeric" min={1} max={65535} value={portInput} disabled={running} onChange={(e) => onPort(Number(e.target.value))} />
            )}
          </Field>
          {!running ? (
            <Button variant="primary" block size="lg" icon={<Play size={16} strokeWidth={1.75} />} disabled={!template || starting} loading={starting} onClick={onStart}>
              {starting ? "Starting…" : "Start server"}
            </Button>
          ) : (
            <Button variant="danger" block size="lg" icon={<Square size={16} strokeWidth={1.75} />} onClick={onStop}>
              Stop server
            </Button>
          )}
        </div>
      </Section>
      {running && port ? (
        <Section title="Use it">
          <div className="s7-copy">
            <p>
              Clients on this phone connect to <span className="mono">127.0.0.1:{port}</span>. Other devices on the same Wi-Fi use this phone's address with the same port. Rack 0, slot 0.
            </p>
          </div>
          <Button variant="primary" block icon={<Plug size={16} strokeWidth={1.75} />} onClick={onUse}>
            Connect this PLC to it
          </Button>
          <Button block icon={<ListPlus size={16} strokeWidth={1.75} />} onClick={onLoadTags}>
            Load its tags
          </Button>
        </Section>
      ) : (
        <div className="s7-copy s7-copy--muted">
          <p>Start the server, then connect this app to <span className="mono">127.0.0.1</span> on the port above, or let another S7 client on your network connect.</p>
          <p>iOS pauses apps in the background, so keep S7 Explorer open while a client is connected.</p>
        </div>
      )}
    </div>
  );
}
