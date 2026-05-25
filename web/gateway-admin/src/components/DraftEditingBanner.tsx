/** Shown on entity editor pages when project has an open draft. Author: kejiqing */

import { Alert } from "antd";
import { useApp } from "../context/AppContext";
import { formatVersionTime } from "../utils/versionDisplay";

export default function DraftEditingBanner() {
  const { projectConfig } = useApp();
  if (!projectConfig?.draftOpen) return null;
  const stable = projectConfig.stableContentRev;
  return (
    <Alert
      type="info"
      showIcon
      style={{ marginBottom: 12 }}
      message="正在编辑草稿（未生效）"
      description={
        stable
          ? `当前 solve 仍使用正式版 ${formatVersionTime(stable)}；保存到各 Tab 会写入草稿，需在「项目」页「保存为正式版」并「设为生效」后才会用于 solve。`
          : "当前修改写入草稿；需在「项目」页「保存为正式版」并「设为生效」后才会用于 solve。"
      }
    />
  );
}
