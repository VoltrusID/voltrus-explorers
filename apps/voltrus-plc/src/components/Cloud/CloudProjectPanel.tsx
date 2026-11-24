import { useState } from "react";
import { Button, Callout, EmptyState } from "@mactools/ui";
import { RefreshCw } from "lucide-react";
import { useCloudStore } from "../../store/cloud";
import { useStore } from "../../store";
import "./cloud.css";

// Cloud project browser: project list (open/delete), create field, and the
// file list of the active project. File ops go through the bridge, so an
// unmapped command or missing desktop app surfaces here as a desktop-only
// hint instead of failing silently.
export default function CloudProjectPanel() {
  const projects = useCloudStore((s) => s.projects);
  const projectId = useCloudStore((s) => s.projectId);
  const token = useCloudStore((s) => s.token);
  const cloudFiles = useCloudStore((s) => s.cloudFiles);
  const loadingProjects = useCloudStore((s) => s.loadingProjects);
  const filesLoading = useCloudStore((s) => s.filesLoading);
  const openingPath = useCloudStore((s) => s.openingPath);
  const error = useCloudStore((s) => s.error);
  const unavailable = useCloudStore((s) => s.unavailable);
  const refreshProjects = useCloudStore((s) => s.refreshProjects);
  const createProject = useCloudStore((s) => s.createProject);
  const deleteProject = useCloudStore((s) => s.deleteProject);
  const selectProject = useCloudStore((s) => s.selectProject);
  const openCloudFile = useCloudStore((s) => s.openCloudFile);

  const dirtyCount = useStore((s) => s.openFiles.reduce((n, f) => n + (f.modified ? 1 : 0), 0));

  const [newName, setNewName] = useState("");

  const active = projects.find((p) => p.id === projectId);

  return (
    <div className="cld-panel">
      {error && (
        <Callout tone={unavailable ? "info" : "danger"}>
          {unavailable
            ? "Desktop-only feature: this command needs the Voltrus PLC desktop app"
            : error}
        </Callout>
      )}
      {dirtyCount > 0 && (
        <Callout tone="warn">
          {dirtyCount} unsaved change{dirtyCount === 1 ? "" : "s"}. Cmd+S saves to the active cloud project.
        </Callout>
      )}

      {!token ? (
        <div className="cld-hint">Sign in to open cloud projects.</div>
      ) : (
        <>
          <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
            <span style={{ fontWeight: 600, color: "var(--text)", minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
              {active ? `Active: ${active.name}` : "No active project"}
            </span>
            <Button
              size="sm"
              variant="ghost"
              icon={<RefreshCw size={14} strokeWidth={1.75} />}
              style={{ marginLeft: "auto" }}
              onClick={() => void refreshProjects()}
              disabled={loadingProjects}
            >
              Refresh
            </Button>
          </div>

          {projects.length === 0 && !loadingProjects && (
            <EmptyState
              compact
              title="No cloud projects yet."
              description="Create one below to sync programs across devices."
            />
          )}
          <ul className="cld-list file-list">
            {projects.map((p) => (
              <li
                key={p.id}
                className={`file-item ${p.id === projectId ? "active" : ""}`}
                style={{ justifyContent: "space-between" }}
              >
                <span title={`${p.fileCount} file(s)`} style={{ minWidth: 0 }}>
                  {p.name}
                  <span className="cld-count">{p.fileCount}</span>
                </span>
                <span style={{ display: "flex", gap: 4 }}>
                  <Button
                    size="sm"
                    onClick={() => void selectProject(p.id)}
                    disabled={filesLoading}
                  >
                    Open
                  </Button>
                  <Button
                    size="sm"
                    variant="danger"
                    onClick={() => void deleteProject(p.id)}
                  >
                    Delete
                  </Button>
                </span>
              </li>
            ))}
          </ul>

          <div className="cld-form-row">
            <input
              type="text"
              className="vt-input vt-input--sm cld-grow"
              placeholder="New project name"
              aria-label="New project name"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
            />
            <Button
              size="sm"
              variant="primary"
              disabled={newName.trim().length === 0}
              onClick={() => {
                void createProject(newName.trim());
                setNewName("");
              }}
            >
              Create
            </Button>
          </div>

          {projectId && (
            <>
              <div className="cld-panel__title" style={{ marginTop: 4 }}>Files</div>
              {filesLoading ? (
                <div className="cld-hint">Loading files…</div>
              ) : cloudFiles.length === 0 ? (
                <EmptyState
                  compact
                  title="No files in this project"
                  description="Save an open program with Cmd+S to add it here."
                />
              ) : (
                <ul className="cld-list file-list">
                  {cloudFiles.map((f) => (
                    <li
                      key={f.path}
                      className="file-item"
                      role="button"
                      onClick={() => void openCloudFile(f.path)}
                    >
                      <span>{f.name}</span>
                      {openingPath === f.path && (
                        <span className="cld-hint" style={{ marginLeft: "auto" }}>Opening…</span>
                      )}
                    </li>
                  ))}
                </ul>
              )}
            </>
          )}
        </>
      )}
    </div>
  );
}
