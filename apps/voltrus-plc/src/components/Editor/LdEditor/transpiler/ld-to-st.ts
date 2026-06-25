import type { LdProgram, LdElement } from "../../../../types/ld";
import { isContactType, isCoilType, getBlockDef } from "../../../../types/ld";

interface TranspileResult {
  source: string;
  diagnostics: Array<{ message: string; severity: "error" | "warning" }>;
}

const IEC_IDENTIFIER_RE = /^[A-Za-z_][A-Za-z0-9_]*$/;

export function transpileLdToSt(program: LdProgram): TranspileResult {
  const diagnostics: TranspileResult["diagnostics"] = [];

  if (program.rungs.length === 0) {
    return { source: `PROGRAM ${program.programName}\nEND_PROGRAM`, diagnostics };
  }

  if (!IEC_IDENTIFIER_RE.test(program.programName)) {
    diagnostics.push({ message: `Invalid program name: "${program.programName}"`, severity: "error" });
  }

  // Validate names
  for (const rung of program.rungs) {
    for (const path of rung.paths) {
      for (const el of path.elements) {
        if (el.variable && !IEC_IDENTIFIER_RE.test(el.variable)) {
          diagnostics.push({ message: `Invalid variable name: "${el.variable}"`, severity: "error" });
        }
        if (el.instanceName && !IEC_IDENTIFIER_RE.test(el.instanceName)) {
          diagnostics.push({ message: `Invalid instance name: "${el.instanceName}"`, severity: "error" });
        }
      }
    }
  }

  if (diagnostics.some((d) => d.severity === "error")) {
    return { source: "", diagnostics };
  }

  // Track declarations
  const varInputs = new Set<string>();
  const varOutputs = new Set<string>();
  const varInouts = new Map<string, string>();
  const fbInstances = new Map<string, string>();
  const edgeInstances = new Map<string, string>();
  let edgeCounter = 0;

  function contactExpr(el: LdElement): string {
    const v = el.variable || `__var_${el.id}`;
    switch (el.type) {
      case "contact_no": return v;
      case "contact_nc": return `NOT ${v}`;
      case "contact_re": {
        edgeCounter++;
        const inst = `__rtrig_${edgeCounter}`;
        edgeInstances.set(inst, v);
        return `${inst}.Q`;
      }
      case "contact_fe": {
        edgeCounter++;
        const inst = `__ftrig_${edgeCounter}`;
        edgeInstances.set(inst, v);
        return `${inst}.Q`;
      }
      default: return v;
    }
  }

  function trackVar(el: LdElement, isInput: boolean) {
    if (!el.variable) return;
    const reg = el.registerType;
    if (reg === "X" && isInput) varInputs.add(el.variable);
    else if (reg === "Y" && !isInput) varOutputs.add(el.variable);
    else varInouts.set(el.variable, el.variableType || "BOOL");
  }

  // Generate body
  const body: string[] = [];

  for (const rung of program.rungs) {
    const pathExprs: string[] = [];
    let outputEl: LdElement | null = null;

    for (const path of rung.paths) {
      const contacts: string[] = [];
      for (const el of path.elements) {
        if (isContactType(el.type)) {
          trackVar(el, true);
          contacts.push(contactExpr(el));
        } else if (isCoilType(el.type)) {
          outputEl = el;
          trackVar(el, false);
        } else if (el.type === "fb_block") {
          outputEl = el;
        }
      }
      if (contacts.length > 0) {
        pathExprs.push(contacts.length === 1 ? contacts[0] : contacts.join(" AND "));
      }
    }

    const rungExpr = pathExprs.length === 0
      ? "TRUE"
      : pathExprs.length === 1
        ? pathExprs[0]
        : `(${pathExprs.join(" OR ")})`;

    if (!outputEl) {
      diagnostics.push({ message: `Rung "${rung.label ?? rung.id}" has no output`, severity: "warning" });
      continue;
    }

    if (isCoilType(outputEl.type)) {
      const outVar = outputEl.variable || `__coil_${outputEl.id}`;
      switch (outputEl.type) {
        case "coil":
          body.push(`    ${outVar} := ${rungExpr};`);
          break;
        case "coil_set":
          body.push(`    IF ${rungExpr} THEN ${outVar} := TRUE; END_IF;`);
          break;
        case "coil_reset":
          body.push(`    IF ${rungExpr} THEN ${outVar} := FALSE; END_IF;`);
          break;
        case "coil_toggle":
          body.push(`    IF ${rungExpr} THEN ${outVar} := NOT ${outVar}; END_IF;`);
          diagnostics.push({ message: `Toggle coil "${outVar}" toggles every scan cycle — consider using R_TRIG`, severity: "warning" });
          break;
      }
    } else if (outputEl.type === "fb_block") {
      const blockType = outputEl.blockType || "TON";
      const def = getBlockDef(blockType);
      if (def) {
        const instance = outputEl.instanceName || `__fb_${outputEl.id}`;
        fbInstances.set(instance, blockType);

        const fbInputs = new Map<string, string>();
        fbInputs.set("IN", rungExpr);

        for (const [k, v] of Object.entries(outputEl.pinValues || {})) {
          if (v && !fbInputs.has(k)) fbInputs.set(k, v);
        }

        const args = def.pins
          .filter((p) => p.direction === "input")
          .map((p) => {
            const val = fbInputs.get(p.name) ?? "";
            return val ? `${p.name} := ${val}` : "";
          })
          .filter(Boolean)
          .join(", ");
        body.push(`    ${instance}(${args});`);
      }
    }
  }

  // Edge detection instances
  for (const [inst, varName] of edgeInstances) {
    body.unshift(`    ${inst}(CLK := ${varName});`);
  }

  // Assemble
  const sections: string[] = [`PROGRAM ${program.programName}`];
  const viArr = Array.from(varInputs);
  const voArr = Array.from(varOutputs);
  if (viArr.length > 0) sections.push("VAR_INPUT", ...viArr.map((v) => `    ${v} : BOOL;`), "END_VAR");
  if (voArr.length > 0) sections.push("VAR_OUTPUT", ...voArr.map((v) => `    ${v} : BOOL;`), "END_VAR");

  const varBody: string[] = [];
  for (const [name, type] of varInouts) varBody.push(`    ${name} : ${type};`);
  for (const [inst, blockType] of fbInstances) varBody.push(`    ${inst} : ${blockType};`);
  for (const inst of edgeInstances.keys()) varBody.push(`    ${inst} : ${inst.includes("ftrig") ? "F_TRIG" : "R_TRIG"};`);
  if (varBody.length > 0) sections.push("VAR", ...varBody, "END_VAR");
  if (body.length > 0) sections.push("", ...body);
  sections.push("END_PROGRAM");

  return { source: sections.join("\n"), diagnostics };
}
