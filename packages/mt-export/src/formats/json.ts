import type { ExportFormat, TagList } from "../model";
import { fileStem, toJson } from "../util";

/** The tag list as it is, with a format marker and version so a later reader can tell what it got. */
export const jsonFormat: ExportFormat = {
  id: "json",
  label: "Generic JSON",
  hint: "The whole tag list: device settings and every tag",
  extension: "json",
  protocols: ["modbus", "bacnet"],
  export(list: TagList) {
    const { source } = list;
    return {
      fileName: `${fileStem(source.name)}-tags.json`,
      content: toJson({ format: "voltrus-tag-list", version: 1, protocol: list.protocol, device: source, tags: list.tags }),
      warnings: [],
    };
  },
};
