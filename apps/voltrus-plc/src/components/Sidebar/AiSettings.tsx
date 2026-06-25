import { useStore } from "../../store";

export default function AiSettings() {
  const {
    aiProvider,
    setAiProvider,
    aiModel,
    setAiModel,
    ollamaEndpoint,
    setOllamaEndpoint,
    aiApiKey,
    setAiApiKey,
  } = useStore();

  const claudeModels = [
    "claude-sonnet-4-6",
    "claude-opus-4-7",
    "claude-haiku-4-5-20251001",
  ];

  const ollamaModels = [
    "codellama:13b",
    "codellama:7b",
    "deepseek-coder:6.7b",
    "llama3.1:8b",
    "mistral:7b",
  ];

  const models = aiProvider === "claude" ? claudeModels : ollamaModels;

  return (
    <div style={{ padding: 8, fontSize: 12 }}>
      <div style={{ marginBottom: 8 }}>
        <label style={{ display: "block", color: "var(--text-muted)", fontSize: 11, marginBottom: 2 }}>
          Provider
        </label>
        <select
          value={aiProvider}
          onChange={(e) => setAiProvider(e.target.value as "claude" | "ollama")}
          style={{
            width: "100%",
            background: "var(--bg-highlight)",
            border: "1px solid var(--border)",
            borderRadius: 4,
            color: "var(--text-primary)",
            padding: "4px 6px",
            fontSize: 12,
          }}
        >
          <option value="claude">Claude (Anthropic)</option>
          <option value="ollama">Ollama (Local)</option>
        </select>
      </div>

      {aiProvider === "claude" && (
        <div style={{ marginBottom: 8 }}>
          <label style={{ display: "block", color: "var(--text-muted)", fontSize: 11, marginBottom: 2 }}>
            API Key
          </label>
          <input
            type="password"
            className="scan-cycle-input"
            style={{ width: "100%", textAlign: "left" }}
            value={aiApiKey}
            onChange={(e) => setAiApiKey(e.target.value)}
            placeholder="sk-ant-api03-..."
          />
        </div>
      )}

      {aiProvider === "ollama" && (
        <div style={{ marginBottom: 8 }}>
          <label style={{ display: "block", color: "var(--text-muted)", fontSize: 11, marginBottom: 2 }}>
            Endpoint
          </label>
          <input
            className="scan-cycle-input"
            style={{ width: "100%", textAlign: "left" }}
            value={ollamaEndpoint}
            onChange={(e) => setOllamaEndpoint(e.target.value)}
            placeholder="localhost:11434"
          />
        </div>
      )}

      <div style={{ marginBottom: 8 }}>
        <label style={{ display: "block", color: "var(--text-muted)", fontSize: 11, marginBottom: 2 }}>
          Model
        </label>
        <select
          value={aiModel}
          onChange={(e) => setAiModel(e.target.value)}
          style={{
            width: "100%",
            background: "var(--bg-highlight)",
            border: "1px solid var(--border)",
            borderRadius: 4,
            color: "var(--text-primary)",
            padding: "4px 6px",
            fontSize: 12,
          }}
        >
          {models.map((m) => (
            <option key={m} value={m}>
              {m}
            </option>
          ))}
        </select>
      </div>
    </div>
  );
}
