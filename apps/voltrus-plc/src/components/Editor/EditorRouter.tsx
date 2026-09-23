import StEditor from "./StEditor";
import FbdEditor from "./FbdEditor/FbdEditor";
import GridLdEditor from "./LdEditor/GridLdEditor";
import SfcEditor from "./SfcEditor/SfcEditor";

interface EditorRouterProps {
  filePath: string;
  content: string;
  onChange: (value: string) => void;
}

export default function EditorRouter({ filePath, content, onChange }: EditorRouterProps) {
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
