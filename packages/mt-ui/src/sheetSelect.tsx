import { useMemo, useState, type CSSProperties, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Check, ChevronDown, Search } from "lucide-react";
import { Dialog } from "./primitives";
import { useIsPhone } from "./phone";
import { filterSheetOptions, groupSheetOptions, isSearchable, type SheetOption } from "./sheetOptions";

export type { SheetOption } from "./sheetOptions";

export interface SheetSelectProps<T extends string | number> {
  value: T;
  options: ReadonlyArray<SheetOption<T>>;
  onChange: (value: T) => void;
  /** Title of the sheet on a phone. */
  sheetTitle: string;
  id?: string;
  className?: string;
  style?: CSSProperties;
  disabled?: boolean;
  /** Trigger text when `value` matches no option. */
  placeholder?: string;
  /** Replaces the selected option's label in the trigger (e.g. a label plus an address). */
  display?: ReactNode;
  /** Force the search field on or off; by default it shows with more than ten options. */
  searchable?: boolean;
  /** Extra content under the list (an "add" action). Gets a function that closes the sheet. */
  footer?: (close: () => void) => ReactNode;
  "aria-label"?: string;
  title?: string;
}

/**
 * The one picker of the phone shell. On a phone it is a button that opens the choices as a bottom sheet: a title, a search
 * field when there are more than ten, 56px rows with a checkmark on the current one, dismissed by a tap outside, the close
 * button or a swipe down. Everywhere else (macOS, iPad) it is a native <select>, so the desktop UI does not change.
 */
export function SheetSelect<T extends string | number>(props: SheetSelectProps<T>) {
  const phone = useIsPhone();
  if (!phone) return <NativeSelect {...props} />;
  return <SheetPicker {...props} />;
}

function NativeSelect<T extends string | number>({ value, options, onChange, id, className, style, disabled, placeholder, title, ...rest }: SheetSelectProps<T>) {
  const groups = groupSheetOptions(options);
  const known = options.some((o) => o.value === value);
  const row = (o: SheetOption<T>) => (
    <option key={String(o.value)} value={String(o.value)} disabled={o.disabled}>
      {o.label}
    </option>
  );
  return (
    <select
      id={id}
      className={className ?? "vt-select"}
      style={style}
      value={String(value)}
      disabled={disabled}
      title={title}
      aria-label={rest["aria-label"]}
      onChange={(e) => {
        const next = options.find((o) => String(o.value) === e.target.value);
        if (next) onChange(next.value);
      }}
    >
      {!known && placeholder !== undefined && (
        <option value={String(value)} disabled>
          {placeholder}
        </option>
      )}
      {groups.map((g, i) =>
        g.group ? (
          <optgroup key={`${i}:${g.group}`} label={g.group}>
            {g.items.map(row)}
          </optgroup>
        ) : (
          g.items.map(row)
        ),
      )}
    </select>
  );
}

function SheetPicker<T extends string | number>({
  value,
  options,
  onChange,
  sheetTitle,
  id,
  className,
  style,
  disabled,
  placeholder,
  display,
  searchable,
  footer,
  title,
  ...rest
}: SheetSelectProps<T>) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const selected = options.find((o) => o.value === value);
  const close = () => {
    setOpen(false);
    setQuery("");
  };
  const showSearch = isSearchable(options.length, searchable);
  const groups = useMemo(() => groupSheetOptions(filterSheetOptions(options, showSearch ? query : "")), [options, query, showSearch]);

  const trigger = (
    <button
      type="button"
      id={id}
      className={["vt-select", "vm-select", className].filter(Boolean).join(" ")}
      style={style}
      disabled={disabled}
      aria-haspopup="listbox"
      aria-expanded={open}
      aria-label={rest["aria-label"]}
      title={title}
      data-sheet-select=""
      onClick={() => setOpen(true)}
    >
      <span className="vm-select__value">{display ?? selected?.label ?? placeholder ?? ""}</span>
      <ChevronDown size={16} strokeWidth={2} className="vm-select__chevron" aria-hidden="true" />
    </button>
  );

  if (!open) return trigger;
  return (
    <>
      {trigger}
      {/* In the body, not in the surrounding dialog: a sheet that opens from inside a sheet must not move with it. */}
      {createPortal(
        <Dialog open onClose={close} title={sheetTitle} footer={footer?.(close)}>
          {showSearch && (
            <label className="vm-sheet-search">
              <Search size={18} strokeWidth={1.75} aria-hidden="true" />
              <input
                type="search"
                className="vt-input"
                placeholder="Search"
                aria-label={`Search ${sheetTitle}`}
                autoCapitalize="off"
                autoCorrect="off"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
              />
            </label>
          )}
          <div className="vm-sheet-list" role="listbox" aria-label={sheetTitle}>
            {groups.length === 0 && <div className="vm-sheet-empty">No matches</div>}
            {groups.map((g, i) => (
              <div key={`${i}:${g.group ?? ""}`} role="group" aria-label={g.group}>
                {g.group && <div className="vm-sheet-group">{g.group}</div>}
                <div className="vm-group">
                  {g.items.map((o) => {
                    const current = o.value === value;
                    const choose = () => {
                      if (o.disabled) return;
                      onChange(o.value);
                      close();
                    };
                    return (
                      <div
                        key={String(o.value)}
                        role="option"
                        aria-selected={current}
                        aria-disabled={o.disabled || undefined}
                        tabIndex={o.disabled ? -1 : 0}
                        className={["vm-row", "vm-row--option", current && "vm-row--selected"].filter(Boolean).join(" ")}
                        onClick={choose}
                        onKeyDown={(e) => {
                          if (e.target === e.currentTarget && (e.key === "Enter" || e.key === " ")) {
                            e.preventDefault();
                            choose();
                          }
                        }}
                      >
                        {o.icon && <span className="vm-row__glyph">{o.icon}</span>}
                        <span className="vm-row__text">
                          <span className="vm-row__title">{o.label}</span>
                          {o.hint && <span className="vm-row__subtitle">{o.hint}</span>}
                        </span>
                        {current && <Check size={20} strokeWidth={2.25} className="vm-row__check" aria-hidden="true" />}
                        {o.trailing}
                      </div>
                    );
                  })}
                </div>
              </div>
            ))}
          </div>
        </Dialog>,
        document.body,
      )}
    </>
  );
}
