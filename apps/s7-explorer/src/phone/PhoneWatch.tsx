import { ChartLine, ListPlus, Lock, Plug } from "lucide-react";
import { Badge, Button, EmptyState, IconButton, ListGroup, ListRow, PageTitle } from "@mactools/ui";
import type { TagConfig } from "../types";
import { READ_ONLY_TYPES } from "../types";
import type { S7Value } from "../useS7Device";
import { formatS7Value } from "../format";

interface Props {
  /** Every tag of the device, polled or not. */
  tags: TagConfig[];
  /** How many of them are polled (the free tier polls the first few). */
  polledCount: number;
  values: S7Value[];
  connected: boolean;
  pollCount: number;
  trendNames: { has: (name: string) => boolean };
  freeLimit: number | null;
  onRowClick: (tag: TagConfig, formatted: string) => void;
  onToggleTrend: (name: string) => void;
  onEditTags: () => void;
  onConnect: () => void;
  onPro: () => void;
}

function valueOf(tag: TagConfig, v: S7Value | undefined) {
  if (v === undefined) return <span className="s7-value s7-value--idle">–</span>;
  if (typeof v === "boolean") return <Badge tone={v ? "success" : "neutral"} className="s7-bool">{v ? "ON" : "OFF"}</Badge>;
  const text = formatS7Value(v);
  return (
    <span className="s7-value">
      {text}
      {tag.data_type === "Timer" && typeof v === "number" && <span className="s7-unit"> ms</span>}
    </span>
  );
}

/** The Watch tab: one row per tag with the live value in large type, its address and type, and a trend toggle. */
export default function PhoneWatch({ tags, polledCount, values, connected, pollCount, trendNames, freeLimit, onRowClick, onToggleTrend, onEditTags, onConnect, onPro }: Props) {
  const live = values.length === polledCount;
  const caption =
    tags.length === 0
      ? undefined
      : connected
        ? `${tags.length} ${tags.length === 1 ? "tag" : "tags"} · ${pollCount.toLocaleString()} polls`
        : `${tags.length} ${tags.length === 1 ? "tag" : "tags"} · not connected`;

  return (
    <div className="vm-body">
      <PageTitle
        title="Watch"
        caption={caption}
        actions={
          <Button size="sm" icon={<ListPlus size={16} strokeWidth={1.75} />} onClick={onEditTags}>
            Tags
          </Button>
        }
      />
      {!connected && tags.length > 0 && (
        <div className="s7-note">
          <Plug size={18} strokeWidth={1.75} aria-hidden="true" />
          <span>Connect to the PLC to read these values.</span>
          <Button size="sm" variant="primary" onClick={onConnect}>
            Connect
          </Button>
        </div>
      )}
      {freeLimit !== null && tags.length > freeLimit && (
        <div className="s7-note">
          <Lock size={18} strokeWidth={1.75} aria-hidden="true" />
          <span>
            {freeLimit} of {tags.length} tags are live.
          </span>
          <Button size="sm" onClick={onPro}>
            Pro for more
          </Button>
        </div>
      )}
      {tags.length === 0 ? (
        <EmptyState
          icon={<ListPlus size={22} strokeWidth={1.75} />}
          title="No tags yet"
          description="Add the addresses you want to watch, like DB1.DBD0 or MW50."
          action={
            <Button variant="primary" onClick={onEditTags}>
              Add tags
            </Button>
          }
        />
      ) : (
        <ListGroup>
          {tags.map((t, i) => {
            const polled = i < polledCount;
            const v = connected && polled && live ? values[i] : undefined;
            const readOnly = READ_ONLY_TYPES.includes(t.data_type);
            const active = trendNames.has(t.name);
            return (
              <ListRow
                key={t.id}
                title={t.name}
                subtitle={
                  <span className="s7-sub">
                    <Badge mono>{t.data_type}</Badge>
                    <span className="mono">{t.address}</span>
                    {readOnly && <span>read only</span>}
                    {!polled && <span>not polled</span>}
                  </span>
                }
                value={valueOf(t, v)}
                onClick={() => onRowClick(t, v === undefined ? "" : formatS7Value(v))}
                trailing={
                  <IconButton
                    outline
                    pressed={active}
                    label={active ? `Remove ${t.name} from trend` : `Add ${t.name} to trend`}
                    onClick={(e) => {
                      e.stopPropagation();
                      onToggleTrend(t.name);
                    }}
                  >
                    <ChartLine size={18} strokeWidth={1.75} />
                  </IconButton>
                }
              />
            );
          })}
        </ListGroup>
      )}
    </div>
  );
}
