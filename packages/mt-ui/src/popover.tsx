import { forwardRef, type ComponentPropsWithoutRef, type ElementRef } from "react";
import * as PopoverPrimitive from "@radix-ui/react-popover";

/*
 * Popover: a floating card anchored to a trigger (Radix Popover, so focus moves in on open, Escape and an outside
 * press close it and focus returns to the trigger). Compose like shadcn/ui:
 *
 *   <Popover><PopoverTrigger asChild><Button>Filters</Button></PopoverTrigger><PopoverContent>...</PopoverContent></Popover>
 */
export const Popover = PopoverPrimitive.Root;
export const PopoverTrigger = PopoverPrimitive.Trigger;
export const PopoverAnchor = PopoverPrimitive.Anchor;
export const PopoverClose = PopoverPrimitive.Close;

export const PopoverContent = forwardRef<ElementRef<typeof PopoverPrimitive.Content>, ComponentPropsWithoutRef<typeof PopoverPrimitive.Content>>(
  function PopoverContent({ className, align = "start", sideOffset = 4, collisionPadding = 8, ...rest }, ref) {
    return (
      <PopoverPrimitive.Portal>
        <PopoverPrimitive.Content
          ref={ref}
          align={align}
          sideOffset={sideOffset}
          collisionPadding={collisionPadding}
          className={["vt-popover", className].filter(Boolean).join(" ")}
          {...rest}
        />
      </PopoverPrimitive.Portal>
    );
  },
);
