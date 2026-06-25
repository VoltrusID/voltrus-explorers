import type { OpenFile } from "../../types";

interface EditorTabBarProps {
  files: OpenFile[];
  activeIndex: number;
  onSelect: (index: number) => void;
  onClose: (index: number) => void;
}

export default function EditorTabBar({
  files,
  activeIndex,
  onSelect,
  onClose,
}: EditorTabBarProps) {
  return (
    <div className="editor-tabs">
      {files.map((file, i) => (
        <button
          key={`${file.path}-${i}`}
          className={`editor-tab ${i === activeIndex ? "active" : ""}`}
          onClick={() => onSelect(i)}
        >
          {file.modified && <span className="dot" />}
          {file.name}
          <span
            className="close-btn"
            onClick={(e) => {
              e.stopPropagation();
              onClose(i);
            }}
          >
            x
          </span>
        </button>
      ))}
    </div>
  );
}
