import {
  forwardRef,
  useEffect,
  useId,
  useRef,
  type ButtonHTMLAttributes,
  type HTMLAttributes,
  type KeyboardEvent,
  type ReactNode,
  type TouchEvent,
} from "react";
import { X } from "lucide-react";

function cx(...parts: Array<string | false | null | undefined>): string {
  return parts.filter(Boolean).join(" ");
}

/* ── Button ──────────────────────────────────────────────────────────────── */

export type ButtonVariant = "default" | "primary" | "danger" | "ghost";
export type ButtonSize = "sm" | "md" | "lg";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  block?: boolean;
  /** Leading icon element, e.g. <Play size={14} />. */
  icon?: ReactNode;
  /** Replaces the icon with a spinner and disables the button. */
  loading?: boolean;
}

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = "default", size = "md", block, icon, loading, className, children, disabled, type = "button", ...rest },
  ref,
) {
  return (
    <button
      ref={ref}
      type={type}
      disabled={disabled || loading}
      aria-busy={loading || undefined}
      className={cx(
        "vt-btn",
        variant !== "default" && `vt-btn--${variant}`,
        size !== "md" && `vt-btn--${size}`,
        block && "vt-btn--block",
        className,
      )}
      {...rest}
    >
      {loading ? <span className="vt-spinner" aria-hidden="true" /> : icon}
      {children}
    </button>
  );
});

export interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  /** Required: icon-only controls need an accessible name. */
  label: string;
  size?: "sm" | "md";
  outline?: boolean;
  danger?: boolean;
  pressed?: boolean;
}

export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(function IconButton(
  { label, size = "md", outline, danger, pressed, className, children, type = "button", title, ...rest },
  ref,
) {
  return (
    <button
      ref={ref}
      type={type}
      aria-label={label}
      title={title ?? label}
      aria-pressed={pressed}
      className={cx(
        "vt-icon-btn",
        size === "sm" && "vt-icon-btn--sm",
        outline && "vt-icon-btn--outline",
        danger && "vt-icon-btn--danger",
        className,
      )}
      {...rest}
    >
      {children}
    </button>
  );
});

/* ── Field ───────────────────────────────────────────────────────────────── */

export interface FieldProps {
  label: ReactNode;
  /** id of the control; generated when omitted and passed to a single child via render prop. */
  htmlFor?: string;
  hint?: ReactNode;
  error?: ReactNode;
  /** On phones a field in a grouped section is a row (label left, value right); this keeps the label above the control. */
  stacked?: boolean;
  className?: string;
  children: ReactNode | ((id: string) => ReactNode);
}

export function Field({ label, htmlFor, hint, error, stacked, className, children }: FieldProps) {
  const auto = useId();
  const id = htmlFor ?? auto;
  return (
    <div className={cx("vt-field", stacked && "vt-field--stacked", className)}>
      <label className="vt-label" htmlFor={id}>
        {label}
      </label>
      {typeof children === "function" ? children(id) : children}
      {error ? (
        <span className="vt-hint vt-hint--error" role="alert">
          {error}
        </span>
      ) : hint ? (
        <span className="vt-hint">{hint}</span>
      ) : null}
    </div>
  );
}

/* ── Segmented control ───────────────────────────────────────────────────── */

export interface SegmentedOption<T extends string> {
  value: T;
  label: ReactNode;
  icon?: ReactNode;
  disabled?: boolean;
  title?: string;
}

export interface SegmentedProps<T extends string> {
  value: T;
  options: ReadonlyArray<SegmentedOption<T>>;
  onChange: (value: T) => void;
  /** Accessible name for the group. */
  label: string;
  size?: "sm" | "md";
  block?: boolean;
  className?: string;
}

export function Segmented<T extends string>({
  value,
  options,
  onChange,
  label,
  size = "md",
  block,
  className,
}: SegmentedProps<T>) {
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
    const enabled = options.filter((o) => !o.disabled);
    const i = enabled.findIndex((o) => o.value === value);
    const next = enabled[(i + (e.key === "ArrowRight" ? 1 : enabled.length - 1)) % enabled.length];
    if (next) {
      e.preventDefault();
      onChange(next.value);
    }
  };
  return (
    <div
      role="radiogroup"
      aria-label={label}
      onKeyDown={onKey}
      className={cx("vt-segmented", size === "sm" && "vt-segmented--sm", block && "vt-segmented--block", className)}
    >
      {options.map((o) => {
        const checked = o.value === value;
        return (
          <button
            key={o.value}
            type="button"
            role="radio"
            aria-checked={checked}
            tabIndex={checked ? 0 : -1}
            disabled={o.disabled}
            title={o.title}
            className="vt-segmented__item"
            onClick={() => onChange(o.value)}
          >
            {o.icon}
            {o.label}
          </button>
        );
      })}
    </div>
  );
}

/* ── Tabs ────────────────────────────────────────────────────────────────── */

export interface TabItem<T extends string> {
  value: T;
  label: ReactNode;
  count?: number;
  icon?: ReactNode;
}

export interface TabsProps<T extends string> {
  value: T;
  items: ReadonlyArray<TabItem<T>>;
  onChange: (value: T) => void;
  label: string;
  /** Extra content pinned to the right end of the tab strip. */
  trailing?: ReactNode;
  className?: string;
}

export function Tabs<T extends string>({ value, items, onChange, label, trailing, className }: TabsProps<T>) {
  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "ArrowRight" && e.key !== "ArrowLeft") return;
    const i = items.findIndex((t) => t.value === value);
    const next = items[(i + (e.key === "ArrowRight" ? 1 : items.length - 1)) % items.length];
    if (next) {
      e.preventDefault();
      onChange(next.value);
    }
  };
  return (
    <div role="tablist" aria-label={label} onKeyDown={onKey} className={cx("vt-tabs", className)}>
      {items.map((t) => {
        const selected = t.value === value;
        return (
          <button
            key={t.value}
            type="button"
            role="tab"
            aria-selected={selected}
            tabIndex={selected ? 0 : -1}
            className="vt-tab"
            onClick={() => onChange(t.value)}
          >
            {t.icon}
            {t.label}
            {t.count !== undefined && <span className="vt-tab__count">{t.count}</span>}
          </button>
        );
      })}
      {trailing && <div style={{ marginLeft: "auto", display: "flex", alignItems: "center", gap: 4 }}>{trailing}</div>}
    </div>
  );
}

/* ── Section / Panel ─────────────────────────────────────────────────────── */

export interface SectionProps extends Omit<HTMLAttributes<HTMLElement>, "title"> {
  title?: ReactNode;
  /** Small trailing text next to the title, e.g. a count. */
  meta?: ReactNode;
  /** Controls aligned right in the header row. */
  actions?: ReactNode;
}

export function Section({ title, meta, actions, className, children, ...rest }: SectionProps) {
  const headingId = useId();
  return (
    <section aria-labelledby={title ? headingId : undefined} className={cx("vt-section", className)} {...rest}>
      {(title || actions) && (
        <div className="vt-section__header">
          <div style={{ display: "flex", alignItems: "baseline", gap: 6, minWidth: 0 }}>
            {title && (
              <h2 id={headingId} className="vt-section__title">
                {title}
              </h2>
            )}
            {meta !== undefined && <span className="vt-section__meta">{meta}</span>}
          </div>
          {actions && <div style={{ display: "flex", alignItems: "center", gap: 4 }}>{actions}</div>}
        </div>
      )}
      <div className="vt-section__body">{children}</div>
    </section>
  );
}

export interface PanelProps extends Omit<HTMLAttributes<HTMLDivElement>, "title"> {
  title?: ReactNode;
  actions?: ReactNode;
}

export function Panel({ title, actions, className, children, ...rest }: PanelProps) {
  return (
    <div className={cx("vt-panel", className)} {...rest}>
      {(title || actions) && (
        <div className="vt-panel__header">
          {title && <span className="vt-panel__title">{title}</span>}
          {actions}
        </div>
      )}
      {children}
    </div>
  );
}

/* ── Empty state ─────────────────────────────────────────────────────────── */

export interface EmptyStateProps {
  icon?: ReactNode;
  title: ReactNode;
  /** Why it is empty and what fills it. */
  description?: ReactNode;
  action?: ReactNode;
  compact?: boolean;
  className?: string;
}

export function EmptyState({ icon, title, description, action, compact, className }: EmptyStateProps) {
  return (
    <div className={cx("vt-empty", compact && "vt-empty--compact", className)}>
      {icon && <div className="vt-empty__icon">{icon}</div>}
      <div className="vt-empty__title">{title}</div>
      {description && <div className="vt-empty__desc">{description}</div>}
      {action && <div className="vt-empty__action">{action}</div>}
    </div>
  );
}

/* ── Badge / StatusDot / Kbd / Callout ───────────────────────────────────── */

export type Tone = "neutral" | "accent" | "success" | "warn" | "danger" | "info";

export interface BadgeProps extends HTMLAttributes<HTMLSpanElement> {
  tone?: Tone;
  mono?: boolean;
}

export function Badge({ tone = "neutral", mono, className, ...rest }: BadgeProps) {
  return (
    <span
      className={cx("vt-badge", tone !== "neutral" && `vt-badge--${tone}`, mono && "vt-badge--mono", className)}
      {...rest}
    />
  );
}

export function StatusDot({ tone = "neutral", label }: { tone?: Tone; label?: string }) {
  return (
    <span
      className={cx("vt-dot", tone !== "neutral" && tone !== "accent" && `vt-dot--${tone}`)}
      role={label ? "img" : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
    />
  );
}

export function Kbd({ children }: { children: ReactNode }) {
  return <kbd className="vt-kbd">{children}</kbd>;
}

export interface CalloutProps extends HTMLAttributes<HTMLDivElement> {
  tone?: Exclude<Tone, "accent">;
  icon?: ReactNode;
}

export function Callout({ tone = "neutral", icon, className, children, ...rest }: CalloutProps) {
  return (
    <div
      role={tone === "danger" ? "alert" : undefined}
      className={cx("vt-callout", tone !== "neutral" && `vt-callout--${tone}`, className)}
      {...rest}
    >
      {icon}
      <div style={{ minWidth: 0 }}>{children}</div>
    </div>
  );
}

/* ── Dialog ──────────────────────────────────────────────────────────────── */

export interface DialogProps {
  open: boolean;
  onClose: () => void;
  title: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  width?: number;
}

/** Open dialogs, innermost last; Escape only closes the innermost one. */
const dialogStack: object[] = [];

export function Dialog({ open, onClose, title, children, footer, width }: DialogProps) {
  const ref = useRef<HTMLDivElement>(null);
  const titleId = useId();
  // Consumers pass inline closures; reading through a ref keeps the effect
  // from re-running (and moving focus) on every parent render.
  const onCloseRef = useRef(onClose);
  onCloseRef.current = onClose;
  useEffect(() => {
    if (!open) return;
    const token = {};
    dialogStack.push(token);
    const prev = document.activeElement as HTMLElement | null;
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === "Escape" && dialogStack[dialogStack.length - 1] === token) onCloseRef.current();
    };
    window.addEventListener("keydown", onKey);
    if (!ref.current?.contains(document.activeElement)) ref.current?.focus();
    return () => {
      window.removeEventListener("keydown", onKey);
      dialogStack.splice(dialogStack.indexOf(token), 1);
      prev?.focus?.();
    };
  }, [open]);
  // Phone sheets: a swipe down on the header (or the grabber above it) follows the finger and closes the sheet past 100px,
  // or past 40px when fast; a shorter drag springs back.
  const drag = useRef<{ y: number; dy: number; t: number } | null>(null);
  const onTouchStart = (e: TouchEvent<HTMLDivElement>) => {
    if (document.documentElement.dataset.shell !== "phone") return;
    const target = e.target as HTMLElement;
    if (target !== ref.current && !target.closest(".vt-dialog__header")) return;
    drag.current = { y: e.touches[0].clientY, dy: 0, t: e.timeStamp };
  };
  const onTouchMove = (e: TouchEvent<HTMLDivElement>) => {
    const d = drag.current;
    const el = ref.current;
    if (!d || !el) return;
    d.dy = Math.max(0, e.touches[0].clientY - d.y);
    el.style.transition = "none";
    el.style.transform = `translateY(${d.dy}px)`;
  };
  const onTouchEnd = (e: TouchEvent<HTMLDivElement>) => {
    const d = drag.current;
    drag.current = null;
    const el = ref.current;
    if (!d || !el) return;
    if (d.dy > 100 || (d.dy > 40 && e.timeStamp - d.t < 250)) {
      onClose();
      return;
    }
    el.style.transition = "transform 200ms var(--ease-out)";
    el.style.transform = "";
  };
  if (!open) return null;
  return (
    <div className="vt-scrim" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div
        ref={ref}
        onTouchStart={onTouchStart}
        onTouchMove={onTouchMove}
        onTouchEnd={onTouchEnd}
        onTouchCancel={onTouchEnd}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        className="vt-dialog"
        style={width ? { width: `min(${width}px, 100%)` } : undefined}
      >
        <div className="vt-dialog__header">
          <h2 id={titleId} className="vt-dialog__title">
            {title}
          </h2>
          <IconButton label="Close" size="sm" onClick={onClose}>
            <X size={14} />
          </IconButton>
        </div>
        <div className="vt-dialog__body">{children}</div>
        {footer && <div className="vt-dialog__footer">{footer}</div>}
      </div>
    </div>
  );
}
