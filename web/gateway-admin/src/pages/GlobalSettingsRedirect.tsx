import { Navigate } from "react-router-dom";

/** 全局配置默认进入全局推理。Author: kejiqing */
export default function GlobalSettingsRedirect() {
  return <Navigate to="/global/inference" replace />;
}
