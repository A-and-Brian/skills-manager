import { Navigate, createRootRoute, createRoute, createRouter } from "@tanstack/react-router";
import { AppProvider } from "./context/AppContext";
import { HelpDialog } from "./components/HelpDialog";
import { CloseActionGuard } from "./components/CloseActionGuard";
import { FirstRunRestoreDialog } from "./components/FirstRunRestoreDialog";
import { RemotePickerHost } from "./components/RemoteDirectoryPicker";
import { Layout } from "./components/Layout";
import { Dashboard } from "./views/Dashboard";
import { MySkills } from "./views/MySkills";
import { WorkspaceView } from "./views/WorkspaceView";
import { CODING_WORKSPACE_CONFIG, LOBSTER_WORKSPACE_CONFIG } from "./views/workspaceConfigs";
import { InstallSkills } from "./views/InstallSkills";
import { parseInstallSearch } from "./views/installSearch";
import { Settings } from "./views/Settings";
import { ProjectDetail } from "./views/ProjectDetail";
import { Backup } from "./views/Backup";

const rootRoute = createRootRoute({
  component: () => (
    <AppProvider>
      <Layout />
      <HelpDialog />
      <CloseActionGuard />
      <FirstRunRestoreDialog />
      <RemotePickerHost />
    </AppProvider>
  ),
  notFoundComponent: () => <Navigate to="/" replace />,
});

// Each route is its own const: routes created inline in `addChildren` lose their
// literal paths, and every `to`, `from` and `params` then type-checks as any string.
const dashboardRoute = createRoute({ getParentRoute: () => rootRoute, path: "/", component: Dashboard });

const mySkillsRoute = createRoute({ getParentRoute: () => rootRoute, path: "/my-skills", component: MySkills });

const globalWorkspaceRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/global-workspace/{-$agentKey}",
  component: () => <WorkspaceView config={CODING_WORKSPACE_CONFIG} />,
});

const lobsterWorkspaceRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/lobster-workspace/{-$agentKey}",
  component: () => <WorkspaceView config={LOBSTER_WORKSPACE_CONFIG} />,
});

const installRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/install",
  validateSearch: parseInstallSearch,
  component: InstallSkills,
});

const backupRoute = createRoute({ getParentRoute: () => rootRoute, path: "/backup", component: Backup });

const projectRoute = createRoute({ getParentRoute: () => rootRoute, path: "/project/$id", component: ProjectDetail });

const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings/{-$category}",
  component: Settings,
});

const routeTree = rootRoute.addChildren([
  dashboardRoute,
  mySkillsRoute,
  globalWorkspaceRoute,
  lobsterWorkspaceRoute,
  installRoute,
  backupRoute,
  projectRoute,
  settingsRoute,
]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
