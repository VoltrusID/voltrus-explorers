import { useState, useSyncExternalStore, type ReactNode } from "react";
import { Check, ChevronRight, Ellipsis } from "lucide-react";
import { Dialog } from "./primitives";

/*
 * Phone shell: the iPhone layout of Modbus Poll and BACnet Explorer.
 *
 * It is opt-in. An app calls `usePhoneShell()` once; below 768 CSS px that sets `data-shell="phone"` on <html>, which
 * switches on the phone skin in ./mobile.css (grouped inset cards, 16px+ inputs, 44pt targets, safe-area insets, bottom
 * sheets), and makes `useIsPhone()` true so the shared Toolbar renders the compact header. Apps that never call the hook
 * (the other explorers) keep the desktop look at every width.
 *
 * Design source: JoyoFit (watch-fit): large page titles, rounded grouped cards on a quieter page, one primary pill button,
 * a pill tab bar with icons and labels tucked into the home-indicator inset.
 */

const PHONE_QUERY = "(max-width: 767px)";

let phone = false;
let query: MediaQueryList | null = null;
const listeners = new Set<() => void>();

function apply(next: boolean) {
  if (next === phone) return;
  phone = next;
  if (next) document.documentElement.setAttribute("data-shell", "phone");
  else document.documentElement.removeAttribute("data-shell");
  listeners.forEach((l) => l());
}

function ensureQuery() {
  if (query || typeof window === "undefined") return;
  query = window.matchMedia(PHONE_QUERY);
  apply(query.matches);
  query.addEventListener("change", (e) => apply(e.matches));
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

const snapshot = () => phone;

/** Turns the phone shell on for this app. True below 768 CSS px; the page follows the viewport if it is resized. */
export function usePhoneShell(): boolean {
  ensureQuery();
  return useSyncExternalStore(subscribe, snapshot, () => false);
}

/** Whether the phone shell is showing. Always false in an app that does not call `usePhoneShell`. */
export function useIsPhone(): boolean {
  return useSyncExternalStore(subscribe, snapshot, () => false);
}

function cx(...parts: Array<string | false | null | undefined>): string {
  return parts.filter(Boolean).join(" ");
}

/* ── Connection chip (header) ────────────────────────────────────────────── */

export interface StatusChipProps {
  connected: boolean;
  /** Overrides "Connected" / "Disconnected" (e.g. "Server running"). */
  label?: string;
  /** Polls so far; exposed as `data-polls` for the screenshot harness and tests. */
  polls?: number;
}

export function StatusChip({ connected, label, polls }: StatusChipProps) {
  return (
    <span className={cx("vm-chip", connected && "vm-chip--on")} role="status" data-connected={connected} data-polls={polls}>
      <span className={cx("vt-dot", connected && "vt-dot--success")} aria-hidden="true" />
      {label ?? (connected ? "Connected" : "Disconnected")}
    </span>
  );
}

/* ── Page title ──────────────────────────────────────────────────────────── */

export interface PageTitleProps {
  title: ReactNode;
  caption?: ReactNode;
  /** One action or a few, right of the title. */
  actions?: ReactNode;
}

/** The 28px title at the top of a tab's page, with an optional caption and actions (JoyoFit `PageHeader`). */
export function PageTitle({ title, caption, actions }: PageTitleProps) {
  return (
    <div className="vm-title">
      <div className="vm-title__text">
        <h1 className="vm-title__h">{title}</h1>
        {caption && <p className="vm-title__caption">{caption}</p>}
      </div>
      {actions && <div className="vm-title__actions">{actions}</div>}
    </div>
  );
}

/* ── Grouped list ────────────────────────────────────────────────────────── */

/** One rounded card holding `ListRow`s with hairlines between them. */
export function ListGroup({ children, className }: { children: ReactNode; className?: string }) {
  return <div className={cx("vm-group", className)}>{children}</div>;
}

export interface ListRowProps {
  icon?: ReactNode;
  title: ReactNode;
  subtitle?: ReactNode;
  /** Value at the right, before the trailing control. */
  value?: ReactNode;
  trailing?: ReactNode;
  /** Draws a chevron; the row is tappable when `onClick` is set. */
  chevron?: boolean;
  selected?: boolean;
  onClick?: () => void;
  className?: string;
}

export function ListRow({ icon, title, subtitle, value, trailing, chevron, selected, onClick, className }: ListRowProps) {
  const body = (
    <>
      {icon && <span className="vm-row__icon">{icon}</span>}
      <span className="vm-row__text">
        <span className="vm-row__title">{title}</span>
        {subtitle && <span className="vm-row__subtitle">{subtitle}</span>}
      </span>
      {value !== undefined && <span className="vm-row__value">{value}</span>}
      {selected && <Check size={18} strokeWidth={2.25} className="vm-row__check" aria-hidden="true" />}
      {trailing}
      {chevron && <ChevronRight size={18} strokeWidth={1.75} className="vm-row__chevron" aria-hidden="true" />}
    </>
  );
  // A div, not a button: `trailing` may hold buttons of its own, and buttons do not nest.
  return onClick ? (
    <div
      role="button"
      tabIndex={0}
      className={cx("vm-row", selected && "vm-row--selected", className)}
      onClick={onClick}
      onKeyDown={(e) => {
        if (e.target === e.currentTarget && (e.key === "Enter" || e.key === " ")) {
          e.preventDefault();
          onClick();
        }
      }}
    >
      {body}
    </div>
  ) : (
    <div className={cx("vm-row", className)}>{body}</div>
  );
}

/* ── Bottom tab bar ──────────────────────────────────────────────────────── */

export interface PhoneTabItem<T extends string = string> {
  id: T;
  /** One short word: it sits under the icon and has to fit a fifth of the screen. */
  label: string;
  icon: ReactNode;
  /** Small count on the icon (e.g. trend series). */
  badge?: number;
  /** Small dot on the icon (e.g. a server is running). */
  dot?: boolean;
}

export interface PhoneTabBarProps<T extends string> {
  tabs: ReadonlyArray<PhoneTabItem<T>>;
  active: T;
  onChange: (tab: T) => void;
  /** Most items drawn at once, "More" included. iOS draws five. */
  maxVisible?: number;
  moreLabel?: string;
}

/**
 * Pill tab bar: icons with short labels, at most `maxVisible` items. With more tabs than that, the last ones move into a
 * "More" item that opens a sheet. It sits in the flow at the bottom of the screen and tucks into the home-indicator inset
 * (the indicator is a thin line, so the full inset would leave an empty band).
 */
export function PhoneTabBar<T extends string>({ tabs, active, onChange, maxVisible = 5, moreLabel = "More" }: PhoneTabBarProps<T>) {
  const [moreOpen, setMoreOpen] = useState(false);
  const overflowing = tabs.length > maxVisible;
  const visible = overflowing ? tabs.slice(0, maxVisible - 1) : tabs;
  const overflow = overflowing ? tabs.slice(maxVisible - 1) : [];
  const moreActive = overflow.some((t) => t.id === active);

  const item = (t: PhoneTabItem<T>) => (
    <button
      key={t.id}
      type="button"
      role="tab"
      aria-selected={active === t.id}
      className="vm-tab"
      onClick={() => onChange(t.id)}
    >
      {/* Label first in the DOM (it names the tab, and the badge digits must not prefix it); drawn under the icon. */}
      <span className="vm-tab__label">{t.label}</span>
      <span className="vm-tab__icon">
        {t.icon}
        {t.badge ? <span className="vm-tab__badge">{t.badge}</span> : null}
        {t.dot ? <span className="vm-tab__dot" aria-hidden="true" /> : null}
      </span>
    </button>
  );

  return (
    <nav className="vm-tabbar" aria-label="Sections">
      <div className="vm-tabbar__pill" role="tablist">
        {visible.map(item)}
        {overflowing && (
          <button
            type="button"
            role="tab"
            aria-selected={moreActive}
            aria-haspopup="dialog"
            className="vm-tab"
            onClick={() => setMoreOpen(true)}
          >
            <span className="vm-tab__label">{moreLabel}</span>
            <span className="vm-tab__icon">
              <Ellipsis size={22} strokeWidth={2} />
              {overflow.some((t) => t.badge) ? <span className="vm-tab__dot" aria-hidden="true" /> : null}
            </span>
          </button>
        )}
      </div>
      {overflowing && (
        <Dialog open={moreOpen} onClose={() => setMoreOpen(false)} title={moreLabel}>
          <ListGroup>
            {overflow.map((t) => (
              <ListRow
                key={t.id}
                icon={t.icon}
                title={t.label}
                value={t.badge ? t.badge : undefined}
                selected={active === t.id}
                chevron={active !== t.id}
                onClick={() => {
                  setMoreOpen(false);
                  onChange(t.id);
                }}
              />
            ))}
          </ListGroup>
        </Dialog>
      )}
    </nav>
  );
}
