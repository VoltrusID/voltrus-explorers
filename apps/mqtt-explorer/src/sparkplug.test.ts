import { describe, expect, it } from "vitest";
import { applySparkplug, EMPTY_SPARKPLUG, parseStatePayload, sortedNodes, type SparkplugState } from "./sparkplug";
import type { Decoded } from "./payload";

// Decoded values in the shape the Rust decoder (mt_mqtt::sparkplug) produces.
function sp(topic: string, value: Record<string, unknown>): Decoded {
  const [, group, type, edge, device] = topic.split("/");
  return {
    kind: "sparkplug",
    value: { topic: { group_id: group, message_type: type, edge_node_id: edge, device_id: device }, ...value },
  };
}

function feed(state: SparkplugState, topic: string, value: Record<string, unknown>, ts = 1): SparkplugState {
  return applySparkplug(state, topic, "", sp(topic, value), ts);
}

describe("applySparkplug", () => {
  it("builds edge node, device and metric values from births and data", () => {
    let s = EMPTY_SPARKPLUG;
    s = feed(s, "spBv1.0/Plant1/NBIRTH/Edge1", {
      seq: 0,
      bd_seq: 3,
      timestamp: 100,
      metrics: [
        { name: "bdSeq", datatype: "Int64", value: 3 },
        { name: "Line/Speed", alias: 1, datatype: "Double", value: 0 },
      ],
    });
    s = feed(s, "spBv1.0/Plant1/DBIRTH/Edge1/Filler", {
      seq: 1,
      metrics: [{ name: "Count", alias: 11, datatype: "UInt32", value: 1200, timestamp: 150 }],
    });
    s = feed(s, "spBv1.0/Plant1/NDATA/Edge1", {
      seq: 2,
      timestamp: 200,
      metrics: [{ name: "Line/Speed", name_from_alias: true, alias: 1, datatype: "Double", value: 1450.5 }],
    });
    s = feed(s, "spBv1.0/Plant1/DDATA/Edge1/Filler", {
      seq: 3,
      metrics: [{ name: "Count", alias: 11, datatype: "UInt32", value: 1201, timestamp: 300 }],
    });

    const [node] = sortedNodes(s);
    expect(node.group).toBe("Plant1");
    expect(node.id).toBe("Edge1");
    expect(node.online).toBe(true);
    expect(node.bdSeq).toBe(3);
    expect(node.seq).toBe(3);
    expect(node.births).toBe(1);
    expect(node.metrics["Line/Speed"]).toMatchObject({ value: 1450.5, alias: 1, datatype: "Double", ts: 200 });
    expect(node.devices.Filler.online).toBe(true);
    expect(node.devices.Filler.metrics.Count).toMatchObject({ value: 1201, ts: 300 });
  });

  it("keeps the session on a stale NDEATH and ends it on the matching one", () => {
    let s = feed(EMPTY_SPARKPLUG, "spBv1.0/G/NBIRTH/E", { seq: 0, bd_seq: 5, metrics: [] });
    s = feed(s, "spBv1.0/G/DBIRTH/E/D", { seq: 1, metrics: [] });
    s = feed(s, "spBv1.0/G/NDEATH/E", { bd_seq: 4, metrics: [], checks: ["NDEATH bdSeq 4 does not match"] });
    expect(s.nodes["G/E"].online).toBe(true);
    expect(s.nodes["G/E"].issues.map((i) => i.text)).toEqual(["NDEATH bdSeq 4 does not match"]);

    s = feed(s, "spBv1.0/G/NDEATH/E", { bd_seq: 5, metrics: [] });
    expect(s.nodes["G/E"].online).toBe(false);
    expect(s.nodes["G/E"].devices.D.online).toBe(false);
  });

  it("names alias-only metrics it cannot resolve and records decoder checks", () => {
    const s = feed(EMPTY_SPARKPLUG, "spBv1.0/G/NDATA/E", {
      seq: 7,
      metrics: [{ alias: 4, datatype: "Double", value: 1 }],
      checks: ["No NBIRTH seen for this edge node since you subscribed. Send a Rebirth"],
    });
    const n = s.nodes["G/E"];
    expect(n.online).toBeUndefined();
    expect(n.metrics["alias 4"].value).toBe(1);
    expect(n.issues[0].text).toContain("Rebirth");
  });

  it("tracks primary host STATE in both payload styles and ignores everything else", () => {
    let s = applySparkplug(EMPTY_SPARKPLUG, "spBv1.0/STATE/scada", '{"online":true,"timestamp":1}', undefined, 5);
    expect(s.hosts.scada).toEqual({ id: "scada", online: true, ts: 5 });
    s = applySparkplug(s, "spBv1.0/STATE/scada", "OFFLINE", undefined, 6);
    expect(s.hosts.scada.online).toBe(false);

    expect(applySparkplug(s, "sensors/t", "1", undefined, 7)).toBe(s);
    expect(applySparkplug(s, "spBv1.0/G/NDATA/E", "x", { kind: "sparkplug", error: "bad" }, 7)).toBe(s);
    // Commands are not node state.
    expect(feed(s, "spBv1.0/G/NCMD/E", { metrics: [{ name: "Node Control/Rebirth", value: true }] })).toBe(s);
  });

  it("parses STATE payloads", () => {
    expect(parseStatePayload("ONLINE")).toBe(true);
    expect(parseStatePayload('{"online":false,"timestamp":2}')).toBe(false);
    expect(parseStatePayload("hello")).toBeUndefined();
  });
});
