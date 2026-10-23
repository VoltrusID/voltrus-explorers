import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowUp, Check, ChevronRight, Eye, Folder, Gauge, Info, Pencil, RefreshCw, Box, ChartLine } from "lucide-react";
import { Badge, Button, Dialog, EmptyState, IconButton, ListGroup, ListRow, PageTitle, Segmented } from "@mactools/ui";
import type { AttributeRead, BrowseChild, ReferenceInfo } from "../BrowserPanel";
import { fmtValue, plain } from "./format";

type Dir = "tx" | "rx" | "info" | "error";

interface Crumb {
  id: string;
  name: string;
}

export interface BrowseNodeRef {
  nodeId: string;
  name: string;
  dataType: string;
  value: unknown;
}

interface Props {
  session: string;
  /** The latest value of a node on the Watch list; undefined when the node is not watched. */
  watched: (nodeId: string) => { value: unknown; dataType: string } | undefined;
  trending: (nodeId: string) => boolean;
  onWatch: (n: BrowseNodeRef) => void;
  onTrend: (n: BrowseNodeRef) => void;
  onWrite: (n: BrowseNodeRef) => void;
  onLog: (d: Dir, m: string) => void;
}

const ROOT: Crumb = { id: "", name: "Objects" };

function iconFor(nodeClass: string) {
  if (nodeClass === "Variable") return <Gauge size={18} strokeWidth={1.75} />;
  if (nodeClass === "Object") return <Folder size={18} strokeWidth={1.75} />;
  return <Box size={18} strokeWidth={1.75} />;
}

/**
 * The address space as drill-down lists: tap an object to go into it, a breadcrumb bar to jump back up, and the attributes
 * and references of any node in a sheet where it can be read again, written, watched and trended.
 */
export function PhoneBrowse({ session, watched, trending, onWatch, onTrend, onWrite, onLog }: Props) {
  const [path, setPath] = useState<Crumb[]>([ROOT]);
  const [children, setChildren] = useState<Record<string, BrowseChild[]>>({});
  const [loading, setLoading] = useState(false);
  const [failed, setFailed] = useState<string | null>(null);
  const [node, setNode] = useState<BrowseChild | null>(null);
  const crumbsRef = useRef<HTMLDivElement>(null);
  const here = path[path.length - 1]!;

  const load = useCallback(
    async (id: string, force = false) => {
      if (!force && children[id]) return;
      setLoading(true);
      setFailed(null);
      try {
        const kids = await invoke<BrowseChild[]>("browse_children", { session, nodeId: id });
        setChildren((prev) => ({ ...prev, [id]: kids ?? [] }));
        onLog("rx", `Browsed ${kids?.length ?? 0} children under ${id || "ObjectsFolder"}`);
      } catch (e) {
        setFailed(String(e));
        onLog("error", `Browse failed: ${e}`);
      }
      setLoading(false);
    },
    [children, session, onLog],
  );

  useEffect(() => {
    void load(here.id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [here.id]);

  // Keep the current level in view in the breadcrumb bar.
  useEffect(() => {
    const bar = crumbsRef.current;
    if (bar) bar.scrollLeft = bar.scrollWidth;
  }, [path]);

  const rows = children[here.id] ?? [];
  const label = (n: BrowseChild) => n.display_name || n.browse_name || n.node_id;

  return (
    <>
      <PageTitle
        title="Browse"
        caption={here.id ? here.name : "Address space"}
        actions={
          <>
            {path.length > 1 && (
              <IconButton label="Up one level" onClick={() => setPath((p) => p.slice(0, -1))}>
                <ArrowUp size={20} strokeWidth={1.75} />
              </IconButton>
            )}
            <IconButton label="Reload this level" onClick={() => void load(here.id, true)}>
              <RefreshCw size={20} strokeWidth={1.75} />
            </IconButton>
          </>
        }
      />
      <div className="oc-crumbs" ref={crumbsRef} role="navigation" aria-label="Path">
        {path.map((c, i) => (
          <span key={`${i}:${c.id}`} style={{ display: "contents" }}>
            {i > 0 && <ChevronRight size={16} strokeWidth={1.75} className="oc-crumbs__sep" aria-hidden="true" />}
            <button type="button" className="oc-crumb" aria-current={i === path.length - 1} onClick={() => setPath((p) => p.slice(0, i + 1))}>
              {c.name}
            </button>
          </span>
        ))}
      </div>
      <div className="oc-scroll">
        {failed ? (
          <EmptyState compact title="Could not browse" description={failed} action={<Button onClick={() => void load(here.id, true)}>Try again</Button>} />
        ) : loading && rows.length === 0 ? (
          <div className="oc-note">Reading the address space</div>
        ) : rows.length === 0 ? (
          <EmptyState compact title="Nothing here" description="This node has no children. Open a variable to read or write its value." />
        ) : (
          <ListGroup>
            {rows.map((n) => {
              const isVar = n.node_class === "Variable";
              return (
                <ListRow
                  key={n.node_id}
                  icon={iconFor(n.node_class)}
                  title={label(n)}
                  subtitle={`${n.node_class}${n.data_type ? ` · ${n.data_type}` : ""}`}
                  value={isVar && watched(n.node_id) ? fmtValue(watched(n.node_id)!.value, n.data_type) : undefined}
                  chevron
                  onClick={() => (isVar ? setNode(n) : setPath((p) => [...p, { id: n.node_id, name: label(n) }]))}
                  trailing={
                    isVar ? undefined : (
                      <IconButton
                        label={`Details of ${label(n)}`}
                        onClick={(e) => {
                          e.stopPropagation();
                          setNode(n);
                        }}
                      >
                        <Info size={18} strokeWidth={1.75} />
                      </IconButton>
                    )
                  }
                />
              );
            })}
          </ListGroup>
        )}
      </div>
      {node && (
        <NodeSheet
          key={node.node_id}
          session={session}
          start={node}
          watching={(id) => watched(id) !== undefined}
          trending={trending}
          onWatch={onWatch}
          onTrend={onTrend}
          onWrite={onWrite}
          onLog={onLog}
          onClose={() => setNode(null)}
        />
      )}
    </>
  );
}

function NodeSheet({
  session,
  start,
  watching,
  trending,
  onWatch,
  onTrend,
  onWrite,
  onLog,
  onClose,
}: {
  session: string;
  start: BrowseChild;
  watching: (nodeId: string) => boolean;
  trending: (nodeId: string) => boolean;
  onWatch: (n: BrowseNodeRef) => void;
  onTrend: (n: BrowseNodeRef) => void;
  onWrite: (n: BrowseNodeRef) => void;
  onLog: (d: Dir, m: string) => void;
  onClose: () => void;
}) {
  const [id, setId] = useState(start.node_id);
  const [attrs, setAttrs] = useState<AttributeRead[]>([]);
  const [refs, setRefs] = useState<ReferenceInfo[]>([]);
  const [tab, setTab] = useState<"attributes" | "references">("attributes");
  const [busy, setBusy] = useState(false);

  const readAttrs = useCallback(async () => {
    return invoke<AttributeRead[]>("read_attributes", { session, nodeId: id }).catch((e) => {
      onLog("error", `read_attributes: ${e}`);
      return [] as AttributeRead[];
    });
  }, [session, id, onLog]);

  const loadAll = useCallback(async () => {
    setBusy(true);
    const [a, r] = await Promise.all([
      readAttrs(),
      invoke<ReferenceInfo[]>("browse_references", { session, nodeId: id }).catch((e) => {
        onLog("error", `browse_references: ${e}`);
        return [] as ReferenceInfo[];
      }),
    ]);
    setAttrs(a);
    setRefs(r);
    setBusy(false);
  }, [readAttrs, session, id, onLog]);

  useEffect(() => {
    void loadAll();
  }, [loadAll]);

  const attr = (name: string) => attrs.find((a) => a.attribute === name);
  const nodeClass = String(attr("NodeClass")?.value ?? (id === start.node_id ? start.node_class : ""));
  const isVar = nodeClass === "Variable";

  // A variable's value is live while the sheet is open (the sheet is where a write is checked).
  useEffect(() => {
    if (!isVar) return;
    const t = window.setInterval(() => {
      void readAttrs().then((a) => a.length > 0 && setAttrs(a));
    }, 2000);
    return () => window.clearInterval(t);
  }, [isVar, readAttrs]);

  const name = String(attr("DisplayName")?.value ?? (id === start.node_id ? start.display_name || start.browse_name : id));
  const value = attr("Value");
  const dataType = String(attr("DataType")?.value ?? (id === start.node_id ? start.data_type : ""));
  // AccessLevel bit 1 is "current write". Unknown access counts as writable so the server decides.
  const access = attr("UserAccessLevel") ?? attr("AccessLevel");
  const writable = access === undefined || (Number(access.value) & 2) === 2;
  const ref: BrowseNodeRef = { nodeId: id, name, dataType, value: value?.value };
  const watched = watching(id);
  const trended = trending(id);
  const numeric = value !== undefined && value.value !== null && !Number.isNaN(Number(value.value));

  return (
    <Dialog open onClose={onClose} title={name}>
      <div className="oc-stack">
      <div className="oc-hero">
        {isVar && value ? (
          <>
            <span className="oc-value oc-value--hero">{fmtValue(value.value)}</span>
            <span style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
              <Badge tone={value.status === "Good" ? "success" : "danger"}>{value.status}</Badge>
              {dataType && <Badge>{dataType}</Badge>}
              {!writable && <Badge>Read only</Badge>}
            </span>
          </>
        ) : (
          nodeClass && <Badge>{nodeClass}</Badge>
        )}
        <span className="oc-hero__id">{id}</span>
      </div>

      {isVar && (
        <div className="oc-actions">
          <Button icon={<RefreshCw size={16} strokeWidth={2} />} loading={busy} onClick={() => void loadAll()}>
            Read
          </Button>
          <Button variant="primary" disabled={!writable} icon={<Pencil size={16} strokeWidth={2} />} onClick={() => onWrite(ref)}>
            Write
          </Button>
          <Button disabled={watched} icon={watched ? <Check size={16} strokeWidth={2} /> : <Eye size={16} strokeWidth={2} />} onClick={() => onWatch(ref)}>
            {watched ? "Watching" : "Watch"}
          </Button>
        </div>
      )}
      {isVar && numeric && (
        <Button block icon={<ChartLine size={16} strokeWidth={2} />} onClick={() => onTrend(ref)}>
          {trended ? "Remove from trend" : "Add to trend"}
        </Button>
      )}

      <Segmented
        block
        label="Node detail"
        value={tab}
        onChange={setTab}
        options={[
          { value: "attributes", label: "Attributes" },
          { value: "references", label: refs.length > 0 ? `References (${refs.length})` : "References" },
        ]}
      />

      {tab === "attributes" ? (
        attrs.length === 0 ? (
          <div className="oc-note">{busy ? "Reading attributes" : "No attributes readable on this node."}</div>
        ) : (
          <ListGroup>
            {attrs.map((a) => (
              <div key={a.attribute} className="vm-row oc-kv">
                <span className="oc-kv__k">{a.attribute}</span>
                <span className={a.status === "Good" ? "oc-kv__v" : "oc-kv__v oc-kv__v--bad"}>{plain(a.value)}</span>
              </div>
            ))}
          </ListGroup>
        )
      ) : refs.length === 0 ? (
        <div className="oc-note">{busy ? "Reading references" : "No references."}</div>
      ) : (
        <ListGroup>
          {refs.map((r, i) => (
            <ListRow
              key={`${i}:${r.node_id}`}
              title={r.display_name || r.browse_name}
              subtitle={`${r.is_forward ? "" : "From: "}${r.reference_type} · ${r.node_class}`}
              chevron
              onClick={() => {
                setId(r.node_id);
                setTab("attributes");
              }}
            />
          ))}
        </ListGroup>
      )}
      </div>
    </Dialog>
  );
}
