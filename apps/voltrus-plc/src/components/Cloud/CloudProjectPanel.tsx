import { useState } from "react";
import { useCloudStore } from "../../store/cloud";
import { useStore } from "../../store";

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
    <div style={{ padding: "0 12px 10px", display: "flex", flexDirection: "column", gap: 6, fontSize: 11 }}>
      {error && (
        <div role="alert" style={{ color: "var(--danger)" }}>
          {unavailable ? "Desktop-only feature — this command needs the Voltrus PLC desktop app" : error}
        </div>
      )}
      {dirtyCount > 0 && (
        <div style={{ color: "var(--amber)" }}>
          {dirtyCount} unsaved change{dirtyCount === 1 ? "" : "s"} — Cmd+S saves to the active cloud project
        </div>
      )}

      {!token ? (
        <div style={{ color: "var(--text-muted)" }}>Sign in to open cloud projects.</div>
      ) : (
        <>
          <div style={{ display: "flex", alignItems: "center" }}>
            <span style={{ fontWeight: 600 }}>
              {active ? `Active: ${active.name}` : "No active project"}
            </span>
            <button
              className="toolbar-btn"
              style={{ marginLeft: "auto", fontSize: 10, padding: "1px 6px" }}
              onClick={() => void refreshProjects()}
              disabled={loadingProjects}
            >
              {loadingProjects ? "…" : "Refresh"}
            </button>
          </div>

          {projects.length === 0 && !loadingProjects && (
            <div style={{ color: "var(--text-muted)" }}>No cloud projects yet.</div>
          )}
          <ul className="file-list">
            {projects.map((p) => (
              <li
                key={p.id}
                className={`file-item ${p.id === projectId ? "active" : ""}`}
                style={{ justifyContent: "space-between" }}
              >
                <span title={`${p.fileCount} file(s)`}>
                  {p.name}
                  <span style={{ color: "var(--text-muted)", marginLeft: 6 }}>{p.fileCount}</span>
                </span>
                <span style={{ display: "flex", gap: 4 }}>
                  <button
                    className="toolbar-btn"
                    style={{ fontSize: 10, padding: "0 6px" }}
                    onClick={() => void selectProject(p.id)}
                    disabled={filesLoading}
                  >
                    Open
                  </button>
                  <button
                    className="toolbar-btn danger"
                    style={{ fontSize: 10, padding: "0 6px" }}
                    onClick={() => void deleteProject(p.id)}
                  >
                    Delete
                  </button>
                </span>
              </li>
            ))}
          </ul>

          <div style={{ display: "flex", gap: 4 }}>
            <input
              type="text"
              placeholder="New project name"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              style={{ flex: 1, fontSize: 11, padding: "2px 4px" }}
            />
            <button
              className="toolbar-btn"
              style={{ fontSize: 10, padding: "0 8px" }}
              disabled={newName.trim().length === 0}
              onClick={() => {
                void createProject(newName.trim());
                setNewName("");
              }}
            >
              Create
            </button>
          </div>

          {projectId && (
            <>
              <div style={{ fontWeight: 600, marginTop: 4 }}>Files</div>
              {filesLoading ? (
                <div style={{ color: "var(--text-muted)" }}>Loading files…</div>
              ) : cloudFiles.length === 0 ? (
                <div style={{ color: "var(--text-muted)" }}>No files in this project.</div>
              ) : (
                <ul className="file-list">
                  {cloudFiles.map((f) => (
                    <li
                      key={f.path}
                      className="file-item"
                      role="button"
                      onClick={() => void openCloudFile(f.path)}
                    >
                      <span>{f.name}</span>
                      {openingPath === f.path && (
                        <span style={{ color: "var(--text-muted)", marginLeft: "auto" }}>Opening…</span>
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
