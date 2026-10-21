import type { ReactNode } from "react";

export type PhoneTab = "devices" | "objects" | "trend" | "alarms" | "log";

export interface PhoneTabItem {
  id: PhoneTab;
  label: string;
  icon: ReactNode;
  /** Small count shown on the tab (e.g. trend series). */
  badge?: number;
}

interface MobileTabBarProps {
  tabs: PhoneTabItem[];
  active: PhoneTab;
  onChange: (tab: PhoneTab) => void;
}

/** Bottom tab bar for the phone layout; clears the home indicator through the safe-area inset. */
export function MobileTabBar({ tabs, active, onChange }: MobileTabBarProps) {
  return (
    <nav className="bx-tabbar" role="tablist" aria-label="Sections">
      {tabs.map((tab) => (
        <button
          key={tab.id}
          type="button"
          role="tab"
          aria-selected={active === tab.id}
          className="bx-tab"
          onClick={() => onChange(tab.id)}
        >
          {tab.icon}
          <span>{tab.label}</span>
          {tab.badge ? <span className="bx-tab__badge">{tab.badge}</span> : null}
        </button>
      ))}
    </nav>
  );
}
