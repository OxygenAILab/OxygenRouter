import React from "react";
import { api, setSessionToken, type User } from "./api";

const STORAGE_KEY = "oxygenrouter:session";
type AuthContextValue = { user: User | null; token: string | null; loading: boolean; login: (username: string, password: string) => Promise<User>; register: (username: string, email: string, password: string) => Promise<User>; logout: () => Promise<void>; };
const AuthContext = React.createContext<AuthContextValue | null>(null);

export function AuthProvider({ children }: { children: React.ReactNode }) {
  const [token, setToken] = React.useState<string | null>(() => localStorage.getItem(STORAGE_KEY));
  const [user, setUser] = React.useState<User | null>(null);
  const [loading, setLoading] = React.useState(true);
  React.useEffect(() => {
    setSessionToken(token);
    if (!token) { setUser(null); setLoading(false); return; }
    setLoading(true);
    api.auth.me().then(setUser).catch(() => { localStorage.removeItem(STORAGE_KEY); setToken(null); }).finally(() => setLoading(false));
  }, [token]);
  const login = async (username: string, password: string) => { const result = await api.auth.login({ username, password }); localStorage.setItem(STORAGE_KEY, result.session_token); setSessionToken(result.session_token); setUser(result.user); setToken(result.session_token); return result.user; };
  const register = (username: string, email: string, password: string) => api.auth.register({ username, email, password });
  const logout = async () => { try { await api.auth.logout(); } finally { localStorage.removeItem(STORAGE_KEY); setSessionToken(null); setUser(null); setToken(null); } };
  return <AuthContext.Provider value={{ user, token, loading, login, register, logout }}>{children}</AuthContext.Provider>;
}
export function useAuth() { const value = React.useContext(AuthContext); if (!value) throw new Error("useAuth must be used within AuthProvider"); return value; }

/**
 * Whether a signed-in account may reach the admin surfaces.
 *
 * The server's `Access::Admin` means "admin or root", and the instance's own
 * bootstrap account is a root — so a page that checked for `role === "admin"`
 * refused the owner. Every admin gate goes through this one function rather
 * than repeating the comparison, which is how the two drifted apart.
 */
export function isAdmin(user: { role: string } | null | undefined): boolean {
  return user?.role === "admin" || user?.role === "root";
}
