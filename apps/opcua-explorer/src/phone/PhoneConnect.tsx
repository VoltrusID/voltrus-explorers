import type { FormEvent } from "react";
import { Award, Plus, Radar, X } from "lucide-react";
import { POLICIES } from "./policies";
import { Button, Field, IconButton, ListGroup, ListRow, PageTitle, ProBadge, Section, SheetSelect, type SheetOption } from "@mactools/ui";

export interface ConnForm {
  endpointUrl: string;
  securityPolicy: string;
  securityMode: string;
  auth: "anonymous" | "user";
  username: string;
  password: string;
}

export const DEFAULT_CONN: ConnForm = {
  endpointUrl: "opc.tcp://localhost:4840",
  securityPolicy: "None",
  securityMode: "None",
  auth: "anonymous",
  username: "",
  password: "",
};

const MODES: SheetOption<string>[] = [
  { value: "None", label: "None", hint: "Nothing is signed or encrypted" },
  { value: "Sign", label: "Sign", hint: "Signed, so tampering shows. Not encrypted" },
  { value: "SignAndEncrypt", label: "Sign and encrypt", hint: "Signed and encrypted, the strongest" },
];

const AUTH: SheetOption<ConnForm["auth"]>[] = [
  { value: "anonymous", label: "Anonymous", hint: "No user name, many test servers allow it" },
  { value: "user", label: "User name and password", hint: "Sent over the secure channel" },
];

export interface SessionRow {
  id: string;
  label: string;
  endpoint: string;
  nodes: number;
}

interface Props {
  form: ConnForm;
  onForm: (next: ConnForm) => void;
  sessions: SessionRow[];
  activeId: string | null;
  adding: boolean;
  loading: boolean;
  onConnect: () => void;
  onCancel: () => void;
  onActivate: (id: string) => void;
  onDisconnect: (id: string) => void;
  onAdd: () => void;
  onDiscover: () => void;
  onCerts: () => void;
}

/** The Connect tab: the sessions that are open, or the endpoint form with security and sign-in as sheets. */
export function PhoneConnect({ form, onForm, sessions, activeId, adding, loading, onConnect, onCancel, onActivate, onDisconnect, onAdd, onDiscover, onCerts }: Props) {
  const showForm = sessions.length === 0 || adding;
  const set = (patch: Partial<ConnForm>) => onForm({ ...form, ...patch });
  const secure = form.securityPolicy !== "None";

  const tools = (
    <Section title="Tools">
      <ListGroup className="vm-group--flat">
        <ListRow icon={<Radar size={18} strokeWidth={1.75} />} title="Find servers" subtitle="Look up servers and endpoints" chevron onClick={onDiscover} />
        <ListRow icon={<Award size={18} strokeWidth={1.75} />} title="Certificates" subtitle="Trust or reject server certificates" trailing={<ProBadge />} chevron onClick={onCerts} />
      </ListGroup>
    </Section>
  );

  if (!showForm) {
    return (
      <>
        <PageTitle title="Connect" caption={`${sessions.length} open ${sessions.length === 1 ? "session" : "sessions"}`} />
        <Section title="Sessions">
          <ListGroup className="vm-group--flat">
            {sessions.map((s) => (
              <ListRow
                key={s.id}
                title={s.label}
                subtitle={`${s.nodes} ${s.nodes === 1 ? "node" : "nodes"}`}
                selected={s.id === activeId}
                onClick={() => onActivate(s.id)}
                trailing={
                  <IconButton
                    label={`Disconnect ${s.label}`}
                    onClick={(e) => {
                      e.stopPropagation();
                      onDisconnect(s.id);
                    }}
                  >
                    <X size={18} strokeWidth={1.75} />
                  </IconButton>
                }
              />
            ))}
          </ListGroup>
        </Section>
        <Button block size="lg" icon={<Plus size={18} strokeWidth={2} />} onClick={onAdd}>
          New connection {sessions.length > 0 && <ProBadge />}
        </Button>
        {tools}
      </>
    );
  }

  const submit = (e: FormEvent) => {
    e.preventDefault();
    onConnect();
  };

  return (
    <form onSubmit={submit} style={{ display: "contents" }}>
      <PageTitle title={adding ? "New connection" : "Connect"} caption="OPC UA server" />
      <Section title="Server">
        <Field stacked label="Endpoint URL">
          {(id) => (
            <input
              id={id}
              className="vt-input vt-input--mono"
              type="url"
              inputMode="url"
              autoCapitalize="off"
              autoCorrect="off"
              spellCheck={false}
              placeholder="opc.tcp://host:4840"
              value={form.endpointUrl}
              onChange={(e) => set({ endpointUrl: e.target.value })}
            />
          )}
        </Field>
        <ListGroup className="vm-group--flat">
          <ListRow icon={<Radar size={18} strokeWidth={1.75} />} title="Find servers" subtitle="Look up servers and endpoints" chevron onClick={onDiscover} />
        </ListGroup>
      </Section>

      <Section title="Security">
        <Field label="Policy">
          <SheetSelect
            sheetTitle="Security policy"
            value={form.securityPolicy}
            options={POLICIES}
            onChange={(v) => set({ securityPolicy: v, securityMode: v === "None" ? "None" : form.securityMode === "None" ? "SignAndEncrypt" : form.securityMode })}
          />
        </Field>
        <Field label="Mode">
          <SheetSelect
            sheetTitle="Security mode"
            value={form.securityMode}
            options={MODES}
            onChange={(v) => set({ securityMode: v, securityPolicy: v !== "None" && form.securityPolicy === "None" ? "Basic256Sha256" : form.securityPolicy })}
          />
        </Field>
        {secure && (
          <span className="vt-hint">
            A secure channel needs the server to trust this app. If the server refuses the connection, trust the app's certificate on the server and connect again.
          </span>
        )}
        <ListGroup className="vm-group--flat">
          <ListRow icon={<Award size={18} strokeWidth={1.75} />} title="Certificates" subtitle="Trust or reject server certificates" trailing={<ProBadge />} chevron onClick={onCerts} />
        </ListGroup>
      </Section>

      <Section title="Sign in">
        <Field label="Identity">
          <SheetSelect sheetTitle="Sign in as" value={form.auth} options={AUTH} onChange={(v) => set({ auth: v })} />
        </Field>
        {form.auth === "user" && (
          <>
            <Field label="User name">
              {(id) => (
                <input id={id} className="vt-input" autoCapitalize="off" autoCorrect="off" spellCheck={false} autoComplete="username" value={form.username} onChange={(e) => set({ username: e.target.value })} />
              )}
            </Field>
            <Field label="Password">
              {(id) => <input id={id} className="vt-input" type="password" autoComplete="current-password" value={form.password} onChange={(e) => set({ password: e.target.value })} />}
            </Field>
          </>
        )}
      </Section>

      <div style={{ display: "flex", flexDirection: "column", gap: 8 }}>
        <Button type="submit" variant="primary" block size="lg" loading={loading} disabled={!form.endpointUrl.trim()}>
          {loading ? "Connecting" : "Connect"}
        </Button>
        {(loading || adding) && (
          <Button type="button" variant="ghost" block onClick={onCancel}>
            Cancel
          </Button>
        )}
      </div>
    </form>
  );
}
