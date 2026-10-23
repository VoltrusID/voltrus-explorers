import { Badge, IconButton, ListGroup } from "@mactools/ui";
import { ChartLine } from "lucide-react";
import { Sparkline } from "../Sparkline";
import { fmtValue, hasValue, quality } from "./format";

export interface WatchItem {
  /** `scope|nodeId`: keys the trend series and the sparkline history. */
  key: string;
  nodeId: string;
  name: string;
  dataType: string;
  value: unknown;
  /** Status of the last notification, when there was one. */
  status?: string;
}

interface Props {
  items: WatchItem[];
  history: Record<string, number[]>;
  trending: { has: (key: string) => boolean };
  /** Tap on a row: writes a value. */
  onOpen: (item: WatchItem) => void;
  onToggleTrend: (item: WatchItem) => void;
  /** Inside a card already: a fill behind the rows instead of a second card. */
  flat?: boolean;
}

const accent = () => getComputedStyle(document.documentElement).getPropertyValue("--accent").trim() || "#4f8cff";

/** Monitored values as rows: the value large at the right, a sparkline under it, the quality of the last update as a chip. */
export function WatchList({ items, history, trending, onOpen, onToggleTrend, flat }: Props) {
  return (
    <ListGroup className={flat ? "vm-group--flat" : undefined}>
      {items.map((it) => {
        const q = quality(it.status, it.value);
        const pts = history[it.key];
        const on = trending.has(it.key);
        return (
          <div
            key={it.key}
            role="button"
            tabIndex={0}
            className="vm-row oc-watch"
            onClick={() => onOpen(it)}
            onKeyDown={(e) => {
              if (e.target === e.currentTarget && (e.key === "Enter" || e.key === " ")) {
                e.preventDefault();
                onOpen(it);
              }
            }}
          >
            <div className="oc-watch__text">
              <span className="oc-watch__name">{it.name || it.nodeId}</span>
              <span className="oc-watch__id">{it.nodeId}</span>
              <span className="oc-watch__meta">
                <Badge tone={q.tone}>{q.label}</Badge>
                {it.dataType && <span className="vm-row__subtitle">{it.dataType}</span>}
              </span>
            </div>
            <div className="oc-watch__value">
              <span className={hasValue(it.value) ? "oc-value" : "oc-value oc-value--empty"}>{fmtValue(it.value, it.dataType)}</span>
              {pts && pts.length > 1 && <Sparkline points={pts} color={accent()} width={72} height={22} />}
            </div>
            <IconButton
              outline
              pressed={on}
              label={`${on ? "Remove" : "Add"} ${it.name || it.nodeId} ${on ? "from" : "to"} trend`}
              onClick={(e) => {
                e.stopPropagation();
                onToggleTrend(it);
              }}
            >
              <ChartLine size={18} strokeWidth={1.75} />
            </IconButton>
          </div>
        );
      })}
    </ListGroup>
  );
}
