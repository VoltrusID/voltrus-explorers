import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronLeft, Database, Download, RefreshCw, Trash2 } from "lucide-react";
import { Button, Callout, Dialog, EmptyState, Field, ListGroup, ListRow, PageTitle, Section } from "@mactools/ui";
import { useBlocks, type BlockListEntry } from "../useBlocks";
import { blockKindName, blockTypeOf, byteRows, dbWindowAddresses, groupBlocks } from "./blockGroups";

interface Props {
  deviceId: string;
  connected: boolean;
  onConnect: () => void;
}

const BYTES = 16;

/** The Blocks tab: block types as rows, a type opens its blocks, a block opens a sheet to read it, save it or delete it. */
export default function PhoneBlocks({ deviceId, connected, onConnect }: Props) {
  const b = useBlocks(deviceId, connected);
  const [type, setType] = useState<string | null>(null);
  const [selected, setSelected] = useState<BlockListEntry | null>(null);
  const groups = useMemo(() => groupBlocks(b.blocks), [b.blocks]);
  const group = groups.find((g) => g.type === type);
  // A refresh can remove the type that was open.
  useEffect(() => {
    if (type && !group) setType(null);
  }, [type, group]);

  const refresh = (
    <Button size="sm" icon={<RefreshCw size={16} strokeWidth={1.75} style={b.loading ? { animation: "vt-spin 700ms linear infinite" } : undefined} />} disabled={!connected || b.loading} onClick={b.refreshBlocks}>
      {b.loading ? "Loading…" : "Refresh"}
    </Button>
  );

  return (
    <div className="vm-body">
      {group ? (
        <PageTitle
          title={group.name}
          caption={`${group.blocks.length} ${group.blocks.length === 1 ? "block" : "blocks"}`}
          actions={
            <Button size="sm" icon={<ChevronLeft size={16} strokeWidth={2} />} onClick={() => setType(null)}>
              Blocks
            </Button>
          }
        />
      ) : (
        <PageTitle title="Blocks" caption={connected && b.blocks.length > 0 ? `${b.blocks.length} on the PLC` : "Program blocks on the PLC"} actions={connected ? refresh : undefined} />
      )}

      {b.error && <Callout tone="danger">{b.error}</Callout>}

      {!connected && (
        <EmptyState
          icon={<Database size={22} strokeWidth={1.75} />}
          title="Not connected"
          description="Connect to a PLC to see its blocks."
          action={
            <Button variant="primary" onClick={onConnect}>
              Connect
            </Button>
          }
        />
      )}

      {connected && b.blocks.length === 0 && !b.loading && !b.error && (
        <EmptyState icon={<Database size={22} strokeWidth={1.75} />} title="No blocks found" description="Tap Refresh to scan the PLC again." />
      )}

      {connected && !group && groups.length > 0 && (
        <ListGroup>
          {groups.map((g) => (
            <ListRow key={g.type} icon={<span className="s7-typechip">{g.type}</span>} title={g.name} value={g.blocks.length} chevron onClick={() => setType(g.type)} />
          ))}
        </ListGroup>
      )}

      {connected && group && (
        <ListGroup>
          {group.blocks.map((blk) => (
            <ListRow
              key={`${blk.block_type}-${blk.block_number}`}
              icon={<span className="s7-typechip">{group.type}</span>}
              title={`${group.type} ${blk.block_number}`}
              subtitle={blockKindName(group.type)}
              chevron
              onClick={() => setSelected(blk)}
            />
          ))}
        </ListGroup>
      )}

      {selected && (
        <BlockSheet
          deviceId={deviceId}
          block={selected}
          saving={b.uploading !== null}
          onSave={b.handleUpload}
          onDelete={async (blk) => {
            const gone = await b.deleteBlock(blk);
            if (gone) setSelected(null);
            return gone;
          }}
          onClose={() => setSelected(null)}
        />
      )}
    </div>
  );
}

interface SheetProps {
  deviceId: string;
  block: BlockListEntry;
  saving: boolean;
  onSave: (block: BlockListEntry) => Promise<boolean>;
  onDelete: (block: BlockListEntry) => Promise<boolean>;
  onClose: () => void;
}

function BlockSheet({ deviceId, block, saving, onSave, onDelete, onClose }: SheetProps) {
  const type = blockTypeOf(block);
  const name = `${type} ${block.block_number}`;
  const isDb = type === "DB";
  const [start, setStart] = useState(0);
  const [bytes, setBytes] = useState<unknown[] | null>(null);
  const [readError, setReadError] = useState("");
  const [saved, setSaved] = useState(false);
  const [confirming, setConfirming] = useState(false);

  // Reads one 16 byte window of the data block; paging moves the window.
  useEffect(() => {
    if (!isDb) return;
    let stale = false;
    const timer = setTimeout(async () => {
      try {
        const tags = dbWindowAddresses(block.block_number, start, BYTES).map((address) => ({ address, data_type: "Byte" }));
        const result = await invoke<unknown[]>("read_tags", { deviceId, tags });
        if (!stale) {
          setBytes(result);
          setReadError("");
        }
      } catch (e) {
        if (!stale) {
          setBytes(null);
          setReadError(String(e));
        }
      }
    }, 200);
    return () => {
      stale = true;
      clearTimeout(timer);
    };
  }, [isDb, deviceId, block.block_number, start]);

  return (
    <>
      <Dialog
        open
        onClose={onClose}
        title={name}
        footer={
          <>
            <Button variant="danger" icon={<Trash2 size={16} strokeWidth={1.75} />} onClick={() => setConfirming(true)}>
              Delete from PLC
            </Button>
            <Button
              variant="primary"
              loading={saving}
              icon={<Download size={16} strokeWidth={1.75} />}
              onClick={async () => setSaved(await onSave(block))}
            >
              Save to file
            </Button>
          </>
        }
      >
        <div className="s7-sheet-stack">
          {saved && <Callout tone="success">Saved {name} to a file.</Callout>}
          <ListGroup>
            <ListRow title="Type" value={<span className="s7-plain">{blockKindName(type)}</span>} />
            <ListRow title="Number" value={block.block_number} />
          </ListGroup>
          {isDb && (
            <Section title="Contents" meta={`bytes ${start} to ${start + BYTES - 1}`}>
              <div className="vt-stack vt-rows">
                <Field label="Start byte">
                  {(id) => (
                    <input id={id} className="vt-input vt-input--mono" type="number" inputMode="numeric" min={0} value={start} onChange={(e) => setStart(Math.max(0, Math.round(Number(e.target.value) || 0)))} />
                  )}
                </Field>
              </div>
              {readError && <Callout tone="danger">{readError}</Callout>}
              {bytes && (
                <ListGroup className="vm-group--flat">
                  {byteRows(start, bytes).map(([offset, hex]) => (
                    <ListRow key={offset} title={<span className="mono">DBB{offset}</span>} value={<span className="s7-hex">{hex}</span>} />
                  ))}
                </ListGroup>
              )}
              <div className="s7-pager">
                <Button disabled={start === 0} onClick={() => setStart(Math.max(0, start - BYTES))}>
                  Previous 16
                </Button>
                <Button onClick={() => setStart(start + BYTES)}>Next 16</Button>
              </div>
            </Section>
          )}
        </div>
      </Dialog>
      {confirming && (
        <Dialog
          open
          onClose={() => setConfirming(false)}
          title={`Delete ${name}?`}
          footer={
            <>
              <Button onClick={() => setConfirming(false)}>Cancel</Button>
              <Button
                variant="danger"
                onClick={async () => {
                  setConfirming(false);
                  await onDelete(block);
                }}
              >
                Delete {name}
              </Button>
            </>
          }
        >
          <div className="s7-copy">
            <p>This removes the block from the PLC. It cannot be undone.</p>
          </div>
        </Dialog>
      )}
    </>
  );
}
