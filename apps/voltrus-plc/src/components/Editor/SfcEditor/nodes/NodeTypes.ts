import type { NodeTypes } from "@xyflow/react";
import StepNode from "./StepNode";
import TransitionNode from "./TransitionNode";
import BranchNode from "./BranchNode";
import JumpNode from "./JumpNode";

export const sfcNodeTypes: NodeTypes = {
  step: StepNode,
  initial_step: StepNode,
  transition: TransitionNode,
  or_diverge: BranchNode,
  or_converge: BranchNode,
  jump: JumpNode,
};
