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

export interface User { id: string; username: string; email: string; role: "root" | "admin" | "user"; status: string; balance_micros: number; created_at: string; updated_at: string; last_login_at: string | null; }
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
  section:
    | "site"
    | "auth"
    | "routing"
    | "billing"
    | "operations"
    | "security"
    | "models"
    /** Read once at startup, so a change needs a restart. */
    | "bootstrap";
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
  /** The calling address; only populated when the instance records IPs. */
  client_ip?: string | null;
}

/**
 * The instance's own display copy, served publicly so the sign-in page and the
 * sidebar can render a name before any credential exists.
 */
export interface SiteValues {
  site_name: string;
  notice: string;
  footer: string;
  server_address: string;
  /** Already parsed by the server; empty when the block is switched off. */
  announcements: Announcement[];
  faq: FaqEntry[];
  /** The instance's own addresses, already parsed and switch-gated. */
  api_info: ApiInfo[];
}

/** One address the operator advertises, in the reference's shape. */
export interface ApiInfo {
  id?: number;
  url: string;
  route: string;
  description: string;
  /** A colour name: blue | green | cyan | purple | orange | red. */
  color?: string;
}

/** One operator announcement, in the reference's shape. */
export interface Announcement {
  id?: number;
  content: string;
  publishDate?: string;
  /** `default` | `ongoing` | `success` | `warning` | `error`. */
  type?: string;
  extra?: string;
}

export interface FaqEntry {
  question: string;
  answer: string;
}

/** One maintenance-task run (`/api/system-task*`). */
export interface SystemTask {
  id: number;
  task_id: string;
  type: string;
  /** `pending` | `running` | `succeeded` | `failed`. */
  status: string;
  /** JSON text: the request. */
  payload: string;
  /** JSON text: live progress. */
  state: string;
  /** JSON text: the outcome, empty until it finishes. */
  result: string;
  error: string;
  created_at: string;
  updated_at: string;
}

export interface SystemTaskList {
  tasks: SystemTask[];
  total: number;
}

/** Relay performance, by model and hour (`GET /api/perf-metrics*`). */
export interface PerfBucketPoint {
  ts: number;
  avg_latency_ms: number;
  success_rate: number;
  avg_tps: number;
  request_count: number;
}

export interface PerfSummary {
  avg_latency_ms: number;
  success_rate: number;
  avg_tps: number;
  request_count: number;
}

export interface PerfGroupSeries extends PerfSummary {
  group: string;
  series: PerfBucketPoint[];
}

export interface PerfModelQuery {
  model_name: string;
  window_start: number;
  window_end: number;
  summary: PerfSummary;
  series: PerfBucketPoint[];
  groups: PerfGroupSeries[];
}

export interface PerfModelSummary extends PerfSummary {
  model_name: string;
  recent_success_series: Array<{ ts: number; success_rate: number }>;
}

export interface PerfSummaryAll {
  window_start: number;
  window_end: number;
  summary: PerfSummary;
  models: PerfModelSummary[];
}

/** One recorded administrative action (`GET /api/audit-logs`). */
export interface AuditLog {
  id: string;
  actor_id: string | null;
  /** Denormalised, so the row stays readable after the account is gone. */
  actor_name: string;
  actor_role: string;
  method: string;
  path: string;
  status_code: number | null;
  /** The request body with credential-bearing fields replaced. */
  detail: string | null;
  client_ip: string | null;
  created_at: string;
}

/** A task plugin as the console lists it (`GET /api/plugin/task`). */
export interface TaskPlugin {
  key: string;
  name: string;
  version: string | null;
  description: string;
  enabled: boolean;
  active_version: string | null;
  /** Every build on file, newest first. */
  versions: string[];
  /** Endpoint protocols the active build claims. */
  protocols: string[];
  updated_at: string;
}

/** One signed-in browser session (`GET /api/user/sessions`). */
export interface SessionView {
  id: string;
  created_at: string;
  expires_at: string;
  /** True for the session making the request. */
  current: boolean;
}

export interface TwoFactorStatus {
  enabled: boolean;
  backup_codes_remaining: number;
}

export interface TwoFactorSetup {
  secret: string;
  otpauth_uri: string;
  issuer: string;
  account: string;
}

export interface AccessTokenStatus {
  enabled: boolean;
  created_at: string | null;
}

/** One model in the public price catalogue (`GET /api/pricing`). */
export interface PublicPricing {
  model_name: string;
  description?: string;
  icon?: string;
  tags?: string;
  vendor?: string;
  /** 0 per token, 1 per call. */
  quota_type: number;
  model_ratio: number;
  model_price: number;
  completion_ratio: number;
  cache_ratio?: number;
  create_cache_ratio?: number;
  image_ratio?: number;
  input_price_per_million: number;
  output_price_per_million: number;
  enable_groups: string[];
  supported_endpoint_types: string[];
  billing_mode?: string;
  billing_expr?: string;
  pricing_version: string;
}

export interface PublicVendor {
  id: string;
  name: string;
  description?: string;
  icon?: string;
}

export interface PricingCatalogue {
  success: boolean;
  data: PublicPricing[];
  vendors: PublicVendor[];
  group_ratio: Record<string, number>;
  usable_group: Record<string, string>;
  supported_endpoint: Record<string, { method: string; path: string }>;
  auto_groups: string[];
  pricing_version: string;
}

/** One row of the public model leaderboard (`GET /api/rankings`). */
export interface RankedModel {
  rank: number;
  previous_rank?: number;
  model_name: string;
  vendor: string;
  vendor_icon?: string;
  category: string;
  total_tokens: number;
  share: number;
  growth_pct: number;
}

export interface RankedVendor {
  rank: number;
  vendor: string;
  vendor_icon?: string;
  total_tokens: number;
  share: number;
  growth_pct: number;
  models_count: number;
  top_model: string;
}

export interface RankingMover {
  model_name: string;
  vendor: string;
  vendor_icon?: string;
  rank_delta: number;
  current_rank: number;
  growth_pct: number;
}

export interface ModelHistorySeries {
  points: Array<{ ts: string; label: string; model: string; vendor: string; tokens: number }>;
  models: Array<{ name: string; vendor: string; total: number }>;
  buckets: number;
}

export interface VendorShareSeries {
  points: Array<{ ts: string; label: string; vendor: string; share: number; tokens: number }>;
  vendors: Array<{ name: string; total: number; share: number }>;
  buckets: number;
}

export interface RankingsResponse {
  models: RankedModel[];
  vendors: RankedVendor[];
  top_movers: RankingMover[];
  top_droppers: RankingMover[];
  models_history: ModelHistorySeries;
  vendor_share_history: VendorShareSeries;
}

/** One model's price, as `/api/option/model_pricing` reports it. */
export interface ModelPricingEntry {
  model_name: string;
  /** Hash of `configured`; a save must present it back. */
  version: string;
  /** Only what the operator stored for this model. */
  configured: Record<string, unknown>;
  /** What the engine will actually charge, defaults included. */
  effective: Record<string, unknown>;
}

export interface ModelPricingSnapshot {
  entries: ModelPricingEntry[];
  options: Record<string, string>;
  empty_version: string;
}

export interface ModelPricingDetails {
  mode: string;
  input_price_per_million?: number;
  output_price_per_million?: number;
  per_call_price?: number;
}

export interface ModelPricingPreview {
  effective: Record<string, unknown>;
  billing_details: ModelPricingDetails;
}

export interface ModelPricingConversion {
  expression?: string;
  unsupported_reason?: string;
  effective: Record<string, unknown>;
}

/** One row of the Uptime-Kuma panel, as `/api/uptime/status` reports it. */
export interface UptimeMonitor {
  name: string;
  /** 24-hour availability, 0..1, as Uptime Kuma reports it. */
  uptime: number;
  /** 0 down, 1 up, 2 pending, 3 maintenance. */
  status: number;
  /** The Uptime Kuma "public group" the monitor belongs to, when it has one. */
  group?: string;
}

export interface UptimeGroupResult {
  categoryName: string;
  monitors: UptimeMonitor[];
}

export interface SystemStatus {
  version: string;
  uptime_seconds: number;
  total_channels: number;
  enabled_channels: number;
  total_requests: number;
  active_requests: number;
  /**
   * Present only for an authenticated caller: /status is public, so the bind
   * address is withheld from the anonymous payload. The local API token is not
   * part of this shape at all -- it arrives from the root-only /system/info.
   */
  listen_host?: string;
  listen_port?: number;
  /** The configured display currency; a public display preference. */
  currency: string;
  /** Whether the console should render the Uptime-Kuma panel. */
  uptime_kuma_enabled: boolean;
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
    // The list is masked, so a copy of the real value is requested explicitly.
    reveal: (id: string) => apiFetch<{ id: string; key: string }>(`/keys/${id}/secret`),
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
    // The caller's own logs. A non-admin cannot reach the instance-wide route, so
    // the console uses this for them; the scope is in the path, not a query flag.
    mine: (params: { page?: number; pageSize?: number; search?: string; status?: string; model?: string } = {}) => {
      const qs = new URLSearchParams();
      if (params.page) qs.set("page", String(params.page));
      if (params.pageSize) qs.set("page_size", String(params.pageSize));
      if (params.search) qs.set("search", params.search);
      if (params.status) qs.set("status", params.status);
      if (params.model) qs.set("model", params.model);
      return apiFetch<Paginated<RequestLog>>(`/logs/self?${qs.toString()}`);
    },
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
  site: {
    get: () => apiFetch<SiteValues>('/site'),
  },
  legal: {
    // Public copy: the instance's own about, terms and privacy text.
    about: () => apiFetch<string>('/about'),
    terms: () => apiFetch<string>('/user-agreement'),
    privacy: () => apiFetch<string>('/privacy-policy'),
  },
  status: {
    get: () => apiFetch<SystemStatus>('/status'),
  },
  uptime: {
    get: () => apiFetch<UptimeGroupResult[]>('/uptime/status'),
  },
  rankings: {
    get: (period: "today" | "week" | "month" = "week") =>
      apiFetch<RankingsResponse>(`/rankings?period=${period}`),
  },
  pricing: {
    // The catalogue's own envelope carries siblings next to `data`, so this one
    // reads the raw document instead of going through `apiFetch`.
    get: async (): Promise<PricingCatalogue> => {
      const headers = new Headers();
      if (sessionToken) headers.set("Authorization", `Bearer ${sessionToken}`);
      const res = await fetch(`${API_BASE}/pricing`, { headers });
      if (!res.ok) throw new Error(`Request failed (${res.status})`);
      return (await res.json()) as PricingCatalogue;
    },
  },
  plugins: {
    list: () => apiFetch<TaskPlugin[]>("/plugin/task"),
    upload: (input: { key: string; version: string; source: string }) =>
      apiFetch<TaskPlugin>("/plugin/task", { method: "POST", body: JSON.stringify(input) }),
    // These three answer a short status string or a count, not the plugin: the
    // console re-reads the list rather than trusting a returned snapshot.
    activate: (key: string, version: string) =>
      apiFetch<string>(`/plugin/task/${encodeURIComponent(key)}/activate`, {
        method: "POST",
        body: JSON.stringify({ version }),
      }),
    setEnabled: (key: string, enabled: boolean) =>
      apiFetch<string>(`/plugin/task/${encodeURIComponent(key)}/status`, {
        method: "POST",
        body: JSON.stringify({ enabled }),
      }),
    dryRun: (key: string, hook: string, input: unknown) =>
      apiFetch<unknown>(`/plugin/task/${encodeURIComponent(key)}/dryrun`, {
        method: "POST",
        body: JSON.stringify({ hook, input }),
      }),
    removeVersion: (key: string, version: string) =>
      apiFetch<number>(
        `/plugin/task/${encodeURIComponent(key)}/versions/${encodeURIComponent(version)}`,
        { method: "DELETE" },
      ),
  },
  audit: {
    // The trail's own reader takes a limit only; filtering and paging happen in
    // the page, which is honest about what the endpoint does.
    list: (limit = 200) => apiFetch<AuditLog[]>(`/audit-logs?limit=${limit}`),
    clear: () => apiFetch<number>("/audit-logs", { method: "DELETE" }),
  },
  systemTasks: {
    // Root-only, like the tree itself: these endpoints rewrite the instance's
    // own history.
    current: (type = "log_cleanup") => apiFetch<SystemTask | null>(`/system-task/current?type=${type}`),
    list: (type = "log_cleanup", limit = 50) =>
      apiFetch<SystemTaskList>(`/system-task/list?type=${type}&limit=${limit}`),
    startLogCleanup: (targetTimestamp: number) =>
      apiFetch<SystemTask>(`/system-task/log-cleanup?target_timestamp=${targetTimestamp}`, { method: "POST" }),
    get: (taskId: string) => apiFetch<SystemTask>(`/system-task/${encodeURIComponent(taskId)}`),
    clearHistory: (type = "log_cleanup") => apiFetch<number>(`/system-task/history?type=${type}`, { method: "DELETE" }),
  },  perfMetrics: {
    // Public, like the reference's pricing module: these figures sit beside the
    // price list.
    summary: (hours = 24) => apiFetch<PerfSummaryAll>(`/perf-metrics/summary?hours=${hours}`),
    model: (model: string, hours = 24, group?: string) => {
      const query = new URLSearchParams({ model, hours: String(hours) });
      if (group) query.set("group", group);
      return apiFetch<PerfModelQuery>(`/perf-metrics?${query}`);
    },
  },
  account: {
    sessions: {
      list: () => apiFetch<SessionView[]>("/user/sessions"),
      revoke: (id: string) => apiFetch<string>(`/user/sessions/${id}`, { method: "DELETE" }),
      revokeOthers: () => apiFetch<number>("/user/sessions/revoke-others", { method: "POST" }),
    },
    twoFactor: {
      status: () => apiFetch<TwoFactorStatus>("/user/2fa/status"),
      setup: () => apiFetch<TwoFactorSetup>("/user/2fa/setup", { method: "POST" }),
      enable: (code: string) =>
        apiFetch<{ enabled: boolean; backup_codes: string[] }>("/user/2fa/enable", {
          method: "POST",
          body: JSON.stringify({ code }),
        }),
      disable: (code: string) =>
        apiFetch<string>("/user/2fa/disable", { method: "POST", body: JSON.stringify({ code }) }),
      backupCodes: (code: string) =>
        apiFetch<{ backup_codes: string[] }>("/user/2fa/backup_codes", {
          method: "POST",
          body: JSON.stringify({ code }),
        }),
    },
    accessToken: {
      status: () => apiFetch<AccessTokenStatus>("/user/token/status"),
      // The value is returned exactly once, here.
      generate: () => apiFetch<string>("/user/token", { method: "POST" }),
      revoke: () => apiFetch<string>("/user/token", { method: "DELETE" }),
    },
  },
  modelPricing: {
    snapshot: (models: string[] = []) => {
      const query = models.map((model) => `model=${encodeURIComponent(model)}`).join('&');
      return apiFetch<ModelPricingSnapshot>(`/option/model_pricing${query ? `?${query}` : ''}`);
    },
    preview: (model_name: string, pricing: Record<string, unknown>) =>
      apiFetch<ModelPricingPreview>('/option/model_pricing/preview', {
        method: 'POST',
        body: JSON.stringify({ model_name, pricing }),
      }),
    convert: (model_name: string, pricing: Record<string, unknown>) =>
      apiFetch<ModelPricingConversion>('/option/model_pricing/convert', {
        method: 'POST',
        body: JSON.stringify({ model_name, pricing }),
      }),
    update: (changes: Array<{ model_name: string; expected_version?: string; pricing?: Record<string, unknown>; reset?: boolean }>) =>
      apiFetch<string[]>('/option/model_pricing', { method: 'PATCH', body: JSON.stringify({ changes }) }),
    reset: (models: string[] = []) =>
      apiFetch<string[]>('/option/reset_model_pricing', {
        method: 'POST',
        body: JSON.stringify({ models }),
      }),
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
