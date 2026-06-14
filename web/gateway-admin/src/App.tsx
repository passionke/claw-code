import { BrowserRouter, Navigate, Outlet, Route, Routes } from "react-router-dom";
import { AppProvider } from "./context/AppContext";
import { ChatSessionProvider } from "./context/ChatSessionContext";
import AdminLayout from "./layout/AdminLayout";
import ChatLayout from "./layout/ChatLayout";
import LoginPage from "./pages/LoginPage";
import ChatAuthGate from "./auth/ChatAuthGate";
import RequireAuth from "./auth/RequireAuth";
import ProjectPage from "./pages/ProjectPage";
import SkillsPage from "./pages/SkillsPage";
import McpPage from "./pages/McpPage";
import ClaudePage from "./pages/ClaudePage";
import RulesPage from "./pages/RulesPage";
import PromptPage from "./pages/PromptPage";
import ToolsPage from "./pages/ToolsPage";
import ExtraSessionPage from "./pages/ExtraSessionPage";
import PreflightPage from "./pages/PreflightPage";
import WorkerIsolationPage from "./pages/WorkerIsolationPage";
import GlobalSettingsRedirect from "./pages/GlobalSettingsRedirect";
import GitPatsPage from "./pages/global/GitPatsPage";
import GlobalInferencePage from "./pages/global/GlobalInferencePage";
import GlobalPoolsPage from "./pages/global/GlobalPoolsPage";
import ChatPage from "./pages/ChatPage";

export default function App() {
  return (
    <BrowserRouter basename="/admin">
      <Routes>
        <Route path="/login" element={<LoginPage />} />
        <Route
          element={
            <AppProvider>
              <Outlet />
            </AppProvider>
          }
        >
          <Route
            path="chat"
            element={
              <ChatAuthGate>
                <ChatSessionProvider>
                  <ChatLayout />
                </ChatSessionProvider>
              </ChatAuthGate>
            }
          >
            <Route index element={<ChatPage />} />
          </Route>
          <Route
            element={
              <RequireAuth>
                <Outlet />
              </RequireAuth>
            }
          >
            <Route element={<AdminLayout />}>
              <Route index element={<ProjectPage />} />
              <Route path="skills" element={<SkillsPage />} />
              <Route path="mcp" element={<McpPage />} />
              <Route path="claude" element={<ClaudePage />} />
              <Route path="rules" element={<RulesPage />} />
              <Route path="prompt" element={<PromptPage />} />
              <Route path="tools" element={<ToolsPage />} />
              <Route path="extra-session" element={<ExtraSessionPage />} />
              <Route path="preflight" element={<PreflightPage />} />
              <Route path="worker-isolation" element={<WorkerIsolationPage />} />
              <Route path="global" element={<GlobalSettingsRedirect />} />
              <Route path="global/inference" element={<GlobalInferencePage />} />
              <Route path="global/models" element={<Navigate to="/global/inference" replace />} />
              <Route path="global/claw-tap" element={<Navigate to="/global/inference" replace />} />
              <Route path="global/pats" element={<GitPatsPage />} />
              <Route path="global/pools" element={<GlobalPoolsPage />} />
            </Route>
          </Route>
        </Route>
        <Route path="*" element={<Navigate to="/chat" replace />} />
      </Routes>
    </BrowserRouter>
  );
}
