import type { SheetOption } from "@mactools/ui";

export const POLICIES: SheetOption<string>[] = [
  { value: "None", label: "None", hint: "No security, for trusted test networks" },
  { value: "Basic256Sha256", label: "Basic256Sha256", hint: "The usual choice, widely supported" },
  { value: "Aes256Sha256RsaPss", label: "Aes256_Sha256_RsaPss", hint: "Strongest, if the server lists it" },
  { value: "Aes128Sha256RsaOaep", label: "Aes128_Sha256_RsaOaep", hint: "Newer SHA-256 profile" },
  { value: "Basic256", label: "Basic256", hint: "Deprecated, only for old servers" },
  { value: "Basic128Rsa15", label: "Basic128Rsa15", hint: "Deprecated, only for old servers" },
];

/** Discovery names policies like `Aes128_Sha256_RsaOaep`; the form uses the same names without underscores. */
export function normalizePolicy(name: string): string {
  const key = name.replace(/_/g, "").toLowerCase();
  return POLICIES.find((p) => p.value.toLowerCase() === key)?.value ?? name;
}
