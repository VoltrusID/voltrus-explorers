import { PhoneTabBar, type PhoneTabItem } from "@mactools/ui";
import { ArrowLeftRight, ChartLine, LayoutTemplate, Layers, Plug, ScrollText, Sparkles, Table2 } from "lucide-react";
import { useStore, type MobileTab } from "../store";

const ICON = { size: 22, strokeWidth: 2 } as const;

type Entry = MobileTab | "assistant";

/**
 * Phone tabs. Five fit the bar; Log, Traffic and the Assistant sit under More. The Assistant is not a page: choosing it
 * calls `assistant`, which opens its sheet. Absent (`assistant` undefined) when it is off for this account.
 */
export function MobileTabBar({ assistant }: { assistant?: () => void }) {
  const { mobileTab, setMobileTab, mode, slaveRunning, trendSeries } = useStore();

  const tabs: PhoneTabItem<Entry>[] = [
    { id: "connect", label: "Connect", icon: <Plug {...ICON} />, dot: mode === "slave" && slaveRunning },
    { id: "data", label: "Data", icon: <Table2 {...ICON} /> },
    mode === "master"
      ? { id: "blocks", label: "Blocks", icon: <Layers {...ICON} /> }
      : { id: "template", label: "Template", icon: <LayoutTemplate {...ICON} /> },
    { id: "trend", label: "Trend", icon: <ChartLine {...ICON} />, badge: trendSeries.size },
    { id: "log", label: "Log", icon: <ScrollText {...ICON} /> },
    { id: "traffic", label: "Traffic", icon: <ArrowLeftRight {...ICON} /> },
    ...(assistant ? [{ id: "assistant" as const, label: "Assistant", icon: <Sparkles {...ICON} /> }] : []),
  ];

  return <PhoneTabBar tabs={tabs} active={mobileTab} onChange={(t) => (t === "assistant" ? assistant?.() : setMobileTab(t))} />;
}
