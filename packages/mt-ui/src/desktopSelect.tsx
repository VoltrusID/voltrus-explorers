import { useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import * as SelectPrimitive from "@radix-ui/react-select";
import { Command } from "cmdk";
import { Check, ChevronDown, ChevronUp, Search } from "lucide-react";
import { Popover, PopoverContent, PopoverTrigger } from "./popover";
import { filterSheetOptions, groupSheetOptions, isSearchable, type SheetOption } from "./sheetOptions";
import type { SheetSelectProps } from "./sheetSelect";

/*
 * The desktop pickers behind `SheetSelect`, styled after shadcn/ui (overlay.css):
 *
 *  - Select: Radix Select. A trigger with a chevron, a bordered popover, a highlighted row under the pointer or the
 *    arrow keys, an accent checkmark on the chosen row, group labels, separators and scroll buttons. Typing jumps to
 *    the row whose label starts with the typed text; Enter or Space picks, Escape closes with focus back on the trigger.
 *  - Combobox: Radix Popover with a cmdk list. The same look plus a search field on top that filters by label,
 *    description and group as you type, "No results", arrow keys + Enter, and an optional footer action. Used for
 *    lists of more than SEARCH_THRESHOLD options and whenever there is a footer (Radix Select has no way to reach one
 *    from the keyboard).
 */

const cx = (...parts: Array<string | false | null | undefined>) => parts.filter(Boolean).join(" ");

/** Radix Select forbids an empty-string value, and cmdk trims and lowercases; a prefix keeps every option value legal. */
const enc = (v: string | number) => `v:${v}`;

type PickerProps<T extends string | number> = SheetSelectProps<T>;

function triggerClass<T extends string | number>({ className, size, fullWidth }: PickerProps<T>) {
  return cx("vt-select", "vt-select-trigger", size === "sm" && "vt-select--sm", fullWidth === false && "vt-select--auto", className);
}

function TriggerValue<T extends string | number>({ display, selected, placeholder }: { display?: ReactNode; selected?: SheetOption<T>; placeholder?: string }) {
  const shown = display ?? (selected ? selected.label : placeholder);
  const empty = display === undefined && !selected;
  return (
    <span className="vt-trigger__value" data-placeholder={empty || undefined}>
      {display === undefined && selected?.icon && <span className="vt-option__icon">{selected.icon}</span>}
      {shown}
    </span>
  );
}

function OptionText<T extends string | number>({ option, label }: { option: SheetOption<T>; label?: ReactNode }) {
  return (
    <>
      {option.icon && <span className="vt-option__icon">{option.icon}</span>}
      <span className="vt-option__text">
        <span className="vt-option__label">{label ?? option.label}</span>
        {option.hint && <span className="vt-option__hint">{option.hint}</span>}
      </span>
    </>
  );
}

/* ── Select ──────────────────────────────────────────────────────────────── */

export function Select<T extends string | number>(props: PickerProps<T>) {
  const { value, options, onChange, id, style, placeholder, display, title } = props;
  // Nothing to choose from (no saved profiles yet): the trigger is dimmed instead of opening an empty box.
  const disabled = props.disabled || options.length === 0;
  const selected = options.find((o) => o.value === value);
  const groups = useMemo(() => groupSheetOptions(options), [options]);
  const row = (o: SheetOption<T>) => (
    <SelectPrimitive.Item
      key={String(o.value)}
      value={enc(o.value)}
      textValue={o.label}
      disabled={o.disabled}
      className={cx("vt-menu-item", "vt-option", o.hint && "vt-option--described")}
    >
      <OptionText option={o} label={<SelectPrimitive.ItemText>{o.label}</SelectPrimitive.ItemText>} />
      <SelectPrimitive.ItemIndicator className="vt-option__check">
        <Check size={14} strokeWidth={2.25} aria-hidden="true" />
      </SelectPrimitive.ItemIndicator>
    </SelectPrimitive.Item>
  );
  return (
    <SelectPrimitive.Root
      value={selected ? enc(selected.value) : ""}
      onValueChange={(v) => {
        const next = options.find((o) => enc(o.value) === v);
        if (next) onChange(next.value);
      }}
      disabled={disabled}
    >
      <SelectPrimitive.Trigger id={id} className={triggerClass(props)} style={style} title={title} aria-label={props["aria-label"]}>
        <TriggerValue display={display} selected={selected} placeholder={placeholder} />
        <SelectPrimitive.Icon asChild>
          <ChevronDown size={14} strokeWidth={2} className="vt-trigger__chevron" aria-hidden="true" />
        </SelectPrimitive.Icon>
      </SelectPrimitive.Trigger>
      <SelectPrimitive.Portal>
        <SelectPrimitive.Content position="popper" sideOffset={4} collisionPadding={8} className="vt-popover vt-select-content">
          <SelectPrimitive.ScrollUpButton className="vt-scroll-btn">
            <ChevronUp size={14} strokeWidth={2} aria-hidden="true" />
          </SelectPrimitive.ScrollUpButton>
          <SelectPrimitive.Viewport>
            {groups.map((g, i) => (
              <div key={`${i}:${g.group ?? ""}`}>
                {i > 0 && <SelectPrimitive.Separator className="vt-menu-sep" />}
                {g.group ? (
                  <SelectPrimitive.Group>
                    <SelectPrimitive.Label className="vt-menu-label">{g.group}</SelectPrimitive.Label>
                    {g.items.map(row)}
                  </SelectPrimitive.Group>
                ) : (
                  g.items.map(row)
                )}
              </div>
            ))}
          </SelectPrimitive.Viewport>
          <SelectPrimitive.ScrollDownButton className="vt-scroll-btn">
            <ChevronDown size={14} strokeWidth={2} aria-hidden="true" />
          </SelectPrimitive.ScrollDownButton>
        </SelectPrimitive.Content>
      </SelectPrimitive.Portal>
    </SelectPrimitive.Root>
  );
}

/* ── Combobox ────────────────────────────────────────────────────────────── */

export function Combobox<T extends string | number>(props: PickerProps<T>) {
  const { value, options, onChange, sheetTitle, id, style, placeholder, display, searchable, footer, title } = props;
  const disabled = props.disabled || (options.length === 0 && !footer);
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const [highlighted, setHighlighted] = useState("");
  const selected = options.find((o) => o.value === value);
  const showSearch = isSearchable(options.length, searchable);
  const shown = useMemo(() => (showSearch ? filterSheetOptions(options, query) : [...options]), [options, query, showSearch]);
  const groups = useMemo(() => groupSheetOptions(shown), [shown]);
  const listRef = useRef<HTMLDivElement>(null);

  const startAt = () => {
    const first = options.find((o) => !o.disabled);
    return selected ? enc(selected.value) : first ? enc(first.value) : "";
  };
  const openWith = (seed: string) => {
    setQuery(seed);
    setHighlighted(startAt());
    setOpen(true);
  };
  const close = () => setOpen(false);
  const pick = (o: SheetOption<T>) => {
    if (o.disabled) return;
    onChange(o.value);
    close();
  };

  // Centre the chosen row when the list opens (not scrollIntoView: it would also scroll the page).
  useLayoutEffect(() => {
    if (!open) return;
    const list = listRef.current;
    const item = list?.querySelector<HTMLElement>('[data-checked="true"]');
    if (list && item) list.scrollTop = item.offsetTop - (list.clientHeight - item.offsetHeight) / 2;
  }, [open]);

  // Without a search field the input is invisible and only jumps to the row that starts with what was typed.
  useEffect(() => {
    if (!open || showSearch || !query) return;
    const q = query.toLowerCase();
    const hit = options.find((o) => !o.disabled && o.label.toLowerCase().startsWith(q));
    if (hit) setHighlighted(enc(hit.value));
    const timer = window.setTimeout(() => setQuery(""), 700);
    return () => window.clearTimeout(timer);
  }, [open, showSearch, query, options]);

  const onTriggerKeyDown = (e: KeyboardEvent<HTMLButtonElement>) => {
    if (disabled || open) return;
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      openWith("");
    } else if (e.key.length === 1 && e.key !== " " && !e.ctrlKey && !e.metaKey && !e.altKey) {
      e.preventDefault();
      openWith(e.key);
    }
  };

  return (
    <Popover
      open={open}
      onOpenChange={(next) => {
        if (next) openWith("");
        else {
          setOpen(false);
          setQuery("");
        }
      }}
    >
      <PopoverTrigger asChild>
        <button
          type="button"
          id={id}
          role="combobox"
          aria-haspopup="listbox"
          aria-expanded={open}
          className={triggerClass(props)}
          style={style}
          title={title}
          disabled={disabled}
          aria-label={props["aria-label"]}
          onKeyDown={onTriggerKeyDown}
        >
          <TriggerValue display={display} selected={selected} placeholder={placeholder} />
          <ChevronDown size={14} strokeWidth={2} className="vt-trigger__chevron" aria-hidden="true" />
        </button>
      </PopoverTrigger>
      <PopoverContent className="vt-combo" aria-label={sheetTitle}>
        <Command shouldFilter={false} loop value={highlighted} onValueChange={setHighlighted} label={sheetTitle}>
          {showSearch ? (
            <div className="vt-combo__search">
              <Search size={14} strokeWidth={2} aria-hidden="true" />
              <Command.Input value={query} onValueChange={setQuery} placeholder="Search…" aria-label={`Search ${sheetTitle}`} autoCapitalize="off" autoCorrect="off" spellCheck={false} />
            </div>
          ) : (
            <input
              className="vt-combo__search--hidden"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              aria-label={sheetTitle}
              autoCapitalize="off"
              autoCorrect="off"
              spellCheck={false}
            />
          )}
          <Command.List ref={listRef} aria-label={sheetTitle}>
            {groups.length === 0 && <div className="vt-combo__empty" role="presentation">No results</div>}
            {groups.map((g, i) => {
              const items = g.items.map((o) => {
                const current = o.value === value;
                return (
                  <Command.Item
                    key={String(o.value)}
                    value={enc(o.value)}
                    disabled={o.disabled}
                    onSelect={() => pick(o)}
                    data-checked={current || undefined}
                    className={cx("vt-menu-item", "vt-option", o.hint && "vt-option--described")}
                  >
                    <OptionText option={o} />
                    {current && (
                      <span className="vt-option__check">
                        <Check size={14} strokeWidth={2.25} aria-hidden="true" />
                      </span>
                    )}
                  </Command.Item>
                );
              });
              return g.group ? (
                <Command.Group key={`${i}:${g.group}`} heading={g.group}>
                  {items}
                </Command.Group>
              ) : (
                <Command.Group key={`${i}:`}>{items}</Command.Group>
              );
            })}
          </Command.List>
        </Command>
        {footer && <div className="vt-combo__footer">{footer(close)}</div>}
      </PopoverContent>
    </Popover>
  );
}
