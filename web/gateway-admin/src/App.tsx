import { BrowserRouter, Navigate, Outlet, Route, Routes } from "react-router-dom";
import { AppProvider } from "./context/AppContext";
import { ChatSessionProvider } from "./context/ChatSessionContext";
import AdminLayout from "./layout/AdminLayout";
import ChatLayout from "./layout/ChatLayout";
import LoginPage from "./pages/LoginPage";
import RequireAuth from "./auth/RequireAuth";
import ProjectPage from "./pages/ProjectPage";
import SkillsPage from "./pages/SkillsPage";
import McpPage from "./pages/McpPage";
import ClaudePage from "./pages/ClaudePage";
import RulesPage from "./pages/RulesPage";
import PromptPage from "./pages/PromptPage";
import ToolsPage from "./pages/ToolsPage";
import PreflightPage from "./pages/PreflightPage";
import GlobalSettingsRedirect from "./pages/GlobalSettingsRedirect";
import GitPatsPage from "./pages/global/GitPatsPage";
import LlmModelsPage from "./pages/global/LlmModelsPage";
import ChatPage from "./pages/ChatPage";

export default function App() {
  return (
    <BrowserRouter basename="/admin">
      <Routes>
        <Route path="/login" element={<LoginPage />} />
        <Route
          element={
            <RequireAuth>
              <AppProvider>
                <Outlet />
              </AppProvider>
            </RequireAuth>
          }
        >
          <Route
            path="chat"
            element={
              <ChatSessionProvider>
                <ChatLayout />
              </ChatSessionProvider>
            }
          >
            <Route index element={<ChatPage />} />
          </Route>
          <Route element={<AdminLayout />}>
            <Route index element={<ProjectPage />} />
            <Route path="skills" element={<SkillsPage />} />
            <Route path="mcp" element={<McpPage />} />
            <Route path="claude" element={<ClaudePage />} />
            <Route path="rules" element={<RulesPage />} />
            <Route path="prompt" element={<PromptPage />} />
            <Route path="tools" element={<ToolsPage />} />
            <Route path="preflight" element={<PreflightPage />} />
            <Route path="global" element={<GlobalSettingsRedirect />} />
            <Route path="global/models" element={<LlmModelsPage />} />
            <Route path="global/pats" element={<GitPatsPage />} />
          </Route>
        </Route>
        <Route path="*" element={<Navigate to="/chat" replace />} />
      </Routes>
    </BrowserRouter>
  );
}
