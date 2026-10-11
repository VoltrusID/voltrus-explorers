import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useStore } from "../store";

const LANG_LABELS: Record<string, string> = {
  en: "English",
  id: "Bahasa Indonesia",
  ja: "Japanese",
  de: "German",
  fr: "French",
  es: "Spanish",
  zh: "Chinese",
  ko: "Korean",
  pt: "Portuguese",
  ru: "Russian",
  ar: "Arabic",
  hi: "Hindi",
  th: "Thai",
  vi: "Vietnamese",
  it: "Italian",
  nl: "Dutch",
  pl: "Polish",
  tr: "Turkish",
};

function langLabel(code: string): string {
  return LANG_LABELS[code] ?? code.toUpperCase();
}

export function LanguageManager() {
  const translations = useStore((s) => s.translations);
  const defaultLanguage = useStore((s) => s.defaultLanguage);
  const addLanguage = useStore((s) => s.addLanguage);
  const removeLanguage = useStore((s) => s.removeLanguage);
  const setTranslation = useStore((s) => s.setTranslation);
  const setDefaultLanguage = useStore((s) => s.setDefaultLanguage);
  const collectTranslatableStrings = useStore((s) => s.collectTranslatableStrings);

  const langCodes = Object.keys(translations);
  const nonDefaultLangs = langCodes.filter((c) => c !== defaultLanguage);

  const [activeLang, setActiveLang] = useState<string>(
    langCodes.length > 0 ? langCodes[0] : ""
  );
  const [addInput, setAddInput] = useState("");
  const [showAddInput, setShowAddInput] = useState(false);

  const translatableKeys = collectTranslatableStrings();

  // Keep activeLang valid when languages change
  useEffect(() => {
    if (langCodes.length === 0) {
      setActiveLang("");
    } else if (!langCodes.includes(activeLang)) {
      setActiveLang(langCodes[0]);
    }
  }, [langCodes, activeLang]);

  const handleAddLanguage = useCallback(() => {
    const code = addInput.trim().toLowerCase();
    if (!code) return;
    if (translations[code]) {
      setAddInput("");
      setShowAddInput(false);
      return;
    }
    addLanguage(code);
    setActiveLang(code);
    setAddInput("");
    setShowAddInput(false);
  }, [addInput, translations, addLanguage]);

  const handleRemoveLanguage = useCallback(
    (code: string) => {
      removeLanguage(code);
    },
    [removeLanguage]
  );

  const handleExportJson = useCallback(async () => {
    try {
      const { save } = await import("@tauri-apps/plugin-dialog");
      const path = await save({
        title: "Export Translations",
        filters: [{ name: "JSON", extensions: ["json"] }],
        defaultPath: "translations.json",
      });
      if (path) {
        await invoke("save_project_data", {
          path,
          data: JSON.stringify(translations, null, 2),
        });
      }
    } catch (e) {
      useStore.getState().showToast("error", `Export translations failed: ${String(e)}`);
    }
  }, [translations]);

  const handleImportJson = useCallback(async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const path = await open({
        title: "Import Translations",
        filters: [{ name: "JSON", extensions: ["json"] }],
        multiple: false,
      });
      if (!path) return;

      const data = await invoke<string>("load_project_data", { path });
      const parsed = JSON.parse(data) as Record<string, Record<string, string>>;

      for (const [langCode, entries] of Object.entries(parsed)) {
        addLanguage(langCode);
        for (const [key, value] of Object.entries(entries)) {
          setTranslation(key, langCode, value);
        }
      }
    } catch (e) {
      useStore.getState().showToast("error", `Import translations failed: ${String(e)}`);
    }
  }, [addLanguage, setTranslation]);

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        height: "100%",
        fontSize: 12,
      }}
    >
      {/* Header */}
      <div
        style={{
          padding: "10px 12px",
          borderBottom: "1px solid var(--border)",
        }}
      >
        <div
          style={{
            fontSize: 11,
            fontWeight: 600,
            textTransform: "uppercase",
            letterSpacing: 0.5,
            color: "var(--text-muted)",
            marginBottom: 8,
          }}
        >
          Languages
        </div>

        {/* Default language selector */}
        <div style={{ display: "flex", alignItems: "center", gap: 6, marginBottom: 8 }}>
          <span style={{ fontSize: 11, color: "var(--text-muted)", flexShrink: 0 }}>
            Default:
          </span>
          <select
            value={defaultLanguage}
            onChange={(e) => setDefaultLanguage(e.target.value)}
            style={{
              flex: 1,
              padding: "3px 6px",
              border: "1px solid var(--border)",
              borderRadius: 4,
              background: "var(--input-bg)",
              color: "var(--text)",
              fontSize: 11,
              outline: "none",
            }}
          >
            {/* The default may not have a translation column yet (e.g. "en"
                before any language is added) — still show it truthfully
                instead of letting the select display a different language. */}
            {!langCodes.includes(defaultLanguage) && (
              <option value={defaultLanguage}>
                {langLabel(defaultLanguage)} ({defaultLanguage})
              </option>
            )}
            {langCodes.map((code) => (
              <option key={code} value={code}>
                {langLabel(code)} ({code})
              </option>
            ))}
          </select>
        </div>

        {/* Language tabs */}
        <div style={{ display: "flex", flexWrap: "wrap", gap: 4, marginBottom: 8 }}>
          {langCodes.map((code) => (
            <div
              key={code}
              style={{
                display: "flex",
                alignItems: "center",
                gap: 3,
                padding: "3px 8px",
                borderRadius: 4,
                cursor: "pointer",
                border:
                  activeLang === code
                    ? "1px solid var(--accent)"
                    : "1px solid var(--border)",
                background:
                  activeLang === code ? "var(--accent)" : "var(--surface)",
                color: activeLang === code ? "#000" : "var(--text)",
                fontSize: 11,
                fontWeight: activeLang === code ? 600 : 400,
                whiteSpace: "nowrap",
              }}
              onClick={() => setActiveLang(code)}
            >
              <span>{code}</span>
              {code !== defaultLanguage && (
                <button
                  onClick={(e) => {
                    e.stopPropagation();
                    handleRemoveLanguage(code);
                  }}
                  title={`Remove ${langLabel(code)}`}
                  style={{
                    border: "none",
                    background: "transparent",
                    color: activeLang === code ? "#000" : "var(--text-muted)",
                    cursor: "pointer",
                    padding: 0,
                    fontSize: 13,
                    lineHeight: 1,
                    marginLeft: 2,
                  }}
                >
                  &times;
                </button>
              )}
            </div>
          ))}
        </div>

        {/* Add language */}
        {showAddInput ? (
          <div style={{ display: "flex", gap: 4 }}>
            <input
              type="text"
              value={addInput}
              onChange={(e) => setAddInput(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") handleAddLanguage();
                else if (e.key === "Escape") {
                  setShowAddInput(false);
                  setAddInput("");
                }
              }}
              placeholder='e.g. "id", "ja", "de"'
              autoFocus
              style={{
                flex: 1,
                minWidth: 0,
                padding: "3px 6px",
                border: "1px solid var(--accent)",
                borderRadius: 4,
                background: "var(--input-bg)",
                color: "var(--text)",
                fontSize: 11,
                outline: "none",
              }}
            />
            <button
              onClick={handleAddLanguage}
              disabled={!addInput.trim()}
              style={{
                padding: "3px 8px",
                border: "1px solid var(--border)",
                borderRadius: 4,
                background: "var(--accent)",
                color: "#000",
                fontSize: 11,
                cursor: addInput.trim() ? "pointer" : "default",
                opacity: addInput.trim() ? 1 : 0.5,
              }}
            >
              Add
            </button>
            <button
              onClick={() => {
                setShowAddInput(false);
                setAddInput("");
              }}
              style={{
                padding: "3px 6px",
                border: "1px solid var(--border)",
                borderRadius: 4,
                background: "var(--surface)",
                color: "var(--text-muted)",
                fontSize: 11,
                cursor: "pointer",
              }}
            >
              Cancel
            </button>
          </div>
        ) : (
          <button
            onClick={() => setShowAddInput(true)}
            style={{
              width: "100%",
              padding: "4px 10px",
              border: "1px solid var(--border)",
              borderRadius: 4,
              background: "var(--surface)",
              color: "var(--text)",
              fontSize: 11,
              cursor: "pointer",
            }}
          >
            + Add Language
          </button>
        )}
      </div>

      {/* Translation Table */}
      <div
        style={{
          flex: 1,
          overflow: "hidden",
          display: "flex",
          flexDirection: "column",
          borderBottom: "1px solid var(--border)",
        }}
      >
        <div
          style={{
            padding: "10px 12px 6px",
            fontSize: 11,
            fontWeight: 600,
            textTransform: "uppercase",
            letterSpacing: 0.5,
            color: "var(--text-muted)",
          }}
        >
          Translations ({translatableKeys.length})
        </div>

        {langCodes.length === 0 ? (
          <div
            style={{
              color: "var(--text-muted)",
              fontSize: 11,
              textAlign: "center",
              padding: "12px",
              opacity: 0.6,
            }}
          >
            Add a language to start translating.
          </div>
        ) : translatableKeys.length === 0 ? (
          <div
            style={{
              color: "var(--text-muted)",
              fontSize: 11,
              textAlign: "center",
              padding: "12px",
              opacity: 0.6,
            }}
          >
            No translatable strings yet. Add screens, widgets, or alarms.
          </div>
        ) : (
          <div style={{ flex: 1, overflowY: "auto", padding: "0 12px 8px" }}>
            <table
              style={{
                width: "100%",
                borderCollapse: "collapse",
                fontSize: 11,
              }}
            >
              <thead>
                <tr>
                  <th
                    style={{
                      textAlign: "left",
                      padding: "4px 6px",
                      borderBottom: "1px solid var(--border)",
                      color: "var(--text-muted)",
                      fontWeight: 600,
                      fontSize: 10,
                      position: "sticky",
                      top: 0,
                      background: "var(--card)",
                      zIndex: 1,
                    }}
                  >
                    Key
                  </th>
                  {langCodes.map((code) => (
                    <th
                      key={code}
                      style={{
                        textAlign: "left",
                        padding: "4px 6px",
                        borderBottom: "1px solid var(--border)",
                        color: code === activeLang ? "var(--accent)" : "var(--text-muted)",
                        fontWeight: 600,
                        fontSize: 10,
                        position: "sticky",
                        top: 0,
                        background: "var(--card)",
                        zIndex: 1,
                        minWidth: 80,
                      }}
                    >
                      {code}
                      {code === defaultLanguage && (
                        <span style={{ opacity: 0.5, marginLeft: 2 }}>*</span>
                      )}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {translatableKeys.map((key) => (
                  <tr key={key}>
                    <td
                      style={{
                        padding: "3px 6px",
                        borderBottom: "1px solid var(--border)",
                        color: "var(--text-muted)",
                        fontSize: 10,
                        maxWidth: 100,
                        overflow: "hidden",
                        textOverflow: "ellipsis",
                        whiteSpace: "nowrap",
                      }}
                      title={key}
                    >
                      {key}
                    </td>
                    {langCodes.map((code) => {
                      const currentValue = translations[code]?.[key] ?? "";
                      return (
                        <td
                          key={code}
                          style={{
                            padding: "2px 4px",
                            borderBottom: "1px solid var(--border)",
                          }}
                        >
                          <input
                            type="text"
                            value={currentValue}
                            onChange={(e) =>
                              setTranslation(key, code, e.target.value)
                            }
                            placeholder={key}
                            style={{
                              width: "100%",
                              padding: "2px 4px",
                              border:
                                code === activeLang
                                  ? "1px solid var(--accent)"
                                  : "1px solid var(--border)",
                              borderRadius: 3,
                              background: "var(--input-bg)",
                              color: currentValue
                                ? "var(--text)"
                                : "var(--text-muted)",
                              fontSize: 11,
                              outline: "none",
                            }}
                          />
                        </td>
                      );
                    })}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {/* Import/Export */}
      <div
        style={{
          padding: "8px 12px",
          display: "flex",
          gap: 6,
        }}
      >
        <button
          onClick={handleImportJson}
          style={{
            flex: 1,
            padding: "4px 10px",
            border: "1px solid var(--border)",
            borderRadius: 4,
            background: "var(--surface)",
            color: "var(--text)",
            fontSize: 11,
            cursor: "pointer",
          }}
        >
          Import JSON
        </button>
        <button
          onClick={handleExportJson}
          disabled={langCodes.length === 0}
          style={{
            flex: 1,
            padding: "4px 10px",
            border: "1px solid var(--border)",
            borderRadius: 4,
            background: "var(--surface)",
            color: "var(--text)",
            fontSize: 11,
            cursor: langCodes.length > 0 ? "pointer" : "default",
            opacity: langCodes.length > 0 ? 1 : 0.5,
          }}
        >
          Export JSON
        </button>
      </div>
    </div>
  );
}
