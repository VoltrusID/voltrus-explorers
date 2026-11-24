// ── Core model shared by the HMI designer and the SCADA editor ──
//
// This is the union of voltrus-hmi's HmiWidget/HmiScreen and scada2's
// WidgetInstance/layout, reshaped so one engine can serve both:
//   - `binding`   ≙ HMI `variableName` / SCADA `sensorId` (stringified)
//   - `bindings`  ≙ SCADA `sensorIds`
//   - `config`    ≙ SCADA per-widget props bag (HMI's flat min/max/color
//                   fields migrate into it)
//   - `rotation`  ≙ SCADA parity; HMI treats it as 0

export type JsonValue =
  | string
  | number
  | boolean
  | null
  | JsonValue[]
  | { [k: string]: JsonValue };

export interface SymbolInstance {
  /** uuid-ish, opaque */
  id: string;
  /** widget kind, registry-owned ('gauge' | 'button' | ... free-form) */
  type: string;
  x: number;
  y: number;
  w: number;
  h: number;
  /** degrees, SCADA parity; HMI treats 0 */
  rotation: number;
  zIndex: number;
  label?: string;
  /** primary tag/sensor key (HMI variableName / SCADA sensorId) */
  binding?: string | null;
  /** multi-bind (SCADA sensorIds) */
  bindings?: string[];
  /** widget-specific props (color, min/max, schema-driven config…) */
  config: JsonValue;
}

export interface SymbolDoc {
  /** screen/page id */
  id: string;
  name: string;
  /** canvas size (HMI: 1024x600) */
  width: number;
  height: number;
  /** z-order = array order (ascending zIndex derived, not stored twice) */
  symbols: SymbolInstance[];
}
