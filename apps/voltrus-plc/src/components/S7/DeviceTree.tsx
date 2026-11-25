import { useEffect, useState, type ReactNode } from "react";
import { IconButton } from "@mactools/ui";
import { Activity, Blocks, ChevronDown, ChevronRight, Cpu, Database, Plus, Table2, Tags } from "lucide-react";
import { useStore } from "../../store";
import { createWatchTable, listDeviceDocs, openDoc, openOnlineTab, readDoc } from "../../lib/deviceProject";
import { docTitle, ONLINE_PATH } from "../../lib/s7Docs";
import { deviceLabel, useS7 } from "../../lib/s7Store";

function Group({ label, icon, open, onToggle, action, children, depth = 1 }: {
  label: string;
  icon: ReactNode;
  open: boolean;
  onToggle: () => void;
  action?: ReactNode;
  children: ReactNode;
  depth?: number;
}) {
  return (
    <li>
      <div className="tree-row" style={{ paddingLeft: depth * 14 }} onClick={onToggle} role="treeitem" aria-expanded={open}>
        {open ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
        {icon}
        <span className="tree-row__name">{label}</span>
        {action && <span className="tree-row__action" onClick={(e) => e.stopPropagation()}>{action}</span>}
      </div>
      {open && <ul role="group">{children}</ul>}
    </li>
  );
}

function Leaf({ label, icon, active, onClick, mark, title, depth = 2 }: {
  label: string;
  icon: ReactNode;
  active?: boolean;
  onClick?: () => void;
  mark?: ReactNode;
  title?: string;
  depth?: number;
}) {
  return (
    <li>
      <div className={`tree-row ${active ? "is-active" : ""}`} style={{ paddingLeft: depth * 14 + 12 }}
        onClick={onClick} role="treeitem" title={title}>
        {icon}
        <span className="tree-row__name">{label}</span>
        {mark}
      </div>
    </li>
  );
}

/** Status mark of the CPU: green in RUN, amber in STOP, grey offline. */
function DeviceMark({ online, mode }: { online: boolean; mode: string | null }) {
  const cls = !online ? "s7-mark" : mode === "RUN" ? "s7-mark s7-mark--ok" : mode === "STOP" ? "s7-mark s7-mark--stop" : "s7-mark s7-mark--warn";
  return <span className={cls} title={online ? `Online, ${mode ?? "mode unknown"}` : "Offline"} />;
}

/**
 * The Siemens device in the left rail: the CPU with its Online & diagnostics view, watch tables,
 * tag tables and DBs, and (online) the blocks in the CPU.
 */
export default function DeviceTree() {
  const { info, target, mode, blocks } = useS7();
  // Re-render when files open, close or the project folder changes.
  const openFiles = useStore((s) => s.openFiles);
  const activeFileIndex = useStore((s) => s.activeFileIndex);
  useStore((s) => s.projectFiles);
  const activePath = openFiles[activeFileIndex]?.path;
  const online = info !== null;
  const [open, setOpen] = useState({ device: true, watch: true, tags: true, blocks: false });
  const toggle = (k: keyof typeof open) => setOpen((o) => ({ ...o, [k]: !o[k] }));

  const watchTables = listDeviceDocs("watch");
  const tagTables = listDeviceDocs("tags");

  return (
    <ul className="device-tree" role="tree" aria-label="Devices">
      <li>
        <div className="tree-row tree-row--device" onClick={() => toggle("device")} role="treeitem" aria-expanded={open.device}>
          {open.device ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
          <Cpu size={14} strokeWidth={1.75} />
          <span className="tree-row__name">{deviceLabel(info, target)}</span>
          {target.host && <span className="tree-row__meta mono">{target.host}</span>}
          <DeviceMark online={online} mode={mode} />
        </div>
        {open.device && (
          <ul role="group">
            <Leaf depth={1} label="Online & diagnostics" icon={<Activity size={14} strokeWidth={1.75} />}
              active={activePath === ONLINE_PATH} onClick={openOnlineTab} />
            <Group label="Watch tables" icon={<Table2 size={14} strokeWidth={1.75} />} open={open.watch} onToggle={() => toggle("watch")}
              action={
                <IconButton size="sm" label="Add watch table" title="Add watch table" onClick={() => void createWatchTable()}>
                  <Plus size={12} />
                </IconButton>
              }>
              {watchTables.length === 0 && <li className="tree-empty">None yet: add one with +</li>}
              {watchTables.map((w) => (
                <Leaf key={w.path} label={docTitle(w.name)} icon={<Table2 size={14} strokeWidth={1.75} />}
                  active={activePath === w.path} onClick={() => void openDoc(w.path)} />
              ))}
            </Group>
            <Group label="PLC tags and DBs" icon={<Tags size={14} strokeWidth={1.75} />} open={open.tags} onToggle={() => toggle("tags")}>
              {tagTables.length === 0 && <li className="tree-empty">Import a tag table or DB source</li>}
              {tagTables.map((t) => (
                <TagLeaf key={t.path} path={t.path} name={t.name} active={activePath === t.path} />
              ))}
            </Group>
            {online && (
              <Group label="Program blocks (online)" icon={<Blocks size={14} strokeWidth={1.75} />} open={open.blocks} onToggle={() => toggle("blocks")}>
                {!blocks && <li className="tree-empty">Not listed by the CPU</li>}
                {blocks?.map((b) => (
                  <Leaf key={`${b.block_type}${b.block_number}`} label={`${b.block_type}${b.block_number}`}
                    icon={<Blocks size={14} strokeWidth={1.75} />}
                    title={`In the CPU${b.language ? ` (${b.language})` : ""}`}
                    mark={<span className="s7-mark s7-mark--ok" />}
                    onClick={openOnlineTab} />
                ))}
              </Group>
            )}
          </ul>
        )}
      </li>
    </ul>
  );
}

/** A tag table or DB; the icon tells which, read lazily from the file. */
function TagLeaf({ path, name, active }: { path: string; name: string; active: boolean }) {
  const [isDb, setIsDb] = useState(false);
  useEffect(() => {
    let live = true;
    readDoc(path).then((t) => { if (live) setIsDb(/"kind":\s*"db"/.test(t)); }).catch(() => {});
    return () => { live = false; };
  }, [path]);
  const Icon = isDb ? Database : Tags;
  return <Leaf label={docTitle(name)} icon={<Icon size={14} strokeWidth={1.75} />} active={active} onClick={() => void openDoc(path)} />;
}
