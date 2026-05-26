import { Navigate } from "react-router-dom";

/** 全局配置默认进入模型列表。Author: kejiqing */
export default function GlobalSettingsRedirect() {
  return <Navigate to="/global/models" replace />;
}
