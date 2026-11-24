/**
 * Theme choice: follow the system, dark, light, or Petrol (light grey panels with a petrol-teal
 * accent, the look of classic engineering workstations). The choice is kept in localStorage;
 * `data-theme` on <html> always holds the resolved theme so CSS never has to look at the system.
 */
export type ThemeChoice = "system" | "dark" | "light" | "petrol";
export type ResolvedTheme = "dark" | "light" | "petrol";

export const THEME_STORAGE_KEY = "vplc_theme";
const CHOICES: readonly ThemeChoice[] = ["system", "dark", "light", "petrol"];

export function loadThemeChoice(storage: Pick<Storage, "getItem"> | undefined = globalThis.localStorage): ThemeChoice {
  try {
    const saved = storage?.getItem(THEME_STORAGE_KEY);
    return CHOICES.includes(saved as ThemeChoice) ? (saved as ThemeChoice) : "system";
  } catch {
    return "system";
  }
}

export function saveThemeChoice(choice: ThemeChoice, storage: Pick<Storage, "setItem"> | undefined = globalThis.localStorage) {
  try {
    storage?.setItem(THEME_STORAGE_KEY, choice);
  } catch {
    // private mode or storage full: the choice lasts for this session only
  }
}

export function resolveTheme(choice: ThemeChoice, systemPrefersLight: boolean): ResolvedTheme {
  if (choice === "system") return systemPrefersLight ? "light" : "dark";
  return choice;
}

/** Whether a resolved theme draws dark text on light surfaces (for CodeMirror and canvas editors). */
export function isLightTheme(theme: ResolvedTheme): boolean {
  return theme !== "dark";
}
