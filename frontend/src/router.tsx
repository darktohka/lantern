import {
  Link,
  Navigate,
  Outlet,
  createRootRoute,
  createRoute,
  createRouter,
  useLocation,
} from "@tanstack/react-router";
import { Flame, LogOut } from "lucide-react";
import type { ReactNode } from "react";

import { Button } from "./components/ui/button";
import { useAuth } from "./lib/auth";
import { AccountsPage } from "./routes/accounts";
import { InvitesPage } from "./routes/invites";
import { LoginPage } from "./routes/login";
import { LogsPage } from "./routes/logs";
import { RegisterPage } from "./routes/register";

function AppShell() {
  const { user, logout } = useAuth();
  const location = useLocation();
  const isAuthPage =
    location.pathname === "/login" || location.pathname === "/register";

  return (
    <div className="min-h-screen">
      {!isAuthPage && user ? (
        <header className="border-b border-border bg-card/90 backdrop-blur">
          <div className="mx-auto flex max-w-6xl items-center justify-between gap-4 px-4 py-3">
            <Link to="/accounts" className="flex items-center gap-2 font-semibold">
              <span className="flex h-9 w-9 items-center justify-center rounded-md bg-primary text-primary-foreground">
                <Flame className="h-5 w-5" />
              </span>
              Lantern
            </Link>
            <nav className="flex flex-1 items-center gap-1 overflow-x-auto px-2">
              <NavLink to="/accounts">Accounts</NavLink>
              <NavLink to="/invites">Invites</NavLink>
              <NavLink to="/logs">Logs</NavLink>
            </nav>
            <div className="flex items-center gap-3">
              <span className="hidden text-sm text-muted-foreground sm:inline">
                {user.username}
              </span>
              <Button variant="ghost" size="icon" onClick={() => void logout()}>
                <LogOut className="h-4 w-4" />
                <span className="sr-only">Log out</span>
              </Button>
            </div>
          </div>
        </header>
      ) : null}
      <Outlet />
    </div>
  );
}

function NavLink({ to, children }: { to: string; children: ReactNode }) {
  return (
    <Link
      to={to}
      className="rounded-md px-3 py-2 text-sm font-medium text-muted-foreground transition-colors hover:bg-muted hover:text-foreground [&.active]:bg-muted [&.active]:text-foreground"
      activeProps={{ className: "active" }}
    >
      {children}
    </Link>
  );
}

function Protected({ children }: { children: ReactNode }) {
  const { user, loading } = useAuth();

  if (loading) {
    return (
      <main className="mx-auto flex min-h-[60vh] max-w-6xl items-center justify-center px-4">
        <div className="text-sm text-muted-foreground">Loading...</div>
      </main>
    );
  }

  if (!user) return <Navigate to="/login" />;

  return <>{children}</>;
}

const rootRoute = createRootRoute({
  component: AppShell,
});

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: () => <Navigate to="/accounts" />,
});

const loginRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/login",
  component: LoginPage,
});

const registerRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/register",
  component: RegisterPage,
});

const accountsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/accounts",
  component: () => (
    <Protected>
      <AccountsPage />
    </Protected>
  ),
});

const invitesRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/invites",
  component: () => (
    <Protected>
      <InvitesPage />
    </Protected>
  ),
});

const logsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/logs",
  component: () => (
    <Protected>
      <LogsPage />
    </Protected>
  ),
});

const routeTree = rootRoute.addChildren([
  indexRoute,
  loginRoute,
  registerRoute,
  accountsRoute,
  invitesRoute,
  logsRoute,
]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
