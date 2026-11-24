import { useState, useEffect, useCallback } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open, save } from "@tauri-apps/plugin-dialog";
import { Download, Plus, Upload, X } from "lucide-react";
import { Button, IconButton } from "@mactools/ui";
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
    <div className="acc-body">
      {/* Language list */}
      <div className="sub-block">
        <div className="sub-title">Languages</div>

        {/* Default language selector */}
        <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
          <span className="prop-label" style={{ width: "auto", flexShrink: 0 }}>
            Default
          </span>
          <select
            value={defaultLanguage}
            onChange={(e) => setDefaultLanguage(e.target.value)}
            aria-label="Default language"
            className="vt-select vt-input--sm"
            style={{ flex: 1, minWidth: 0 }}
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

        {/* Language chips */}
        {langCodes.length > 0 && (
          <div className="lang-chips">
            {langCodes.map((code) => (
              <div
                key={code}
                className="lang-chip"
                aria-pressed={activeLang === code}
                onClick={() => setActiveLang(code)}
                title={langLabel(code)}
              >
                <span>{code}</span>
                {code !== defaultLanguage && (
                  <IconButton
                    label={`Remove ${langLabel(code)}`}
                    size="sm"
                    className="lang-chip-remove"
                    onClick={(e) => {
                      e.stopPropagation();
                      handleRemoveLanguage(code);
                    }}
                  >
                    <X size={11} strokeWidth={1.75} />
                  </IconButton>
                )}
              </div>
            ))}
          </div>
        )}

        {/* Add language */}
        {showAddInput ? (
          <div className="sub-actions">
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
              aria-label="Language code"
              autoFocus
              className="vt-input vt-input--sm"
              style={{ flex: 1, minWidth: 0 }}
            />
            <Button size="sm" variant="primary" onClick={handleAddLanguage} disabled={!addInput.trim()}>
              Add
            </Button>
            <Button
              size="sm"
              variant="ghost"
              onClick={() => {
                setShowAddInput(false);
                setAddInput("");
              }}
            >
              Cancel
            </Button>
          </div>
        ) : (
          <Button
            size="sm"
            variant="ghost"
            icon={<Plus size={14} strokeWidth={1.75} />}
            onClick={() => setShowAddInput(true)}
          >
            Add language
          </Button>
        )}
      </div>

      {/* Translation table */}
      <div className="sub-block" style={{ flex: 1, minHeight: 0, overflow: "hidden" }}>
        <div className="sub-title">
          <span>Translations</span>
          <span className="row-meta">{translatableKeys.length}</span>
        </div>

        {langCodes.length === 0 ? (
          <div className="sub-empty">Add a language to start translating.</div>
        ) : translatableKeys.length === 0 ? (
          <div className="sub-empty">No translatable strings yet. Add screens, widgets, or alarms.</div>
        ) : (
          <div className="sub-list" style={{ padding: "0 2px 8px" }}>
            <table className="vt-table lang-table">
              <thead>
                <tr>
                  <th>Key</th>
                  {langCodes.map((code) => (
                    <th key={code} className={code === activeLang ? "is-active" : undefined}>
                      {code}
                      {code === defaultLanguage && <span style={{ opacity: 0.5, marginLeft: 2 }}>*</span>}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {translatableKeys.map((key) => (
                  <tr key={key}>
                    <td title={key}>{key}</td>
                    {langCodes.map((code) => {
                      const currentValue = translations[code]?.[key] ?? "";
                      return (
                        <td key={code}>
                          <input
                            type="text"
                            value={currentValue}
                            onChange={(e) => setTranslation(key, code, e.target.value)}
                            placeholder={key}
                            aria-label={`Translation of ${key} in ${code}`}
                            className="vt-input vt-input--sm"
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

      {/* Import/export */}
      <div className="sub-block">
        <div className="sub-actions">
          <Button
            size="sm"
            variant="default"
            icon={<Upload size={14} strokeWidth={1.75} />}
            onClick={handleImportJson}
          >
            Import JSON
          </Button>
          <Button
            size="sm"
            variant="default"
            icon={<Download size={14} strokeWidth={1.75} />}
            onClick={handleExportJson}
            disabled={langCodes.length === 0}
          >
            Export JSON
          </Button>
        </div>
      </div>
    </div>
  );
}
