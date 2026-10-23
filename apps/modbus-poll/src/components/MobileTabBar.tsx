import { PhoneTabBar, type PhoneTabItem } from "@mactools/ui";
import { ArrowLeftRight, ChartLine, LayoutTemplate, Layers, Plug, ScrollText, Table2 } from "lucide-react";
import { useStore, type MobileTab } from "../store";

const ICON = { size: 22, strokeWidth: 2 } as const;

/** Phone tabs. Five fit the bar; Log and Traffic sit under More. */
export function MobileTabBar() {
  const { mobileTab, setMobileTab, mode, slaveRunning, trendSeries } = useStore();

  const tabs: PhoneTabItem<MobileTab>[] = [
    { id: "connect", label: "Connect", icon: <Plug {...ICON} />, dot: mode === "slave" && slaveRunning },
    { id: "data", label: "Data", icon: <Table2 {...ICON} /> },
    mode === "master"
      ? { id: "blocks", label: "Blocks", icon: <Layers {...ICON} /> }
      : { id: "template", label: "Template", icon: <LayoutTemplate {...ICON} /> },
    { id: "trend", label: "Trend", icon: <ChartLine {...ICON} />, badge: trendSeries.size },
    { id: "log", label: "Log", icon: <ScrollText {...ICON} /> },
    { id: "traffic", label: "Traffic", icon: <ArrowLeftRight {...ICON} /> },
  ];

  return <PhoneTabBar tabs={tabs} active={mobileTab} onChange={setMobileTab} />;
}
