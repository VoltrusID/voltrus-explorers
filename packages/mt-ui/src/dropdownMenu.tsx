import { forwardRef, type ComponentPropsWithoutRef, type ElementRef, type ReactNode } from "react";
import * as MenuPrimitive from "@radix-ui/react-dropdown-menu";

/*
 * DropdownMenu: the action menu behind a button or an icon button (Radix DropdownMenu: arrow keys, Home/End, type-ahead,
 * Enter/Space to run an item, Escape to close with focus back on the trigger). Compose like shadcn/ui:
 *
 *   <DropdownMenu>
 *     <DropdownMenuTrigger asChild><IconButton label="Export">...</IconButton></DropdownMenuTrigger>
 *     <DropdownMenuContent align="end">
 *       <DropdownMenuItem icon={<Copy size={14} />} onSelect={copy}>Copy all</DropdownMenuItem>
 *     </DropdownMenuContent>
 *   </DropdownMenu>
 */
export const DropdownMenu = MenuPrimitive.Root;
export const DropdownMenuTrigger = MenuPrimitive.Trigger;
export const DropdownMenuGroup = MenuPrimitive.Group;

export const DropdownMenuContent = forwardRef<ElementRef<typeof MenuPrimitive.Content>, ComponentPropsWithoutRef<typeof MenuPrimitive.Content>>(
  function DropdownMenuContent({ className, sideOffset = 4, collisionPadding = 8, ...rest }, ref) {
    return (
      <MenuPrimitive.Portal>
        <MenuPrimitive.Content
          ref={ref}
          sideOffset={sideOffset}
          collisionPadding={collisionPadding}
          className={["vt-popover", className].filter(Boolean).join(" ")}
          {...rest}
        />
      </MenuPrimitive.Portal>
    );
  },
);

export interface DropdownMenuItemProps extends ComponentPropsWithoutRef<typeof MenuPrimitive.Item> {
  /** Glyph before the label (a 14px lucide icon). */
  icon?: ReactNode;
  danger?: boolean;
}

export const DropdownMenuItem = forwardRef<ElementRef<typeof MenuPrimitive.Item>, DropdownMenuItemProps>(function DropdownMenuItem(
  { icon, danger, className, children, ...rest },
  ref,
) {
  return (
    <MenuPrimitive.Item ref={ref} className={["vt-menu-item", danger && "vt-menu-item--danger", className].filter(Boolean).join(" ")} {...rest}>
      {icon}
      {children}
    </MenuPrimitive.Item>
  );
});

export const DropdownMenuLabel = forwardRef<ElementRef<typeof MenuPrimitive.Label>, ComponentPropsWithoutRef<typeof MenuPrimitive.Label>>(
  function DropdownMenuLabel({ className, ...rest }, ref) {
    return <MenuPrimitive.Label ref={ref} className={["vt-menu-label", className].filter(Boolean).join(" ")} {...rest} />;
  },
);

export const DropdownMenuSeparator = forwardRef<ElementRef<typeof MenuPrimitive.Separator>, ComponentPropsWithoutRef<typeof MenuPrimitive.Separator>>(
  function DropdownMenuSeparator({ className, ...rest }, ref) {
    return <MenuPrimitive.Separator ref={ref} className={["vt-menu-sep", className].filter(Boolean).join(" ")} {...rest} />;
  },
);
