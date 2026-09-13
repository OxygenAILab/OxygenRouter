import React from "react";
import { Routes, Route, Navigate, Link, useLocation } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import {
  Activity,
  BarChart3,
  ChevronRight,
  Cpu,
  FileText,
  Info,
  Key,
  MessageSquare,
  Menu,
  PanelLeftClose,
  PanelLeftOpen,
  Route as RouteIcon,
  Server,
  Settings,
  SlidersHorizontal,
  Sparkles,
  CreditCard,
  ShieldCheck,
  UserRound,
  X,
} from "lucide-react";
import { I18nProvider, useI18n } from "./lib/i18nContext";
import { ThemeProvider, useTheme } from "./lib/themeContext";
import { ToastProvider } from "./lib/toast";
import { AuthProvider, useAuth } from "./lib/auth";
import { api } from "./lib/api";
import CommandPalette from "./components/CommandPalette";
import ChannelsPage from "./pages/ChannelsPage";
import OverviewPage from "./pages/OverviewPage";
import AnalyticsPage from "./pages/AnalyticsPage";
import KeysPage from "./pages/KeysPage";
import LogsPage from "./pages/LogsPage";
import ModelsPage from "./pages/ModelsPage";
import RoutesPage from "./pages/RoutesPage";
import SettingsPage from "./pages/SettingsPage";
import SystemInfoPage from "./pages/SystemInfoPage";
import PlaygroundPage from "./pages/PlaygroundPage";
import ModelMetadataPage from "./pages/ModelMetadataPage";
import SystemSettingsPage from "./pages/SystemSettingsPage";
import AuthPage from "./pages/AuthPage";
import WalletPage from "./pages/WalletPage";
import PlansPage from "./pages/PlansPage";
import SubscriptionsPage from "./pages/SubscriptionsPage";
import AdminUsersPage from "./pages/AdminUsersPage";
import AdminBillingPage from "./pages/AdminBillingPage";

const BASE = "/ui";
const COLLAPSE_KEY = "oxygenrouter:sidebar-collapsed";

function Sidebar({ open, onClose, collapsed, onToggle }: { open: boolean; onClose: () => void; collapsed: boolean; onToggle: () => void }) {
  const location = useLocation();
  const { t } = useI18n();
  const { data: channels } = useQuery({ queryKey: ["channels"], queryFn: api.channels.list, refetchInterval: 30_000 });
  const { data: keys } = useQuery({ queryKey: ["keys"], queryFn: api.keys.list, refetchInterval: 30_000 });
  const { data: maps } = useQuery({ queryKey: ["modelMaps"], queryFn: api.modelMaps.list, refetchInterval: 30_000 });
  const { user } = useAuth();

  const enabledChannels = channels?.filter((c) => c.enabled).length ?? 0;
  const totalChannels = channels?.length ?? 0;
  const totalKeys = keys?.length ?? 0;
  const totalMaps = maps?.length ?? 0;

  const navItems = [
    { path: `${BASE}/overview`, label: t.nav.overview, icon: Activity, key: "overview", count: null as number | null },
    { path: `${BASE}/analytics`, label: t.nav.analytics, icon: BarChart3, key: "analytics", count: null },
    { path: `${BASE}/channels`, label: t.nav.channels, icon: Server, key: "channels", count: enabledChannels > 0 ? enabledChannels : null, total: totalChannels },
    { path: `${BASE}/keys`, label: t.nav.keys, icon: Key, key: "keys", count: totalKeys > 0 ? totalKeys : null },
    { path: `${BASE}/routes`, label: t.nav.routes, icon: RouteIcon, key: "routes", count: totalMaps > 0 ? totalMaps : null },
    { path: `${BASE}/logs`, label: t.nav.logs, icon: FileText, key: "logs", count: null },
    { path: `${BASE}/models`, label: t.nav.models, icon: Sparkles, key: "models", count: null },
    { path: `${BASE}/model-registry`, label: t.nav.modelsMeta, icon: Cpu, key: "model-registry", count: null },
    { path: `${BASE}/playground`, label: t.nav.playground, icon: MessageSquare, key: "playground", count: null },
    { path: `${BASE}/system`, label: t.nav.system, icon: Info, key: "system", count: null },
    { path: `${BASE}/system-settings`, label: t.nav.systemSettings, icon: SlidersHorizontal, key: "system-settings", count: null },
    { path: `${BASE}/settings`, label: t.nav.settings, icon: Settings, key: "settings", count: null },
    { path: `${BASE}/wallet`, label: t.nav.wallet, icon: CreditCard, key: "wallet", count: null },
    { path: `${BASE}/plans`, label: t.nav.plans, icon: Sparkles, key: "plans", count: null },
    { path: `${BASE}/subscriptions`, label: t.nav.subscriptions, icon: UserRound, key: "subscriptions", count: null },
  ];
  if (user?.role === "admin") navItems.push({ path: `${BASE}/admin/users`, label: t.nav.users, icon: UserRound, key: "admin-users", count: null }, { path: `${BASE}/admin/billing`, label: t.nav.billing, icon: ShieldCheck, key: "admin-billing", count: null });
  return (
    <aside className={`app-sidebar ${open ? "is-open" : ""} ${collapsed ? "is-collapsed" : ""}`}>
      <button className="mobile-close btn-ghost" onClick={onClose} aria-label="Close navigation">
        <X size={18} />
      </button>
      <div className="sidebar-brand">
        <div className="brand-mark">O₂</div>
        <div className="brand-copy">
          <div className="brand-name">{t.app.brand}</div>
          <div className="brand-version">{t.app.version}</div>
        </div>
        <button
          className="sidebar-toggle btn-ghost"
          onClick={onToggle}
          aria-label={collapsed ? t.app.expand : t.app.collapse}
          title={collapsed ? t.app.expand : t.app.collapse}
        >
          {collapsed ? <PanelLeftOpen size={16} /> : <PanelLeftClose size={16} />}
        </button>
      </div>
      <div className="sidebar-section-label">Workspace</div>
      <nav className="sidebar-nav">
        {navItems.map(({ path, label, icon: Icon, key, count, total }) => {
          const active = location.pathname === path || location.pathname.startsWith(path + "/");
          return (
            <Link
              key={key}
              to={path}
              onClick={onClose}
              className={`sidebar-link ${active ? "is-active" : ""}`}
              aria-label={label}
              title={collapsed ? label : undefined}
            >
              <span className="sidebar-link-icon">
                <Icon size={16} strokeWidth={1.8} />
              </span>
              <span className="sidebar-link-label">{label}</span>
              {count !== null && (
                <span className="sidebar-link-count" title={total ? `${count} / ${total}` : `${count}`}>
                  {count}
                </span>
              )}
              {active && <span className="sidebar-link-marker" aria-hidden />}
            </Link>
          );
        })}
      </nav>
      <div className="sidebar-footer">
        <span className="pulse-dot" />
        <span className="sidebar-footer-copy">{t.app.localMode}</span>
      </div>
    </aside>
  );
}

function PageHeader({ title, description, action }: { title: string; description?: string; action?: React.ReactNode }) {
  return (
    <div className="page-header">
      <div>
        <h1>{title}</h1>
        {description && <p>{description}</p>}
      </div>
      {action && <div>{action}</div>}
    </div>
  );
}

function TopBar() {
  const { t } = useI18n();
  const location = useLocation();
  const path = location.pathname.replace(BASE, "").replace(/^\//, "");
  const label = (path || "overview").split("/")[0];
  const labelMap: Record<string, string> = {
    overview: t.nav.overview,
    analytics: t.nav.analytics,
    channels: t.nav.channels,
    keys: t.nav.keys,
    routes: t.nav.routes,
    logs: t.nav.logs,
    models: t.nav.models,
    playground: t.nav.playground,
    system: t.nav.system,
    settings: t.nav.settings,
    wallet: t.nav.wallet,
    plans: t.nav.plans,
    subscriptions: t.nav.subscriptions,
    admin: t.nav.users,
  };
  const current = labelMap[label] || t.nav.overview;
  const { user, logout, loading } = useAuth();
  return (
    <header className="app-topbar">
      <div className="topbar-crumbs">
        <span className="topbar-crumb-root">{t.app.brand}</span>
        <ChevronRight size={12} />
        <span className="topbar-crumb-leaf">{current}</span>
      </div>
      <div className="topbar-spacer" />
      <div className="topbar-actions">
        <LanguageToggle />
        <ThemeToggle />
        {!loading && (user ? <button className="account-button" onClick={() => void logout()} title="Sign out"><span>{user.username}</span><small>{user.role}</small></button> : <Link className="btn-outlined topbar-signin" to="/ui/sign-in">Sign in</Link>)}
      </div>
    </header>
  );
}

function LanguageToggle() {
  const { t, locale, setLocale } = useI18n();
  return (
    <div className="lang-pill" role="group" aria-label={t.settings.language}>
      <button
        className={`lang-pill-btn ${locale === "en" ? "is-active" : ""}`}
        onClick={() => setLocale("en")}
        aria-pressed={locale === "en"}
      >
        EN
      </button>
      <button
        className={`lang-pill-btn ${locale === "zh" ? "is-active" : ""}`}
        onClick={() => setLocale("zh")}
        aria-pressed={locale === "zh"}
      >
        中
      </button>
    </div>
  );
}

function ThemeToggle() {
  const { theme, setTheme } = useTheme();
  return (
    <button
      className="theme-toggle btn-ghost"
      onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
      title={theme === "dark" ? "Switch to light" : "Switch to dark"}
    >
      {theme === "dark" ? (
        <svg width={16} height={16} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
          <circle cx="12" cy="12" r="4" />
          <path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M4.93 19.07l1.41-1.41M17.66 6.34l1.41-1.41" />
        </svg>
      ) : (
        <svg width={16} height={16} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
          <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79Z" />
        </svg>
      )}
    </button>
  );
}

function Shell() {
  const { t } = useI18n();
  const [sidebarOpen, setSidebarOpen] = React.useState(false);
  const [sidebarCollapsed, setSidebarCollapsed] = React.useState(
    () => localStorage.getItem(COLLAPSE_KEY) === "true",
  );
  const [paletteOpen, setPaletteOpen] = React.useState(false);
  const toggleSidebar = () =>
    setSidebarCollapsed((value) => {
      localStorage.setItem(COLLAPSE_KEY, String(!value));
      return !value;
    });
  React.useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && (e.key === "k" || e.key === "K")) {
        e.preventDefault();
        setPaletteOpen((v) => !v);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  return (
    <div className={`app-shell ${sidebarCollapsed ? "sidebar-is-collapsed" : ""}`}>
      <div
        className={`sidebar-backdrop ${sidebarOpen ? "is-visible" : ""}`}
        onClick={() => setSidebarOpen(false)}
      />
      <Sidebar
        open={sidebarOpen}
        onClose={() => setSidebarOpen(false)}
        collapsed={sidebarCollapsed}
        onToggle={toggleSidebar}
      />
      <div className="app-main">
        <TopBar />
        <button
          className="mobile-menu btn-ghost"
          onClick={() => setSidebarOpen(true)}
          aria-label={t.app.openNav}
        >
          <Menu size={18} />
        </button>
        <main className="content-container">
          <Routes>
            <Route path="/" element={<Navigate to="/ui/overview" replace />} />
            <Route path="/ui" element={<Navigate to="/ui/overview" replace />} />
            <Route path="/ui/overview" element={<OverviewPage />} />
            <Route path="/ui/analytics" element={<AnalyticsPage />} />
            <Route path="/ui/channels" element={<ChannelsPage />} />
            <Route path="/ui/keys" element={<KeysPage />} />
            <Route path="/ui/routes" element={<RoutesPage />} />
            <Route path="/ui/logs" element={<LogsPage />} />
            <Route path="/ui/models" element={<ModelsPage />} />
            <Route path="/ui/model-registry" element={<ModelMetadataPage />} />
            <Route path="/ui/system" element={<SystemInfoPage />} />
            <Route path="/ui/system-settings" element={<SystemSettingsPage />} />
            <Route path="/ui/playground" element={<PlaygroundPage />} />
            <Route path="/ui/settings" element={<SettingsPage />} />
            <Route path="/ui/sign-in" element={<AuthPage mode="sign-in" />} />
            <Route path="/ui/sign-up" element={<AuthPage mode="sign-up" />} />
            <Route path="/ui/wallet" element={<WalletPage />} />
            <Route path="/ui/plans" element={<PlansPage />} />
            <Route path="/ui/subscriptions" element={<SubscriptionsPage />} />
            <Route path="/ui/admin/users" element={<AdminUsersPage />} />
            <Route path="/ui/admin/billing" element={<AdminBillingPage />} />
            <Route path="*" element={<Navigate to="/ui/overview" replace />} />
          </Routes>
        </main>
      </div>
      <CommandPalette open={paletteOpen} onClose={() => setPaletteOpen(false)} />
    </div>
  );
}

function App() {
  return (
    <I18nProvider>
      <ThemeProvider>
        <ToastProvider>
          <AuthProvider><Shell /></AuthProvider>
        </ToastProvider>
      </ThemeProvider>
    </I18nProvider>
  );
}

export { PageHeader };
export default App;
