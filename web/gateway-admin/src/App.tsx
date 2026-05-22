import { BrowserRouter, Navigate, Route, Routes } from "react-router-dom";
import { AppProvider } from "./context/AppContext";
import AdminLayout from "./layout/AdminLayout";
import LoginPage from "./pages/LoginPage";
import RequireAuth from "./auth/RequireAuth";
import ProjectPage from "./pages/ProjectPage";
import SkillsPage from "./pages/SkillsPage";
import McpPage from "./pages/McpPage";
import ClaudePage from "./pages/ClaudePage";
import RulesPage from "./pages/RulesPage";
import PromptPage from "./pages/PromptPage";
import ToolsPage from "./pages/ToolsPage";
import GlobalSettingsPage from "./pages/GlobalSettingsPage";

export default function App() {
  return (
    <BrowserRouter basename="/admin">
      <Routes>
        <Route path="/login" element={<LoginPage />} />
        <Route
          element={
            <RequireAuth>
              <AppProvider>
                <AdminLayout />
              </AppProvider>
            </RequireAuth>
          }
        >
          <Route index element={<ProjectPage />} />
          <Route path="skills" element={<SkillsPage />} />
          <Route path="mcp" element={<McpPage />} />
          <Route path="claude" element={<ClaudePage />} />
          <Route path="rules" element={<RulesPage />} />
          <Route path="prompt" element={<PromptPage />} />
          <Route path="tools" element={<ToolsPage />} />
          <Route path="global" element={<GlobalSettingsPage />} />
        </Route>
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </BrowserRouter>
  );
}
