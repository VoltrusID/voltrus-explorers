import { useState, useEffect, useRef } from "react";
import { bridgeMode, invoke, listen, type UnlistenFn } from "../../api/bridge";
import { useStore } from "../../store";
import { useCloudStore } from "../../store/cloud";
import type { ChatMessage, CompileResult } from "../../types";

const MAX_MESSAGES = 100;

export default function AiChatPanel() {
  const {
    openFiles,
    activeFileIndex,
    aiMessages,
    addAiMessage,
    clearAiMessages,
    aiProvider,
    setAiProvider,
    aiModel,
    setAiModel,
    aiStreaming,
    setAiStreaming,
    ollamaEndpoint,
    setOllamaEndpoint,
    aiApiKey,
    setAiApiKey,
    diagnostics,
  } = useStore();

  // Cloud mode: the AI proxy authenticates with the server-side key (AI Keys
  // panel), so the local key input is replaced by a masked hint when one is
  // set; the apiKey argument is ignored by the cloud bridge either way.
  const cloudClaudeKey = useCloudStore((s) =>
    bridgeMode === "cloud" ? s.aiKeys.find((k) => k.provider === "claude") : undefined,
  );

  const [input, setInput] = useState("");
  const [streamBuffer, setStreamBuffer] = useState("");
  const [showSettings, setShowSettings] = useState(false);
  const [validationState, setValidationState] = useState<Map<string, { checking: boolean; result: CompileResult | null }>>(new Map());
  const messagesEnd = useRef<HTMLDivElement>(null);
  const streamingRef = useRef(false);

  const claudeModels = ["claude-sonnet-4-6", "claude-opus-4-7", "claude-haiku-4-5-20251001"];
  const ollamaModels = ["codellama:13b", "codellama:7b", "deepseek-coder:6.7b", "llama3.1:8b", "mistral:7b"];
  const models = aiProvider === "claude" ? claudeModels : ollamaModels;

  useEffect(() => {
    messagesEnd.current?.scrollIntoView({ behavior: "smooth" });
  }, [aiMessages, streamBuffer]);

  const buildContext = (): string => {
    const activeFile = openFiles[activeFileIndex];
    const source = activeFile?.content ?? "";
    const diagText = diagnostics
      .slice(0, 10)
      .map((d) => `  ${d.severity}: line ${d.line} — ${d.message}`)
      .join("\n");

    return `You are an IEC 61131-3 Structured Text PLC programming assistant.
Current PLC code:
\`\`\`st
${source}
\`\`\`
${diagText ? `Compile diagnostics:\n${diagText}` : "No compile errors."}
${openFiles.length > 1 ? `Other open POUs: ${openFiles.filter((_, i) => i !== activeFileIndex).map((f) => f.name).join(", ")}` : ""}

Help the user write, debug, explain, or optimize their PLC Structured Text programs. When generating code, wrap it in st code blocks. Keep explanations concise. Always output valid IEC 61131-3 ST syntax.`;
  };

  const sendMessage = async () => {
    const text = input.trim();
    if (!text || streamingRef.current) return;
    streamingRef.current = true;
    setAiStreaming(true);
    setShowSettings(false);

    const userMsg: ChatMessage = { role: "user", content: text };
    addAiMessage(userMsg);
    setInput("");
    setStreamBuffer("");

    const assistantMsg: ChatMessage = { role: "assistant", content: "" };
    addAiMessage(assistantMsg);

    let fullResponse = "";
    let unlistenChunk: UnlistenFn | null = null;
    let unlistenDone: UnlistenFn | null = null;
    let cleanedUp = false;

    const cleanup = () => {
      if (cleanedUp) return;
      cleanedUp = true;
      unlistenChunk?.();
      unlistenDone?.();
      streamingRef.current = false;
      setAiStreaming(false);
      setStreamBuffer("");
    };

    try {
      unlistenChunk = await listen<string>("ai-chunk", (event) => {
        fullResponse += event.payload;
        setStreamBuffer(fullResponse);
      });

      unlistenDone = await listen("ai-done", () => {
        cleanup();
        const msgs = useStore.getState().aiMessages;
        const lastIdx = msgs.length - 1;
        if (lastIdx >= 0 && msgs[lastIdx].role === "assistant") {
          msgs[lastIdx] = { role: "assistant", content: fullResponse };
          useStore.setState({ aiMessages: [...msgs] });
        }
      });

      const allMessages = useStore.getState().aiMessages;
      const recentMessages = allMessages.length > MAX_MESSAGES
        ? allMessages.slice(-MAX_MESSAGES)
        : allMessages;

      const messages: ChatMessage[] = [
        { role: "system", content: buildContext() },
        ...recentMessages.filter((m) => m.role !== "assistant" || m.content !== ""),
      ];

      if (aiProvider === "claude") {
        await invoke("chat_claude_stream", {
          messages,
          apiKey: aiApiKey,
          model: aiModel,
        });
      } else {
        await invoke("chat_ollama_stream", {
          messages,
          endpoint: ollamaEndpoint,
          model: aiModel,
        });
      }
    } catch (e) {
      cleanup();
      const msgs = useStore.getState().aiMessages;
      const lastIdx = msgs.length - 1;
      if (lastIdx >= 0 && msgs[lastIdx].role === "assistant") {
        msgs[lastIdx] = { role: "assistant", content: `Error: ${e}` };
        useStore.setState({ aiMessages: [...msgs] });
      }
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      sendMessage();
    }
  };

  const extractCodeBlocks = (content: string) => {
    const re = /```(?:st|iecst|structured.?text)?\s*\n([\s\S]*?)```/gi;
    const blocks: string[] = [];
    let m;
    while ((m = re.exec(content)) !== null) {
      blocks.push(m[1].trim());
    }
    return blocks;
  };

  const validateAndApply = async (code: string) => {
    const key = code.slice(0, 80);
    setValidationState((prev) => {
      const next = new Map(prev);
      next.set(key, { checking: true, result: null });
      return next;
    });
    try {
      const result = await invoke<CompileResult>("compile_source", {
        source: code,
        fileName: "AI_Generated.st",
      });
      setValidationState((prev) => {
        const next = new Map(prev);
        next.set(key, { checking: false, result });
        return next;
      });
      if (result?.success) {
        useStore.getState().updateFileContent(activeFileIndex, code);
      }
    } catch {
      setValidationState((prev) => {
        const next = new Map(prev);
        next.set(key, { checking: false, result: null });
        return next;
      });
    }
  };

  const selectStyle: React.CSSProperties = {
    width: "100%",
    background: "var(--bg-highlight)",
    border: "1px solid var(--border)",
    borderRadius: 4,
    color: "var(--text-primary)",
    padding: "4px 6px",
    fontSize: 12,
  };

  const labelStyle: React.CSSProperties = {
    display: "block",
    color: "var(--text-muted)",
    fontSize: 11,
    marginBottom: 2,
  };

  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%", fontSize: 12 }}>
      {/* Header with gear toggle */}
      <div style={{
        display: "flex",
        alignItems: "center",
        padding: "6px 10px",
        borderBottom: "1px solid var(--border)",
        gap: 8,
      }}>
        <span style={{ fontSize: 11, fontWeight: 600, color: "var(--text-muted)", textTransform: "uppercase", letterSpacing: 0.5 }}>
          AI Chat
        </span>
        <span style={{ fontSize: 10, color: "var(--text-muted)", opacity: 0.6 }}>
          {aiProvider === "claude" ? "Claude" : "Ollama"} / {aiModel.split(":")[0].split("-").slice(-1)[0]}
        </span>
        <span style={{ flex: 1 }} />
        <button
          onClick={() => setShowSettings(!showSettings)}
          style={{
            background: showSettings ? "var(--border)" : "none",
            border: "none",
            cursor: "pointer",
            color: "var(--text-muted)",
            fontSize: 14,
            padding: "2px 4px",
            borderRadius: 4,
            lineHeight: 1,
          }}
          title="AI Settings"
        >
          &#9881;
        </button>
      </div>

      {/* Collapsible settings */}
      {showSettings && (
        <div style={{
          padding: "8px 10px",
          borderBottom: "1px solid var(--border)",
          background: "var(--surface)",
        }}>
          <div style={{ marginBottom: 8 }}>
            <label style={labelStyle}>Provider</label>
            <select
              value={aiProvider}
              onChange={(e) => {
                setAiProvider(e.target.value as "claude" | "ollama");
                const newModels = e.target.value === "claude" ? claudeModels : ollamaModels;
                if (!newModels.includes(aiModel)) setAiModel(newModels[0]);
              }}
              style={selectStyle}
            >
              <option value="claude">Claude (Anthropic)</option>
              <option value="ollama">Ollama (Local)</option>
            </select>
          </div>

          {aiProvider === "claude" && (
            <div style={{ marginBottom: 8 }}>
              <label style={labelStyle}>API Key</label>
              {cloudClaudeKey ? (
                <div style={{ color: "var(--text-muted)" }}>
                  Using cloud key ({cloudClaudeKey.masked})
                </div>
              ) : (
                <input
                  type="password"
                  className="scan-cycle-input"
                  style={{ width: "100%", textAlign: "left" }}
                  value={aiApiKey}
                  onChange={(e) => setAiApiKey(e.target.value)}
                  placeholder="sk-ant-api03-..."
                />
              )}
            </div>
          )}

          {aiProvider === "ollama" && (
            <div style={{ marginBottom: 8 }}>
              <label style={labelStyle}>Endpoint</label>
              <input
                className="scan-cycle-input"
                style={{ width: "100%", textAlign: "left" }}
                value={ollamaEndpoint}
                onChange={(e) => setOllamaEndpoint(e.target.value)}
                placeholder="localhost:11434"
              />
            </div>
          )}

          <div>
            <label style={labelStyle}>Model</label>
            <select value={aiModel} onChange={(e) => setAiModel(e.target.value)} style={selectStyle}>
              {models.map((m) => (
                <option key={m} value={m}>{m}</option>
              ))}
            </select>
          </div>
        </div>
      )}

      {/* Messages */}
      <div style={{ flex: 1, overflow: "auto", padding: "8px 0" }}>
        {aiMessages.length === 0 && (
          <div style={{ color: "var(--text-muted)", padding: 16, textAlign: "center" }}>
            Ask me about PLC programming, debugging, or code generation.
            {aiProvider === "claude" && (
              <div style={{ marginTop: 8, fontSize: 10, opacity: 0.7 }}>
                Note: your PLC source code will be sent to Anthropic's API.
              </div>
            )}
          </div>
        )}
        {aiMessages.map((msg, i) => (
          <div
            key={`${msg.role}-${i}`}
            style={{
              marginBottom: 8,
              padding: "4px 8px",
              borderRadius: 6,
              background: msg.role === "user" ? "var(--bg-highlight)" : "transparent",
            }}
          >
            <span style={{ color: "var(--accent)", fontSize: 10, fontWeight: 600 }}>
              {msg.role === "user" ? "You" : "AI"}
            </span>
            <div style={{ whiteSpace: "pre-wrap", wordBreak: "break-word", marginTop: 2 }}>
              {msg.content ||
                (i === aiMessages.length - 1 && aiStreaming ? (
                  <span>
                    {streamBuffer || <span style={{ color: "var(--text-muted)" }}>Thinking...</span>}
                    <span className="cursor-blink">|</span>
                  </span>
                ) : null)}
            </div>
            {msg.role === "assistant" && msg.content && extractCodeBlocks(msg.content).map((code, j) => {
              const vKey = code.slice(0, 80);
              const vs = validationState.get(vKey);
              return (
                <div key={`code-${i}-${j}`} style={{ marginTop: 4 }}>
                  <pre style={{
                    background: "var(--bg-highlight)",
                    padding: 8,
                    borderRadius: 4,
                    fontSize: 11,
                    maxHeight: 200,
                    overflow: "auto",
                    border: vs?.result
                      ? vs.result.success
                        ? "1px solid #22c55e"
                        : "1px solid var(--danger)"
                      : "1px solid transparent",
                  }}>
                    <code>{code}</code>
                  </pre>
                  <div style={{ display: "flex", gap: 4, marginTop: 2, alignItems: "center" }}>
                    <button
                      className="toolbar-btn primary"
                      style={{ fontSize: 10, padding: "2px 8px" }}
                      onClick={() => validateAndApply(code)}
                      disabled={vs?.checking}
                    >
                      {vs?.checking ? "Validating..." : "Validate & Apply"}
                    </button>
                    {vs?.result && !vs.result.success && (
                      <span style={{ fontSize: 10, color: "var(--danger)" }}>
                        {vs.result.diagnostics.length} error{vs.result.diagnostics.length > 1 ? "s" : ""}
                      </span>
                    )}
                    {vs?.result?.success && (
                      <span style={{ fontSize: 10, color: "#22c55e" }}>Valid</span>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        ))}
        <div ref={messagesEnd} />
      </div>

      {/* Input */}
      <div style={{ borderTop: "1px solid var(--border)", padding: "8px 8px 0" }}>
        <textarea
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={handleKeyDown}
          placeholder={aiProvider === "claude" ? "Ask Claude..." : "Ask Ollama..."}
          disabled={aiStreaming}
          style={{
            width: "100%",
            background: "var(--bg-highlight)",
            border: "1px solid var(--border)",
            borderRadius: 6,
            color: "var(--text-primary)",
            padding: "6px 8px",
            fontSize: 12,
            resize: "none",
            minHeight: 40,
            maxHeight: 120,
          }}
          rows={2}
        />
        <div style={{ display: "flex", gap: 6, marginTop: 4, paddingBottom: 8 }}>
          <button
            className="toolbar-btn primary"
            style={{ flex: 1, fontSize: 11 }}
            onClick={sendMessage}
            disabled={aiStreaming || !input.trim()}
          >
            {aiStreaming ? "Streaming..." : "Send"}
          </button>
          <button
            className="toolbar-btn"
            style={{ fontSize: 11 }}
            onClick={clearAiMessages}
          >
            Clear
          </button>
        </div>
      </div>
    </div>
  );
}
