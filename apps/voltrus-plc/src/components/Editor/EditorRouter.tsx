import StEditor from "./StEditor";
import FbdEditor from "./FbdEditor/FbdEditor";
import GridLdEditor from "./LdEditor/GridLdEditor";
import SfcEditor from "./SfcEditor/SfcEditor";
import OnlineDiagnostics from "../S7/OnlineDiagnostics";
import TagTableEditor from "../S7/TagTableEditor";
import WatchTableEditor from "../S7/WatchTableEditor";
import { isTagDoc, isWatchDoc, ONLINE_PATH } from "../../lib/s7Docs";
import "./editor.css";

interface EditorRouterProps {
  filePath: string;
  content: string;
  onChange: (value: string) => void;
}

export default function EditorRouter({ filePath, content, onChange }: EditorRouterProps) {
  if (filePath === ONLINE_PATH) {
    return <OnlineDiagnostics />;
  }
  if (isWatchDoc(filePath)) {
    return <WatchTableEditor path={filePath} value={content} onChange={onChange} />;
  }
  if (isTagDoc(filePath)) {
    return <TagTableEditor path={filePath} value={content} onChange={onChange} />;
  }
  if (filePath.endsWith(".sfc")) {
    return <SfcEditor value={content} onChange={onChange} />;
  }
  if (filePath.endsWith(".ld")) {
    return <GridLdEditor value={content} onChange={onChange} />;
  }
  if (filePath.endsWith(".fbd")) {
    return <FbdEditor value={content} onChange={onChange} />;
  }
  return <StEditor value={content} onChange={onChange} filePath={filePath} />;
}
