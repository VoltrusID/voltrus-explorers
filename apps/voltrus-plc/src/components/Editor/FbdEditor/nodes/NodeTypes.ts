import type { NodeTypes } from "@xyflow/react";
import { BlockNode } from "./BlockNode";
import { InputPinNode } from "./InputPinNode";
import { OutputPinNode } from "./OutputPinNode";
import { TimerNode } from "./TimerNode";
import { CounterNode } from "./CounterNode";

export const fbdNodeTypes: NodeTypes = {
  block: BlockNode,
  input_pin: InputPinNode,
  output_pin: OutputPinNode,
  timer: TimerNode,
  counter: CounterNode,
};
