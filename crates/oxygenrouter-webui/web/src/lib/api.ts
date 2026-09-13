export const API_BASE = '/api';

export interface ApiResponse<T> {
  success: boolean;
  data: T | null;
  error: string | null;
}

export interface ChannelKeyStatus {
  index: number;
  preview: string;
  status: number;
  disabled_reason: string | null;
  disabled_time: number | null;
}

export interface ModelMetadata {
  id: string;
  model_name: string;
  description: string;
  icon: string;
  tags: string;
  vendor: string;
  endpoints: string[];
  name_rule: number;
  status: number;
  sync_official: number;
  created_at: string;
  updated_at: string;
}

export interface Paginated<T> {
  items: T[];
  total: number;
  page: number;
  page_size: number;
}

export interface LogStats {
  total: number;
  success: number;
  errors: number;
  success_rate: number;
  avg_latency_ms: number;
  total_tokens: number;
  model_breakdown: DashboardBreakdown[];
  channel_breakdown: DashboardBreakdown[];
}

let sessionToken: string | null = null;
export function setSessionToken(token: string | null) { sessionToken = token; }

export interface User { id: string; username: string; email: string; role: "admin" | "user"; status: string; balance_micros: number; created_at: string; updated_at: string; last_login_at: string | null; }
export interface LedgerEntry { id: string; user_id: string; amount_micros: number; balance_after_micros: number; kind: string; description: string; reference_id: string | null; created_at: string; }
export interface SubscriptionPlan { id: string; name: string; description: string; price_micros: number; quota_micros: number; duration_days: number; enabled: boolean; created_at: string; updated_at: string; }
export interface Subscription { id: string; user_id: string; plan_id: string; status: string; started_at: string; expires_at: string; created_at: string; }
export interface RedemptionCode { id: string; code: string; amount_micros: number; plan_id: string | null; enabled: boolean; max_uses: number; used_count: number; expires_at: string | null; created_at: string; }
export interface PaymentOrder { id: string; user_id: string; amount_micros: number; provider: string; status: string; external_reference: string | null; created_at: string; updated_at: string; }
export interface Wallet { balance_micros: number; ledger: LedgerEntry[]; }
export interface SubscribeResult { subscription: Subscription; balance_micros: number; }
export interface LoginResult { user: User; session_token: string; expires_at: string; }

export interface ChannelInfo {
  is_multi_key: boolean;
  multi_key_size: number;
  multi_key_status_list: Record<string, number>;
  multi_key_disabled_reason: Record<string, string>;
  multi_key_disabled_time: Record<string, number>;
  multi_key_polling_index: number;
  multi_key_mode: string;
}

export interface Channel {
  id: string;
  name: string;
  provider: string;
  base_url: string;
  api_key: string;
  priority: number;
  weight: number;
  enabled: boolean;
  test_model: string;
  group_name: string;
  tags: string[];
  model_list: string[];
  response_headers: Record<string, string>;
  status_code_mapping: Record<string, unknown>;
  override_parameters: Record<string, unknown>;
  balance_micros: number;
  last_test_at: string | null;
  info: ChannelInfo;
  config: Record<string, string>;
  models: string[];
  model_mapping: string;
  system_prompt: string;
  created_at: string;
  updated_at: string;
}

export interface ProviderField {
  key: string;
  label: string;
  placeholder?: string;
  type?: "text" | "password" | "textarea";
}

export interface ProviderPreset {
  id: string;
  name: string;
  baseUrl: string;
  testModel: string;
  fields?: ProviderField[];
  defaultModels?: string[];
  supportsModelMapping?: boolean;
  supportsSystemPrompt?: boolean;
}

export const PROVIDER_PRESETS: ProviderPreset[] = [
  {
    id: "openai", name: "OpenAI", baseUrl: "https://api.openai.com", testModel: "gpt-3.5-turbo",
    supportsModelMapping: true,
    supportsSystemPrompt: true,
    defaultModels: ["gpt-4o", "gpt-4o-mini", "gpt-4-turbo", "gpt-4", "gpt-3.5-turbo", "o1-preview", "o1-mini"],
  },
  {
    id: "anthropic", name: "Anthropic", baseUrl: "https://api.anthropic.com", testModel: "claude-3-haiku-20240307",
    supportsModelMapping: true,
    supportsSystemPrompt: true,
    defaultModels: ["claude-sonnet-4-20250514", "claude-3-5-sonnet-20241022", "claude-3-5-haiku-20241022", "claude-3-haiku-20240307"],
  },
  {
    id: "azure", name: "Azure OpenAI", baseUrl: "https://{resource}.openai.azure.com", testModel: "gpt-35-turbo",
    supportsModelMapping: true,
    supportsSystemPrompt: true,
    fields: [
      { key: "azure_endpoint", label: "Azure Endpoint", placeholder: "https://docs-test-001.openai.azure.com" },
      { key: "azure_api_version", label: "API Version", placeholder: "2024-03-01-preview" },
      { key: "azure_deployment", label: "Deployment Name", placeholder: "gpt-35-turbo" },
    ],
    defaultModels: ["gpt-4o", "gpt-4o-mini", "gpt-4-turbo", "gpt-4", "gpt-35-turbo"],
  },
  {
    id: "deepseek", name: "DeepSeek", baseUrl: "https://api.deepseek.com", testModel: "deepseek-chat",
    supportsModelMapping: true,
    supportsSystemPrompt: true,
    defaultModels: ["deepseek-chat", "deepseek-reasoner"],
  },
  {
    id: "moonshot", name: "Moonshot", baseUrl: "https://api.moonshot.cn", testModel: "moonshot-v1-8k",
    supportsModelMapping: true,
    supportsSystemPrompt: true,
    defaultModels: ["moonshot-v1-8k", "moonshot-v1-32k", "moonshot-v1-128k"],
  },
  {
    id: "zhipu", name: "Zhipu GLM", baseUrl: "https://open.bigmodel.cn/api/paas", testModel: "glm-4-flash",
    supportsModelMapping: true,
    supportsSystemPrompt: true,
    defaultModels: ["glm-4-plus", "glm-4", "glm-4-flash", "glm-4-long"],
  },
  {
    id: "aws", name: "AWS Bedrock", baseUrl: "https://bedrock-runtime.{region}.amazonaws.com", testModel: "anthropic.claude-3-haiku-20240307-v1:0",
    fields: [
      { key: "aws_region", label: "Region", placeholder: "us-east-1" },
      { key: "aws_access_key", label: "Access Key (AK)", placeholder: "AKIA..." },
      { key: "aws_secret_key", label: "Secret Key (SK)", placeholder: "...", type: "password" },
    ],
    supportsModelMapping: true,
    supportsSystemPrompt: true,
    defaultModels: ["anthropic.claude-3-haiku-20240307-v1:0", "anthropic.claude-3-sonnet-20240229-v1:0", "anthropic.claude-3-opus-20240229-v1:0"],
  },
  {
    id: "vertex", name: "Vertex AI", baseUrl: "https://{region}-aiplatform.googleapis.com", testModel: "gemini-pro",
    fields: [
      { key: "vertex_region", label: "Region", placeholder: "us-central1" },
      { key: "vertex_project_id", label: "Project ID", placeholder: "my-project-id" },
      { key: "vertex_credentials", label: "Service Account Key (JSON)", placeholder: "{ ... }", type: "textarea" },
    ],
    supportsModelMapping: true,
    supportsSystemPrompt: true,
    defaultModels: ["gemini-1.5-pro", "gemini-1.5-flash", "gemini-pro"],
  },
  {
    id: "ollama", name: "Ollama (Local)", baseUrl: "http://127.0.0.1:11434", testModel: "llama3",
    supportsModelMapping: true,
    supportsSystemPrompt: true,
    defaultModels: ["llama3", "mistral", "codellama", "phi3"],
  },
  {
    id: "custom", name: "Custom", baseUrl: "https://", testModel: "gpt-3.5-turbo",
    supportsModelMapping: true,
    supportsSystemPrompt: true,
  },
];

export interface ApiKey {
  id: string;
  key: string;
  name: string;
  priority: number;
  enabled: boolean;
  created_at: string;
  expires_at: string | null;
  quota_micros: number;
  used_micros: number;
  allowed_models: string[];
  ip_allowlist: string[];
  group_name: string;
  cross_group_retry: boolean;
}

export interface OptionEntry {
  key: string;
  section: "site" | "auth" | "routing" | "billing" | "operations" | "security" | "models";
  kind: "string" | "int" | "bool" | "float";
  value: string;
  default: string;
  description: string;
  secret: boolean;
}

export interface ApiKeyUsage {  api_key_id: string;
  api_key_name: string;
  enabled: boolean;
  total_requests: number;
  successful_requests: number;
  total_tokens: number;
  average_latency_ms: number;
}

export interface ModelMap {
  id: string;
  channel_id: string;
  pattern: string;
  target_model: string;
  enabled: boolean;
  created_at: string;
}

export interface RouteRule {
  id: string;
  name: string;
  rule_type: string;
  priority: number;
  enabled: boolean;
  config: Record<string, unknown>;
  created_at: string;
}

export interface RequestLog {
  id: string;
  method: string;
  path: string;
  model: string | null;
  channel_id: string | null;
  api_key_id: string | null;
  status_code: number | null;
  error: string | null;
  tokens_used: number | null;
  duration_ms: number;
  created_at: string;
}

export interface SystemStatus {
  version: string;
  uptime_seconds: number;
  total_channels: number;
  enabled_channels: number;
  total_requests: number;
  active_requests: number;
  local_api_token: string;
  listen_host: string;
  listen_port: number;
}

export interface DashboardBreakdown {
  name: string;
  requests: number;
  errors: number;
  average_latency_ms: number;
  tokens: number;
}

export interface TimeBucket {
  label: string;
  requests: number;
  errors: number;
  latency_ms: number;
}

export interface ModelTimeBucket extends TimeBucket {
  model: string;
  tokens: number;
}

export interface AnalyticsFlowNode {
  id: string;
  label: string;
  kind: "api_key" | "model" | "channel";
}

export interface AnalyticsFlowLink {
  source: string;
  target: string;
  request_count: number;
  tokens: number;
}

export interface AnalyticsFlow {
  range_start: string;
  range_end: string;
  nodes: AnalyticsFlowNode[];
  links: AnalyticsFlowLink[];
}

export interface ChannelPerf {
  channel_id: string;
  channel_name: string;
  requests: number;
  errors: number;
  latency_ms: number;
  last_error: string | null;
  last_test_at: number | null;
  enabled: boolean;
}

export interface DashboardSnapshot {
  range_start: string;
  range_end: string;
  bucket_seconds: number;
  total_requests: number;
  successful_requests: number;
  failed_requests: number;
  success_rate: number;
  average_latency_ms: number;
  today_requests: number;
  today_errors: number;
  total_tokens: number;
  active_channels: number;
  model_breakdown: DashboardBreakdown[];
  channel_breakdown: DashboardBreakdown[];
  api_key_breakdown: DashboardBreakdown[];
  recent_requests: RequestLog[];
  time_series: TimeBucket[];
  model_time_series: ModelTimeBucket[];
  api_key_time_series: ModelTimeBucket[];
  channel_perf: ChannelPerf[];
  time_range: string;
}

export interface AppSettings {
  listen_host: string;
  listen_port: number;
  local_api_token: string;
  open_browser_on_start: boolean;
  db_path: string;
  log_level: string;
  max_retries: number;
  retry_delay_ms: number;
  retry_backoff: string;
  upstream_timeout_ms: number;
  user_agent: string;
  max_concurrent_requests: number;
  request_log_retention_days: number;
  theme: string;
  language: string;
}

export interface ChannelTestResult {
  channel_id: string;
  success: boolean;
  latency_ms: number | null;
  error: string | null;
  model: string | null;
}

export async function apiFetch<T>(path: string, opts?: RequestInit): Promise<T> {
  const headers = new Headers(opts?.headers);
  if (!headers.has("Content-Type") && opts?.body) headers.set("Content-Type", "application/json");
  if (sessionToken && !headers.has("Authorization")) headers.set("Authorization", `Bearer ${sessionToken}`);
  const res = await fetch(`${API_BASE}${path}`, {
    ...opts,
    headers,
  });
  const json: ApiResponse<T> = await res.json().catch(() => ({ success: false, data: null, error: `Request failed (${res.status})` }));
  if (!json.success) throw new Error(json.error ?? 'Unknown error');
  return json.data as T;
}

export const api = {
  auth: {
    register: (input: { username: string; email: string; password: string }) => apiFetch<User>("/auth/register", { method: "POST", body: JSON.stringify(input) }),
    login: (input: { username: string; password: string }) => apiFetch<LoginResult>("/auth/login", { method: "POST", body: JSON.stringify(input) }),
    logout: () => apiFetch<string>("/auth/logout", { method: "POST" }),
    me: () => apiFetch<User>("/auth/me"),
  },
  wallet: { get: () => apiFetch<Wallet>("/wallet") },
  plans: { list: () => apiFetch<SubscriptionPlan[]>("/plans") },
  subscriptions: { mine: () => apiFetch<Subscription[]>("/subscriptions/me"), subscribe: (plan_id: string) => apiFetch<SubscribeResult>("/subscriptions/subscribe", { method: "POST", body: JSON.stringify({ plan_id }) }) },
  redemption: { redeem: (code: string) => apiFetch<string>("/redemption/redeem", { method: "POST", body: JSON.stringify({ code }) }) },
  orders: { manual: (amount_micros: number) => apiFetch<PaymentOrder>("/orders/manual", { method: "POST", body: JSON.stringify({ amount_micros }) }) },
  admin: {
    users: { list: (search?: string) => apiFetch<User[]>(`/admin/users${search ? `?search=${encodeURIComponent(search)}` : ""}`), create: (input: { username: string; email: string; password: string; role: "admin" | "user"; status: string }) => apiFetch<User>("/admin/users", { method: "POST", body: JSON.stringify(input) }), adjustBalance: (id: string, input: { amount_micros: number; description: string }) => apiFetch<LedgerEntry>(`/admin/users/${id}/balance-adjust`, { method: "POST", body: JSON.stringify(input) }) },
    plans: { list: () => apiFetch<SubscriptionPlan[]>("/admin/plans"), create: (input: Omit<SubscriptionPlan, "id" | "created_at" | "updated_at">) => apiFetch<SubscriptionPlan>("/admin/plans", { method: "POST", body: JSON.stringify(input) }) },
    codes: { list: () => apiFetch<RedemptionCode[]>("/admin/redemption-codes"), create: (input: Omit<RedemptionCode, "id" | "used_count" | "created_at">) => apiFetch<RedemptionCode>("/admin/redemption-codes", { method: "POST", body: JSON.stringify(input) }) },
    orders: { list: () => apiFetch<PaymentOrder[]>("/admin/orders"), complete: (id: string) => apiFetch<PaymentOrder>(`/admin/orders/${id}/complete`, { method: "POST" }) },
  },
  channels: {
    list: () => apiFetch<Channel[]>('/channels'),
    create: (ch: Partial<Channel>) => apiFetch<Channel>('/channels', { method: 'POST', body: JSON.stringify(ch) }),
    get: (id: string) => apiFetch<Channel>(`/channels/${id}`),
    update: (id: string, ch: Partial<Channel>) => apiFetch<Channel>(`/channels/${id}`, { method: 'PUT', body: JSON.stringify(ch) }),
    delete: (id: string) => apiFetch<string>(`/channels/${id}`, { method: 'DELETE' }),
    test: (id: string) => apiFetch<ChannelTestResult>(`/channels/${id}/test`, { method: 'POST' }),
    fetchModels: (id: string) => apiFetch<string[]>(`/channels/${id}/models/fetch`, { method: 'POST' }),
    listKeys: (id: string) => apiFetch<ChannelKeyStatus[]>(`/channels/${id}/keys`),
    manageKeys: (id: string, input: { action: string; index?: number; reason?: string; keys?: string }) =>
      apiFetch<Channel>(`/channels/${id}/keys`, { method: 'POST', body: JSON.stringify(input) }),
    batchUpdate: (ids: string[], enabled: boolean) => apiFetch<{ updated: number }>('/channels/batch', { method: 'PATCH', body: JSON.stringify({ ids, enabled }) }),
    batchDelete: (ids: string[]) => apiFetch<{ deleted: number }>('/channels/batch', { method: 'DELETE', body: JSON.stringify({ ids }) }),
  },
  modelMetadata: {
    list: (params: { page?: number; pageSize?: number; search?: string } = {}) => {
      const qs = new URLSearchParams();
      if (params.page) qs.set('page', String(params.page));
      if (params.pageSize) qs.set('page_size', String(params.pageSize));
      if (params.search) qs.set('search', params.search);
      return apiFetch<Paginated<ModelMetadata>>(`/models-metadata?${qs.toString()}`);
    },
    create: (m: Partial<ModelMetadata>) => apiFetch<ModelMetadata>('/models-metadata', { method: 'POST', body: JSON.stringify(m) }),
    update: (id: string, m: Partial<ModelMetadata>) => apiFetch<ModelMetadata>(`/models-metadata/${id}`, { method: 'PUT', body: JSON.stringify(m) }),
    delete: (id: string) => apiFetch<string>(`/models-metadata/${id}`, { method: 'DELETE' }),
    missing: () => apiFetch<string[]>('/models-metadata/missing'),
    sync: () => apiFetch<number>('/models-metadata/sync', { method: 'POST' }),
  },
  keys: {
    list: () => apiFetch<ApiKey[]>('/keys'),
    create: (k: Partial<ApiKey>) => apiFetch<ApiKey>('/keys', { method: 'POST', body: JSON.stringify(k) }),
    delete: (id: string) => apiFetch<string>(`/keys/${id}`, { method: 'DELETE' }),
    usage: () => apiFetch<ApiKeyUsage[]>('/keys/usage'),
    query: (params: { page?: number; pageSize?: number; search?: string } = {}) => {
      const qs = new URLSearchParams();
      if (params.page) qs.set('page', String(params.page));
      if (params.pageSize) qs.set('page_size', String(params.pageSize));
      if (params.search) qs.set('search', params.search);
      return apiFetch<Paginated<ApiKey>>(`/keys/query?${qs.toString()}`);
    },
  },
  modelMaps: {
    list: () => apiFetch<ModelMap[]>('/model-maps'),
    create: (m: Partial<ModelMap>) => apiFetch<ModelMap>('/model-maps', { method: 'POST', body: JSON.stringify(m) }),
    delete: (id: string) => apiFetch<string>(`/model-maps/${id}`, { method: 'DELETE' }),
  },
  rules: {
    list: () => apiFetch<RouteRule[]>('/rules'),
    create: (r: Partial<RouteRule>) => apiFetch<RouteRule>('/rules', { method: 'POST', body: JSON.stringify(r) }),
    delete: (id: string) => apiFetch<string>(`/rules/${id}`, { method: 'DELETE' }),
  },
  logs: {
    list: (limit = 100) => apiFetch<RequestLog[]>(`/logs?limit=${limit}`),
    query: (params: {
      page?: number;
      pageSize?: number;
      start?: string;
      end?: string;
      model?: string;
      channelId?: string;
      apiKeyId?: string;
      status?: string;
      search?: string;
    }): Promise<Paginated<RequestLog>> => {
      const qs = new URLSearchParams({ paginated: "true" });
      if (params.page) qs.set("page", String(params.page));
      if (params.pageSize) qs.set("page_size", String(params.pageSize));
      if (params.start) qs.set("start", params.start);
      if (params.end) qs.set("end", params.end);
      if (params.model && params.model !== "all") qs.set("model", params.model);
      if (params.channelId && params.channelId !== "all") qs.set("channel_id", params.channelId);
      if (params.apiKeyId && params.apiKeyId !== "all") qs.set("api_key_id", params.apiKeyId);
      if (params.status && params.status !== "all") qs.set("status", params.status);
      if (params.search) qs.set("search", params.search);
      return apiFetch<Paginated<RequestLog>>(`/logs?${qs.toString()}`);
    },
    stats: (timeRange = "24h") => apiFetch<LogStats>(`/log/stats?time_range=${timeRange}`),
    export: async (format: "csv" | "json" = "csv"): Promise<void> => {
      const res = await fetch(`${API_BASE}/logs?limit=10000`);
      const json: ApiResponse<RequestLog[]> = await res.json();
      if (!json.success || !json.data) throw new Error("Export failed");
      const rows = json.data;
      if (format === "json") {
        const blob = new Blob([JSON.stringify(rows, null, 2)], { type: "application/json" });
        downloadBlob(blob, `oxygenrouter-logs-${Date.now()}.json`);
      } else {
        const headers = ["created_at", "method", "path", "model", "channel_id", "status_code", "duration_ms", "tokens_used", "error"];
        const csv = [
          headers.join(","),
          ...rows.map((r) =>
            headers.map((h) => {
              const v = (r as unknown as Record<string, unknown>)[h];
              if (v === null || v === undefined) return "";
              const s = String(v);
              return s.includes(",") || s.includes('"') || s.includes("\n") ? `"${s.replace(/"/g, '""')}"` : s;
            }).join(","),
          ),
        ].join("\n");
        const blob = new Blob([csv], { type: "text/csv" });
        downloadBlob(blob, `oxygenrouter-logs-${Date.now()}.csv`);
      }
    },
  },
  status: {
    get: () => apiFetch<SystemStatus>('/status'),
  },
  dashboard: {
    get: (timeRange: string = "24h", filters: Record<string, string | undefined> = {}) => {
      const query = new URLSearchParams({ time_range: timeRange });
      Object.entries(filters).forEach(([key, value]) => { if (value) query.set(key, value); });
      return apiFetch<DashboardSnapshot>(`/dashboard?${query}`);
    },
  },
  analytics: {
    flow: (timeRange: string = "24h", filters: Record<string, string | undefined> = {}) => {
      const query = new URLSearchParams({ time_range: timeRange });
      Object.entries(filters).forEach(([key, value]) => { if (value) query.set(key, value); });
      return apiFetch<AnalyticsFlow>(`/analytics/flow?${query}`);
    },
  },
  settings: {
    get: () => apiFetch<AppSettings>('/settings'),
    update: (s: Partial<AppSettings>) => apiFetch<AppSettings>('/settings', { method: 'PUT', body: JSON.stringify(s) }),
  },
  options: {
    list: () => apiFetch<OptionEntry[]>('/options'),
    update: (key: string, value: string) => apiFetch<string>('/options', { method: 'PUT', body: JSON.stringify({ key, value }) }),
  },
  system: {
    info: () => apiFetch<SystemInfo>('/system/info'),
  },
  backup: {
    create: async (): Promise<void> => {
      const res = await fetch(`${API_BASE}/backup/create`, { method: 'POST' });
      const blob = await res.blob();
      const filename = res.headers.get('content-disposition')?.match(/filename="?([^"]+)"?/)?.[1] ?? `oxygenrouter-backup-${Date.now()}.db`;
      downloadBlob(blob, filename);
    },
  },
};

function downloadBlob(blob: Blob, filename: string) {
  const url = URL.createObjectURL(blob);
  const a = document.createElement("a");
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}

export interface SystemInfo {
  version: string;
  uptime_seconds: number;
  db_size_bytes: number;
  log_count: number;
  channel_count: number;
  enabled_channel_count: number;
  key_count: number;
  model_map_count: number;
  rule_count: number;
  local_api_token: string;
  listen_host: string;
  listen_port: number;
  max_concurrent_requests: number;
  upstream_timeout_ms: number;
  max_retries: number;
  platform: string;
  arch: string;
  rustc_version: string;
  build_profile: string;
  started_at: string;
}
