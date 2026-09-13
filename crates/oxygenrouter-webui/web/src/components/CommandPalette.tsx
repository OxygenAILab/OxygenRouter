import React, { useEffect, useMemo, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useQueryClient } from "@tanstack/react-query";
import {
  Activity,
  BarChart3,
  Eraser,
  FileText,
  Info,
  Key,
  LucideIcon,
  MessageSquare,
  Plus,
  RefreshCw,
  Route as RouteIcon,
  Search,
  Server,
  Settings,
  CreditCard,
  Terminal,
} from "lucide-react";
import { useI18n } from "../lib/i18nContext";
import Dialog from "./ui/Dialog";

const BASE = "/ui";

export interface CommandPaletteProps {
  open: boolean;
  onClose: () => void;
}

type Item = {
  id: string;
  label: string;
  category: "navigation" | "actions";
  icon: LucideIcon;
  keywords?: string[];
  perform: () => void;
};

export default function CommandPalette({ open, onClose }: CommandPaletteProps) {
  const { t } = useI18n();
  const navigate = useNavigate();
  const qc = useQueryClient();
  const [query, setQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  const items: Item[] = useMemo(
    () => [
      {
        id: "nav.overview",
        label: t.nav.overview,
        category: "navigation",
        icon: Activity,
        keywords: ["home", "dashboard"],
        perform: () => {
          navigate(`${BASE}/overview`);
          onClose();
        },
      },
      {
        id: "nav.analytics",
        label: t.nav.analytics,
        category: "navigation",
        icon: BarChart3,
        keywords: ["stats", "metrics", "models"],
        perform: () => {
          navigate(`${BASE}/analytics`);
          onClose();
        },
      },
      {
        id: "nav.channels",
        label: t.nav.channels,
        category: "navigation",
        icon: Server,
        keywords: ["upstream", "providers"],
        perform: () => {
          navigate(`${BASE}/channels`);
          onClose();
        },
      },
      {
        id: "nav.keys",
        label: t.nav.keys,
        category: "navigation",
        icon: Key,
        keywords: ["api", "token", "secret"],
        perform: () => {
          navigate(`${BASE}/keys`);
          onClose();
        },
      },
      {
        id: "nav.routes",
        label: t.nav.routes,
        category: "navigation",
        icon: RouteIcon,
        keywords: ["routing", "rules", "rewrite"],
        perform: () => {
          navigate(`${BASE}/routes`);
          onClose();
        },
      },
      {
        id: "nav.logs",
        label: t.nav.logs,
        category: "navigation",
        icon: FileText,
        keywords: ["requests", "history"],
        perform: () => {
          navigate(`${BASE}/logs`);
          onClose();
        },
      },
      {
        id: "nav.playground",
        label: t.nav.playground,
        category: "navigation",
        icon: MessageSquare,
        keywords: ["chat", "test"],
        perform: () => {
          navigate(`${BASE}/playground`);
          onClose();
        },
      },
      {
        id: "nav.system",
        label: t.nav.system,
        category: "navigation",
        icon: Info,
        keywords: ["info", "runtime"],
        perform: () => {
          navigate(`${BASE}/system`);
          onClose();
        },
      },
      {
        id: "nav.settings",
        label: t.nav.settings,
        category: "navigation",
        icon: Settings,
        keywords: ["config", "preferences"],
        perform: () => {
          navigate(`${BASE}/settings`);
          onClose();
        },
      },
      {
        id: "nav.wallet",
        label: t.nav.wallet,
        category: "navigation",
        icon: CreditCard,
        keywords: ["billing", "balance", "credits"],
        perform: () => { navigate(`${BASE}/wallet`); onClose(); },
      },
      {
        id: "nav.plans",
        label: t.nav.plans,
        category: "navigation",
        icon: CreditCard,
        keywords: ["billing", "pricing", "quota"],
        perform: () => { navigate(`${BASE}/plans`); onClose(); },
      },
      {
        id: "action.addChannel",
        label: t.commandPalette.addChannel,
        category: "actions",
        icon: Plus,
        keywords: ["create", "new", "channel", "provider"],
        perform: () => {
          navigate(`${BASE}/channels`);
          onClose();
        },
      },
      {
        id: "action.addKey",
        label: t.commandPalette.addKey,
        category: "actions",
        icon: Plus,
        keywords: ["create", "new", "key", "api"],
        perform: () => {
          navigate(`${BASE}/keys`);
          onClose();
        },
      },
      {
        id: "action.addRoute",
        label: t.commandPalette.addRoute,
        category: "actions",
        icon: Plus,
        keywords: ["create", "new", "route", "rule", "map"],
        perform: () => {
          navigate(`${BASE}/routes`);
          onClose();
        },
      },
      {
        id: "action.clearLogs",
        label: t.commandPalette.clearLogs,
        category: "actions",
        icon: Eraser,
        keywords: ["delete", "reset", "logs"],
        perform: async () => {
          try {
            const res = await fetch("/api/logs", { method: "DELETE" });
            const json = await res.json();
            if (json.success) {
              qc.invalidateQueries({ queryKey: ["logs"] });
              qc.invalidateQueries({ queryKey: ["dashboard"] });
            }
          } catch {
            // ignore
          }
          onClose();
        },
      },
      {
        id: "action.refreshAll",
        label: t.commandPalette.refreshAll,
        category: "actions",
        icon: RefreshCw,
        keywords: ["reload", "sync"],
        perform: () => {
          qc.invalidateQueries();
          onClose();
        },
      },
    ],
    [t, navigate, onClose, qc],
  );

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return items;
    return items.filter((item) => {
      if (item.label.toLowerCase().includes(q)) return true;
      if (item.id.toLowerCase().includes(q)) return true;
      if (item.keywords?.some((k) => k.toLowerCase().includes(q))) return true;
      return false;
    });
  }, [items, query]);

  useEffect(() => {
    if (open) {
      setQuery("");
      setActiveIndex(0);
      setTimeout(() => inputRef.current?.focus(), 0);
    }
  }, [open]);

  useEffect(() => {
    setActiveIndex(0);
  }, [query]);

  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setActiveIndex((i) => Math.min(i + 1, filtered.length - 1));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        setActiveIndex((i) => Math.max(i - 1, 0));
      } else if (e.key === "Enter") {
        e.preventDefault();
        const item = filtered[activeIndex];
        if (item) item.perform();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, filtered, activeIndex]);

  useEffect(() => {
    if (!open) return;
    const active = listRef.current?.querySelector(".command-palette-item.is-active") as HTMLElement | null;
    if (active) active.scrollIntoView({ block: "nearest" });
  }, [activeIndex, open]);

  if (!open) return null;

  const navItems = filtered.filter((i) => i.category === "navigation");
  const actionItems = filtered.filter((i) => i.category === "actions");

  const renderItem = (item: Item) => {
    const Icon = item.icon;
    const flatIndex = filtered.indexOf(item);
    return (
      <button
        key={item.id}
        className={`command-palette-item ${flatIndex === activeIndex ? "is-active" : ""}`}
        onMouseEnter={() => setActiveIndex(flatIndex)}
        onClick={() => item.perform()}
      >
        <Icon size={14} />
        <span>{item.label}</span>
      </button>
    );
  };

  return (
    <Dialog open={open} onClose={onClose} title={t.commandPalette.placeholder} size="md" className="command-palette">
        <div className="command-palette-search">
          <Terminal size={14} style={{ color: "var(--md-on-surface-variant)" }} />
          <Search size={14} style={{ color: "var(--md-on-surface-variant)" }} />
          <input
            ref={inputRef}
            data-dialog-autofocus
            type="text"
            placeholder={t.commandPalette.placeholder}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
          <kbd className="command-palette-kbd">esc</kbd>
        </div>
        <div className="command-palette-list" ref={listRef}>
          {filtered.length === 0 ? (
            <div className="command-palette-empty">{t.commandPalette.noResults}</div>
          ) : (
            <>
              {navItems.length > 0 && (
                <div className="command-palette-group">
                  <div className="command-palette-group-label">{t.commandPalette.navigation}</div>
                  {navItems.map(renderItem)}
                </div>
              )}
              {actionItems.length > 0 && (
                <div className="command-palette-group">
                  <div className="command-palette-group-label">{t.commandPalette.actions}</div>
                  {actionItems.map(renderItem)}
                </div>
              )}
            </>
          )}
        </div>
    </Dialog>
  );
}
