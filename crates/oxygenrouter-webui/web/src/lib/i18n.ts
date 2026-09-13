export type Locale = "en" | "zh";

export const STRINGS = {
  en: {
    app: {
      brand: "OxygenRouter",
      version: "v26 路 Alpha",
      localMode: "Local Mode",
      openNav: "Open navigation",
      closeNav: "Close navigation",
      collapse: "Collapse navigation",
      expand: "Expand navigation",
      workspace: "Workspace",
    },
    common: {
      save: "Save",
      saving: "Saving...",
      saved: "Saved!",
      cancel: "Cancel",
      create: "Create",
      edit: "Edit",
      delete: "Delete",
      refresh: "Refresh",
      search: "Search",
      copy: "Copy",
      copied: "Copied",
      minutes: "minutes",
      hours: "hours",
      days: "days",
      enabled: "Enabled",
      disabled: "Disabled",
      yes: "Yes",
      no: "No",
      confirm: "Confirm",
      language: "Language",
      backToList: "Back to list",
      notFound: "Endpoint not found",
    },
    nav: {
      overview: "Overview",
      dashboard: "Dashboard",
      analytics: "Model Analytics",
      channels: "Channels",
      keys: "API Keys",
      routes: "Routes",
      logs: "Request Logs",
      settings: "Settings",
      system: "System Info",
      systemSettings: "System Settings",
      playground: "Playground",
      models: "Models",
      modelsMeta: "Model Registry",
      wallet: "Wallet",
      plans: "Plans",
      subscriptions: "Subscriptions",
      users: "Users",
      billing: "Billing",
    },
    dashboard: {
      title: "Dashboard",
      description: "Local routing health and usage",
      eyebrow: "OXYGENROUTER / LOCAL INSTANCE",
      heading: "Keep your model traffic in view.",
      intro: "One local OpenAI-compatible address for every configured channel, with routing and request history kept on this machine.",
      endpoint: "OpenAI base address",
      copyAddress: "Copy address",
      activeChannels: (n: number) => (n === 0 ? "Add your first channel" : `${n} active channels`),
      bannerTitle: "Get started in minutes with your API gateway",
      bannerReady: "All systems operational",
      bannerSub: "Setup progress:",
      createApiKey: "Create API Key",
      addChannel: "Add Channel",
      viewLogs: "View Logs",
      usageOverview: "Usage Overview",
      monitorBalance: "Monitor balance, usage and request count",
      todayConsumption: "24h Consumption",
      last24h: "Last 24 hours",
      totalUsage: "Total Usage",
      totalConsumption: "Total consumption (USD)",
      requestCount: "Request Count",
      totalRequests: "Total requests",
      remainingBalance: "Remaining Balance",
      statusNormal: "Normal",
      availableTime: "Available time",
      noUsage: "No usage record",
      wallet: "Wallet",
      performanceHealth: "Performance Health",
      last24hPerf: "Last 24 hours performance metrics",
      successRate: "Success Rate",
      avgLatency: "Average Latency",
      throughput: "Throughput",
      requestsToday: "requests today",
      quickActions: "Quick Actions",
      createApiKeyDesc: "Create keys for your apps and services",
      viewLogsDesc: "Review requests, errors and billing details",
      viewAnalyticsDesc: "View detailed usage analytics",
      configureDesc: "Configure your router settings",
      apiInfo: "API Information",
      routingEnabled: "Routing enabled",
      currentDomain: "Current domain",
      authConfigured: "Authentication",
      requiresApiKey: "Requires API key",
      noKeys: "No keys configured",
      modelSelected: "Model selected",
      mappingsActive: "mappings active",
      noMappings: "No mappings configured",
      announcements: "Announcements",
      noAnnouncements: "No announcements yet",
      metrics: {
        total: "Total Requests",
        success: "Success Rate",
        latency: "Average Latency",
        channels: "Active Channels",
        today: "today",
        across: "Across local logs",
        failed: "failed requests",
        healthy: "All configured channels healthy",
      },
      byModel: "Requests by Model",
      byChannel: "Requests by Channel",
      localData: "Local data",
      recent: "Recent Activity",
      recentSub: "Latest requests captured locally",
      live: "Live",
      updated: "Updated",
      localNote: "Data stays on this machine",
      getStarted: "Get Started",
      getStartedSub: "Three local setup steps",
      step1: { title: "Add an upstream channel", desc: "Set the Base URL, API key, priority, and weight." },
      step2: { title: "Create a local API key", desc: "Use it as the Bearer token in your client." },
      step3: { title: "Inspect request activity", desc: "Review latency, status, model, and selected channel." },
      services: "Local Services",
      servicesSub: "Runtime snapshot",
      servicesProxy: "Proxy listener",
      servicesStorage: "Storage",
      servicesLog: "Request log",
      servicesTokens: "Token usage",
      channelsRoute: "Go to channels",
      keysRoute: "Go to keys",
      logsRoute: "Go to logs",
      noRequests: "Make a request to populate the dashboard.",
      noData: "No local data yet.",
      noModels: "Make a request to see model usage.",
      noChannels: "Configure a channel to begin routing.",
      errors: "errors",
      noErrors: "No errors",
      ms: "ms",
    },
    channels: {
      title: "Channels",
      description: "Manage upstream API providers and their routing priority",
      add: "Add Channel",
      empty: "No channels yet. Add your first upstream provider.",
      new: "New Channel",
      edit: "Edit Channel",
      name: "Name",
      provider: "Provider",
      baseUrl: "Base URL",
      apiKey: "API Key",
      priority: "Priority",
      weight: "Weight",
      testModel: "Test Model",
      syncModels: "Sync upstream models",
      modelsSynced: "Models synchronized",
      modelsSyncFailed: "Model sync failed",
      modelsAvailable: "models available",
      multiKeys: "Multi-key management",
      multiKeysDesc: "Manage individual API keys for this channel, including health and rotation mode",
      keysEnabled: "keys enabled",
      keyMode: "Rotation mode",
      keyModeRandom: "Random",
      keyModePolling: "Polling",
      enableKey: "Enable",
      disableKey: "Disable",
      enableAllKeys: "Enable all",
      disableAllKeys: "Disable all",
      autoDisabled: "Auto-disabled",
      keyActionFailed: "Key operation failed",
      appendKeys: "Append keys",
      appendKeysBtn: "Append",
      deleteDisabledKeys: "Clean auto-disabled",
      test: "Test channel",
      testAll: "Test all",
      testing: "Testing…",
      delete: "Delete channel",
      deleteConfirm: (name: string) => `Delete channel "${name}"?`,
      namePlaceholder: "OpenAI Official",
      baseUrlPlaceholder: "https://api.openai.com",
      ok: "OK",
      err: "Error",
      search: "Search channels",
      showDisabled: "Show disabled",
      allProviders: "All providers",
      allGroups: "All groups",
      group: "Group",
      enabledLabel: "enabled",
      copyUrl: "Copy base URL",
      clickEnable: "Enable channel",
      clickDisable: "Disable channel",
      selected: "X selected",
      enableSelected: "Enable Selected",
      disableSelected: "Disable Selected",
      deleteSelected: "Delete Selected",
      deleteSelectedConfirm: "Delete X selected channels? This cannot be undone.",
      models: "Models",
      modelSearchPlaceholder: "Search models...",
      fillModels: "Fill",
      clearModels: "Clear",
      customModelPlaceholder: "Custom model name",
      batchMode: "Batch key mode (one per line)",
      batchKeyPlaceholder: "Enter API keys, one per line...",
      advancedSettings: "Advanced Settings",
      modelMapping: "Model Mapping (JSON)",
      systemPrompt: "System Prompt",
      systemPromptPlaceholder: "Optional system prompt for this channel",
    },
    keys: {
      title: "API Keys",
      description: "Configure client API keys for local forwarding",
      add: "Add Key",
      empty: "No API keys configured yet.",
      noMatch: "No keys match your search.",
      new: "New API Key",
      name: "Name",
      key: "Key",
      delete: "Delete",
      deleteConfirm: (name: string) => `Delete "${name}"?`,
      namePlaceholder: "My App Key",
      keyPlaceholder: "sk-...",
      search: "Search keys…",
      copyKey: "Copy key",
      reveal: "Reveal key",
      colName: "Name",
      colStatus: "Status",
      colKey: "API Key",
      colQuota: "Quota",
      colUsage: "Usage",
      colModels: "Models",
      colIp: "IP Restriction",
      colGroup: "Group",
      colCreated: "Created",
      expired: "Expired",
      view: "View",
      totalLabel: "Total",
      created: "Key created",
      createdDesc: "Your new API key is ready to use.",
      deleted: "Key deleted",
      totalKeys: "Total Keys",
      totalRequests: "Total Requests",
      successRate: "Success Rate",
      totalTokens: "Total Tokens",
      requests: "Requests",
      tokens: "Tokens",
    },
    routes: {
      title: "Routes",
      description: "Model rewriting and routing rules",
      addMap: "Add",
      addRule: "Add",
      modelMaps: "Model Maps",
      modelMapsDesc: "Channel-specific model rewriting (glob patterns)",
      rules: "Route Rules",
      rulesDesc: "Priority-based request routing",
      ruleName: "Vision Models 鈫?GPT-4V",
      pattern: "Pattern (glob/regex)",
      target: "Target Model",
      patternPlaceholder: "gpt-4* or *",
      targetPlaceholder: "gpt-4-turbo",
      newMap: "New Model Map",
      newRule: "New Route Rule",
      ruleType: "Rule Type",
      autoHeuristic: "Auto Heuristic (model=auto)",
      keywordMatch: "Keyword Match",
      typeRouting: "Type Routing",
      emptyMap: "No model maps defined.",
      emptyRule: "No route rules defined.",
      selectChannel: "Select channel...",
    },
    logs: {
      title: "Request Logs",
      description: "Recent API requests",
      empty: "No requests logged yet. Make an API call to see logs.",
      time: "Time",
      method: "Method",
      path: "Path",
      model: "Model",
      channel: "Channel",
      status: "Status",
      duration: "Duration",
      ago: "just now",
      filterAll: "All",
      filter2xx: "2xx",
      filter4xx: "4xx",
      filter5xx: "5xx",
      filterError: "Errors",
      allModels: "All models",
      allChannels: "All channels",
      allKeys: "All keys",
      customRange: "Custom",
      startTime: "From",
      endTime: "To",
      statTotal: "Requests",
      searchPlaceholder: "Filter by path, model, error, channel",
      noError: "No error reported.",
      detail: {
        channel: "channel_id",
        tokens: "tokens_used",
        requestId: "request_id",
      },
      limit: "Limit",
      export: "Export",
      live: "Live",
      connecting: "Connecting…",
      connected: "Connected",
      disconnected: "Disconnected",
    },
    models: {
      title: "Models",
      description: "Browse available models across providers",
      search: "Search models…",
      allProviders: "All",
      context: "Context",
      type: "Type",
      copyName: "Copy name",
      copied: "Copied",
      allTypes: "All types",
      typeChat: "Chat",
      typeEmbedding: "Embedding",
      typeImage: "Image",
      typeAudio: "Audio",
    },
    systemSettings: {
      title: "System Settings",
      description: "Typed key-value configuration grouped by domain",
      entries: "options",
      empty: "No options in this section.",
      saved: "Option updated",
      saveFailed: "Could not update option",
      resetDefault: "Reset to default",
      sections: {
        site: "Site",
        auth: "Authentication",
        routing: "Routing",
        billing: "Billing",
        operations: "Operations",
        security: "Security",
        models: "Models",
      },
    },
    modelsMeta: {
      title: "Model Registry",      description: "Persistent model metadata: naming rules, vendors, endpoints and status",
      create: "New Model",
      edit: "Edit Model",
      modelName: "Model name",
      description2: "Description",
      vendor: "Vendor",
      icon: "Icon",
      tags: "Tags",
      nameRule: "Name rule",
      endpoints: "Endpoints",
      statusEnabled: "Enabled",
      syncOfficial: "Sync with official",
      sync: "Sync from channels",
      syncHint: "Create metadata rows for every model discovered on channels",
      syncDone: "Registry synchronized",
      syncCreated: "created",
      syncFailed: "Sync failed",
      missingHint: "models on channels have no metadata yet",
      search: "Search model, vendor or tag…",
      entries: "entries",
      empty: "No model metadata yet.",
      created: "Model metadata created",
      updated: "Model metadata updated",
      deleted: "Model metadata deleted",
    },
    settings: {
      title: "Settings",
      description: "Configure OxygenRouter behavior",
      ui: "Interface",
      uiDesc: "Visual and language preferences",
      tabGeneral: "General",
      tabDanger: "Danger",
      theme: "Theme",
      themeDesc: "Light or dark interface",
      themeDark: "Dark",
      themeLight: "Light",
      language: "Language",
      languageDesc: "Switch the WebUI between supported languages",
      server: "Server",
      listenHost: "Listen Host",
      listenHostDesc: "IP address the server binds to (127.0.0.1 for local only)",
      listenPort: "Listen Port",
      listenPortDesc: "HTTP port for WebUI and proxy",
      openBrowser: "Open Browser on Start",
      openBrowserDesc: "Automatically open the WebUI in your default browser when the app launches",
      routing: "Routing & Retry",
      maxRetries: "Max Retries",
      maxRetriesDesc: "How many times to retry a failed channel before giving up",
      retryDelay: "Initial Retry Delay (ms)",
      retryDelayDesc: "Base delay between retries",
      retryBackoff: "Retry Backoff",
      retryBackoffDesc: "How delay grows between attempts",
      backoffFixed: "Fixed",
      backoffLinear: "Linear",
      backoffExp: "Exponential",
      upstream: "Upstream",
      upstreamTimeout: "Upstream Timeout (ms)",
      upstreamTimeoutDesc: "Maximum time a single upstream call may take",
      userAgent: "User-Agent",
      userAgentDesc: "Sent on every request to upstream providers",
      concurrency: "Concurrency",
      maxConcurrent: "Max Concurrent Requests",
      maxConcurrentDesc: "Limit in-flight upstream requests",
      retention: "Retention",
      logRetention: "Request Log Retention (days)",
      logRetentionDesc: "0 keeps logs forever; positive values prune after N days",
      security: "Security",
      localToken: "Local API Token",
      localTokenDesc: "Bearer token for clients connecting to this router",
      showToken: "Show",
      hideToken: "Hide",
      logging: "Logging",
      logLevel: "Log Level",
      saved: "Settings saved.",
      advanced: "Advanced",
      advancedDesc: "Upstream, concurrency and retention controls",
      dangerZoneTitle: "Danger zone",
      dangerZoneSub: "These actions affect the local database and cannot be undone.",
      resetLogs: "Reset request log",
      resetLogsDesc: "Permanently delete all stored request log rows.",
      resetLogsBtn: "Reset log",
      resetLogsConfirm: "Delete ALL request log rows? This cannot be undone.",
      rotateToken: "Rotate local API token",
      rotateTokenDesc: "Generate a fresh token. All connected clients must be reconfigured.",
      rotateTokenConfirm: "Generate a new local API token? Existing clients will need to be reconfigured.",
    },
    systemInfo: {
      title: "System Info",
      description: "Server runtime, database stats, and backup",
      uptime: "Uptime",
      sinceStart: "Since start",
      dbSize: "Database Size",
      sqlite: "SQLite file",
      logCount: "Request Logs",
      stored: "Stored rows",
      channels: "Channels",
      enabledTotal: "enabled / total",
      apiKeys: "API Keys",
      configured: "Configured",
      modelMaps: "Model Maps",
      active: "Active",
      server: "Server",
      listenAddress: "Listen address",
      localToken: "Local API token",
      maxRetries: "Max retries",
      upstreamTimeout: "Upstream timeout",
      maxConcurrent: "Max concurrent",
      routeRules: "Route rules",
      runtime: "Runtime",
      version: "Version",
      platform: "Platform",
      architecture: "Architecture",
      rustc: "Rust compiler",
      buildProfile: "Build profile",
      startedAt: "Started at",
      backup: "Backup",
      backupDesc: "Download a snapshot of the local database",
      backupTitle: "Create database backup",
      backupSub: "Generates a .db file via SQLite VACUUM INTO. Safe to run while the server is live.",
      backupBtn: "Download backup",
      backupPending: "Creating…",
      backupConfirm: "Generate a backup of the database now?",
      loadError: "Could not load system info.",
    },
    playground: {
      title: "Playground",
      description: "Test chat, embeddings, and image generation in-browser",
      tabChat: "Chat",
      tabEmbeddings: "Embeddings",
      tabImage: "Image",
      noChannel: "No enabled channels. Add a channel first to use the playground.",
      goChannels: "Go to Channels",
      settings: "Settings",
      channel: "Channel",
      model: "Model",
      temperature: "Temperature",
      maxTokens: "Max tokens",
      systemPrompt: "System prompt",
      size: "Size",
      startChat: "Start a conversation by typing a message below.",
      you: "You",
      assistant: "Assistant",
      thinking: "Thinking…",
      inputPlaceholder: "Type a message… (⌘+Enter to send)",
      send: "Send",
      cancel: "Cancel",
      clear: "Clear",
      latency: "Latency",
      tokens: "Tokens",
      estCost: "Est. cost",
      textToEmbed: "Text to embed",
      embed: "Embed",
      prompt: "Prompt",
      generate: "Generate",
      openImage: "Open generated image",
      stream: "Stream",
      buffered: "Buffered",
      streamMode: "Delivery",
      regenerate: "Regenerate",
      messageActions: "Message actions",
      requestFailed: "Request failed",
    },
    commandPalette: {
      title: "Command Palette",
      placeholder: "Type a command…",
      navigation: "Navigation",
      actions: "Actions",
      addChannel: "Add Channel",
      addKey: "Add Key",
      addRoute: "Add Route",
      clearLogs: "Clear Logs",
      refreshAll: "Refresh All",
      noResults: "No results",
    },
    analytics: {
      title: "Model Analytics",
      description: "Model call analysis, distribution, and usage statistics",
      loadError: "Failed to load analytics data.",
      tabModel: "Models",
      tabDistribution: "Distribution",
      tabUsers: "User Stats",
      totalRequests: "Total Requests",
      totalTokens: "Total Tokens",
      avgRPM: "Avg RPM",
      avgTPM: "Avg TPM",
      successRate: "Success Rate",
      statRequests: "All time",
      statTokens: "All time",
      statRPM: "Requests per minute",
      statTPM: "Tokens per minute",
      consumption: "Consumption Distribution",
      total: "Total",
      barChart: "Bar",
      areaChart: "Area",
      modelAnalysis: "Model Call Analysis",
      subTrend: "Call Trend",
      subDistribution: "Call Distribution",
      subRanking: "Call Ranking",
      filterTitle: "Analytics Filter",
      filter: "Filter",
      quickRange: "Quick Range",
      applyFilter: "Apply Filter",
      preferences: "Preferences",
      prefsTitle: "Default Settings",
      prefsDefaultRange: "Default Range",
      prefsDefaultChart: "Default Chart",
      distributionTitle: "Traffic Distribution",
      distributionEmpty: "Distribution data will appear here once there is sufficient traffic.",
      usersTitle: "User Statistics",
      usersEmpty: "User statistics will appear here once there is sufficient traffic.",
      ms: "ms",
      tabConsumers: "Consumers",
      tabFlow: "Flow",
      range: "Range",
      updated: "Updated",
      autoRefresh: "Auto refresh every 15 seconds",
      requests: "Requests",
      tokens: "Tokens",
      modelUsage: "Model usage",
      callTrend: "Model call trend",
      distribution: "Request distribution",
      rankings: "Model rankings",
      health: "Model health",
      model: "Model",
      avgLatency: "Avg latency",
      consumerUsage: "API key usage",
      consumerTrend: "API key trend",
      apiKey: "API key",
      flowTitle: "Request lineage",
      flowFilter: "Filter lineage",
      flowRequests: "Requests",
      flowTokens: "Tokens",
      noData: "No requests in this range.",
      retry: "Retry",
    },
    saas: {
      signIn: "Sign in", signUp: "Create account", wallet: "Wallet", plans: "Plans", subscriptions: "Subscriptions", users: "Users", billing: "Billing", adminRequired: "Administrator access is required.", plansDescription: "Available service quota plans.", signInToPurchase: "Sign in to purchase a plan.", loadingPlans: "Loading plans...", noPlans: "No plans are currently available.", subscribe: "Subscribe", confirmPurchase: "Confirm purchase", forTerm: "for", purchasePrompt: "This amount will be deducted from your wallet.", purchaseSuccess: "Subscription purchased", purchaseFailed: "Purchase failed", quota: "Quota", duration: "Duration", signInToViewSubscriptions: "Sign in to view subscriptions.", subscriptionsDescription: "Current and previous subscription records.", browsePlans: "Browse plans", loadingSubscriptions: "Loading subscriptions...", noSubscriptions: "No subscriptions yet.", plan: "Plan", status: "Status", started: "Started", expires: "Expires",
    },
  },
  zh: {
    app: {
      brand: "OxygenRouter",
      version: "v26 · Alpha",
      localMode: "本地模式",
      openNav: "打开导航",
      closeNav: "关闭导航",
      collapse: "收起导航",
      expand: "展开导航",
      workspace: "工作区",
    },
    common: {
      save: "保存",
      saving: "保存中…",
      saved: "已保存",
      cancel: "取消",
      create: "创建",
      edit: "编辑",
      delete: "删除",
      refresh: "刷新",
      search: "搜索",
      copy: "复制",
      copied: "已复制",
      minutes: "分钟",
      hours: "Сʱ",
      days: "天",
      enabled: "已启用",
      disabled: "已停用",
      yes: "是",
      no: "否",
      confirm: "确认",
      language: "语言",
      backToList: "返回列表",
      notFound: "接口不存在",
    },
    nav: {
      overview: "概览",
      dashboard: "仪表盘",
      analytics: "模型分析",
      channels: "渠道",
      keys: "API 密钥",
      routes: "路由",
      logs: "请求日志",
      settings: "设置",
      system: "系统信息",
      systemSettings: "系统设置",
      playground: "演练场",
      models: "模型",
      modelsMeta: "模型注册表",
      wallet: "钱包",
      plans: "套餐",
      subscriptions: "订阅",
      users: "用户",
      billing: "计费",
    },
    dashboard: {
      title: "仪表盘",
      description: "本地路由健康与使用情况",
      eyebrow: "OXYGENROUTER / 本地实例",
      heading: "统一管理本地模型流量。",
      intro: "一个本地 OpenAI 兼容地址即可对接所有已配置渠道，路由与请求历史只保存在本机。",
      endpoint: "OpenAI 基础地址",
      copyAddress: "复制地址",
      activeChannels: (n: number) => (n === 0 ? "请添加第一个渠道" : `${n} 个启用渠道`),
      bannerTitle: "几分钟内开始使用你的 API 网关",
      bannerReady: "全部系统正常运行",
      bannerSub: "设置进度：",
      createApiKey: "创建 API 密钥",
      addChannel: "添加渠道",
      viewLogs: "查看日志",
      usageOverview: "用量概览",
      monitorBalance: "监控余额、用量和请求量",
      todayConsumption: "24 小时消耗",
      last24h: "近 24 小时",
      totalUsage: "历史使用情况",
      totalConsumption: "总消耗 (USD)",
      requestCount: "请求数",
      totalRequests: "总请求数",
      remainingBalance: "剩余额度",
      statusNormal: "正常",
      availableTime: "可用时长",
      noUsage: "暂无使用记录",
      wallet: "钱包",
      performanceHealth: "性能健康",
      last24hPerf: "最近 24 小时的性能指标",
      successRate: "成功率",
      avgLatency: "平均延迟",
      throughput: "吞吐量",
      requestsToday: "次请求",
      quickActions: "快捷操作",
      createApiKeyDesc: "为你的应用或服务创建密钥",
      viewLogsDesc: "查看请求、错误和计费详情",
      viewAnalyticsDesc: "查看详细使用分析",
      configureDesc: "配置路由器设置",
      apiInfo: "API 信息",
      routingEnabled: "路由已启用",
      currentDomain: "当前域名",
      authConfigured: "认证已配置",
      requiresApiKey: "需要 API 密钥",
      noKeys: "未配置密钥",
      modelSelected: "已选择模型",
      mappingsActive: "个映射已激活",
      noMappings: "未配置映射",
      announcements: "公告",
      noAnnouncements: "目前暂无公告",
      metrics: {
        total: "总请求数",
        success: "成功率",
        latency: "平均延迟",
        channels: "启用渠道",
        today: "今日",
        across: "全量本地日志",
        failed: "失败请求",
        healthy: "全部渠道运行正常",
      },
      byModel: "按模型统计",
      byChannel: "按渠道统计",
      localData: "本地数据",
      recent: "最近活动",
      recentSub: "最近捕获的请求",
      live: "ʵʱ",
      updated: "更新于",
      localNote: "数据仅保留在本机",
      getStarted: "快速开始",
      getStartedSub: "三步完成本地配置",
      step1: { title: "添加上游渠道", desc: "设置 Base URL、API Key、优先级与权重。" },
      step2: { title: "创建本地 API 密钥", desc: "在客户端配置中作为 Bearer Token 使用。" },
      step3: { title: "查看请求活动", desc: "审查延迟、状态、模型与所用渠道。" },
      services: "本地服务",
      servicesSub: "运行状态",
      servicesProxy: "代理监听",
      servicesStorage: "存储",
      servicesLog: "请求日志",
      servicesTokens: "Token 累计",
      channelsRoute: "前往渠道",
      keysRoute: "前往密钥",
      logsRoute: "前往日志",
      noRequests: "发起一次请求即可填充仪表盘。",
      noData: "暂无本地数据。",
      noModels: "发起一次请求以查看模型使用情况。",
      noChannels: "配置渠道以开始路由。",
      errors: "次错误",
      noErrors: "无错误",
      ms: "ms",
    },
    channels: {
      title: "渠道",
      description: "管理上游 API 提供商及其路由优先级",
      add: "添加渠道",
      empty: "暂无渠道，请添加第一个上游提供商。",
      new: "新建渠道",
      edit: "编辑渠道",
      name: "名称",
      provider: "提供商",
      baseUrl: "Base URL",
      apiKey: "API Key",
      priority: "优先级",
      weight: "权重",
      testModel: "测试模型",
      syncModels: "同步上游模型",
      modelsSynced: "模型已同步",
      modelsSyncFailed: "模型同步失败",
      modelsAvailable: "个可用模型",
      multiKeys: "多密钥管理",
      multiKeysDesc: "管理该渠道的单个 API 密钥，包括健康状态与轮换模式",
      keysEnabled: "个密钥可用",
      keyMode: "轮换模式",
      keyModeRandom: "随机",
      keyModePolling: "轮询",
      enableKey: "启用",
      disableKey: "停用",
      enableAllKeys: "全部启用",
      disableAllKeys: "全部停用",
      autoDisabled: "自动停用",
      keyActionFailed: "密钥操作失败",
      appendKeys: "追加密钥",
      appendKeysBtn: "追加",
      deleteDisabledKeys: "清理自动停用",
      test: "测试渠道",
      testAll: "测试全部",
      testing: "测试中…",
      delete: "删除渠道",
      deleteConfirm: (name: string) => `确定删除渠道 "${name}"?`,
      namePlaceholder: "OpenAI 官方",
      baseUrlPlaceholder: "https://api.openai.com",
      ok: "成功",
      err: "失败",
      search: "搜索渠道",
      showDisabled: "显示已停用",
      allProviders: "全部提供商",
      allGroups: "全部分组",
      group: "分组",
      enabledLabel: "已启用",
      copyUrl: "复制 Base URL",
      clickEnable: "启用渠道",
      clickDisable: "停用渠道",
      selected: "已选择 X 项",
      enableSelected: "启用所选",
      disableSelected: "禁用所选",
      deleteSelected: "删除所选",
      deleteSelectedConfirm: "确定删除 X 个所选渠道？此操作不可恢复。",
      models: "模型",
      modelSearchPlaceholder: "搜索模型...",
      fillModels: "填充",
      clearModels: "清空",
      customModelPlaceholder: "输入自定义模型名",
      batchMode: "批量 Key 模式（每行一个）",
      batchKeyPlaceholder: "每行一个 API Key",
      advancedSettings: "高级设置",
      modelMapping: "模型映射 (JSON)",
      systemPrompt: "系统提示",
      systemPromptPlaceholder: "注入到 system 消息（可选）",
    },
    keys: {
      title: "API 密钥",
      description: "配置本地客户端转发用的 API 密钥",
      add: "添加密钥",
      empty: "暂无 API 密钥。",
      noMatch: "没有匹配搜索的密钥。",
      new: "新建 API 密钥",
      name: "名称",
      key: "密钥",
      delete: "删除",
      deleteConfirm: (name: string) => `确定删除 "${name}"?`,
      namePlaceholder: "我的应用",
      keyPlaceholder: "sk-...",
      search: "搜索密钥…",
      copyKey: "复制密钥",
      reveal: "显示密钥",
      colName: "名称",
      colStatus: "״̬",
      colKey: "密钥",
      colQuota: "额度",
      colUsage: "用量",
      colModels: "模型限制",
      colIp: "IP 限制",
      colGroup: "分组",
      colCreated: "创建时间",
      expired: "已过期",
      view: "视图",
      totalLabel: "总计",
      created: "密钥已创建",
      createdDesc: "新的 API 密钥已可用。",
      deleted: "密钥已删除",
      totalKeys: "密钥总数",
      totalRequests: "请求总数",
      successRate: "成功率",
      totalTokens: "Token 总数",
      requests: "请求",
      tokens: "Tokens",
    },
    routes: {
      title: "路由",
      description: "模型重写与路由规则",
      addMap: "添加",
      addRule: "添加",
      modelMaps: "模型映射",
      modelMapsDesc: "针对渠道的模型重写（支持通配）",
      rules: "路由规则",
      rulesDesc: "基于优先级的请求路由",
      ruleName: "视觉模型 → GPT-4V",
      pattern: "匹配模式（通配/正则）",
      target: "目标模型",
      patternPlaceholder: "gpt-4* 鎴?*",
      targetPlaceholder: "gpt-4-turbo",
      newMap: "新建模型映射",
      newRule: "新建路由规则",
      ruleType: "规则类型",
      autoHeuristic: "自动启发（model=auto）",
      keywordMatch: "关键字匹配",
      typeRouting: "类型路由",
      emptyMap: "暂无模型映射。",
      emptyRule: "暂无路由规则。",
      selectChannel: "选择渠道...",
    },
    logs: {
      title: "请求日志",
      description: "鏈€杩戠殑 API 璇锋眰",
      empty: "暂无请求记录，发起一次调用即可看到。",
      time: "时间",
      method: "方法",
      path: "路径",
      model: "模型",
      channel: "渠道",
      status: "״̬",
      duration: "耗时",
      ago: "刚刚",
      filterAll: "全部",
      filter2xx: "2xx",
      filter4xx: "4xx",
      filter5xx: "5xx",
      filterError: "错误",
      allModels: "全部模型",
      allChannels: "全部渠道",
      allKeys: "全部密钥",
      customRange: "自定义",
      startTime: "开始",
      endTime: "结束",
      statTotal: "请求数",
      searchPlaceholder: "按路径、模型、错误、渠道过滤",
      noError: "无错误报告。",
      detail: {
        channel: "渠道ID",
        tokens: "使用Token",
        requestId: "请求ID",
      },
      limit: "条数",
      export: "导出",
      live: "ʵʱ",
      connecting: "连接中…",
      connected: "已连接",
      disconnected: "宸叉柇寮€",
    },
    models: {
      title: "模型",
      description: "浏览各提供商的可用模型",
      search: "搜索模型…",
      allProviders: "全部",
      context: "上下文",
      type: "类型",
      copyName: "复制名称",
      copied: "已复制",
      allTypes: "全部类型",
      typeChat: "聊天",
      typeEmbedding: "嵌入",
      typeImage: "图像",
      typeAudio: "音频",
    },
    systemSettings: {
      title: "系统设置",
      description: "按域分组的强类型键值配置",
      entries: "个配置项",
      empty: "该分区暂无配置项。",
      saved: "配置已更新",
      saveFailed: "配置更新失败",
      resetDefault: "恢复默认值",
      sections: {
        site: "站点",
        auth: "认证",
        routing: "路由",
        billing: "计费",
        operations: "运维",
        security: "安全",
        models: "模型",
      },
    },
    modelsMeta: {
      title: "模型注册表",
      description: "持久化模型元数据：命名规则、供应商、端点与状态",
      create: "新建模型",
      edit: "编辑模型",
      modelName: "模型名称",
      description2: "描述",
      vendor: "供应商",
      icon: "图标",
      tags: "标签",
      nameRule: "命名规则",
      endpoints: "端点",
      statusEnabled: "启用",
      syncOfficial: "与官方同步",
      sync: "从渠道同步",
      syncHint: "为渠道中发现的每个模型创建元数据行",
      syncDone: "注册表已同步",
      syncCreated: "条已创建",
      syncFailed: "同步失败",
      missingHint: "个渠道模型尚无元数据",
      search: "搜索模型、供应商或标签…",
      entries: "条记录",
      empty: "暂无模型元数据。",
      created: "模型元数据已创建",
      updated: "模型元数据已更新",
      deleted: "模型元数据已删除",
    },
    settings: {
      title: "设置",
      description: "配置 OxygenRouter 行为",
      ui: "界面",
      uiDesc: "外观与语言偏好",
      tabGeneral: "通用",
      tabDanger: "危险",
      theme: "主题",
      themeDesc: "亮色或暗色界面",
      themeDark: "暗色",
      themeLight: "亮色",
      language: "语言",
      languageDesc: "在受支持的语言之间切换 WebUI",
      server: "服务",
      listenHost: "监听地址",
      listenHostDesc: "服务绑定的 IP（127.0.0.1 仅本地）",
      listenPort: "监听端口",
      listenPortDesc: "WebUI 与代理的 HTTP 端口",
      openBrowser: "启动时打开浏览器",
      openBrowserDesc: "应用启动时自动在默认浏览器中打开 WebUI",
      routing: "路由与重试",
      maxRetries: "最大重试次数",
      maxRetriesDesc: "失败渠道的重试次数上限",
      retryDelay: "初始重试延迟（ms）",
      retryDelayDesc: "閲嶈瘯涔嬮棿鐨勫熀纭€寤惰繜",
      retryBackoff: "退避策略",
      retryBackoffDesc: "重试延迟的增长方式",
      backoffFixed: "固定",
      backoffLinear: "线性",
      backoffExp: "指数",
      upstream: "上游",
      upstreamTimeout: "上游超时（ms）",
      upstreamTimeoutDesc: "单次上游请求的最长允许时间",
      userAgent: "User-Agent",
      userAgentDesc: "鍙戦€佸埌涓婃父鐨?User-Agent 鏍囪瘑",
      concurrency: "并发",
      maxConcurrent: "最大并发请求数",
      maxConcurrentDesc: "同时进行的上游请求数上限",
      retention: "保留",
      logRetention: "请求日志保留天数",
      logRetentionDesc: "0 表示永久保留，正数表示超过 N 天后清理",
      security: "安全",
      localToken: "本地 API 令牌",
      localTokenDesc: "客户端连接本路由时使用的 Bearer 令牌",
      showToken: "显示",
      hideToken: "隐藏",
      logging: "日志",
      logLevel: "日志级别",
      saved: "已保存。",
      advanced: "高级",
      advancedDesc: "上游、并发与保留策略",
      dangerZoneTitle: "危险操作",
      dangerZoneSub: "这些操作会影响本地数据库且不可恢复。",
      resetLogs: "重置请求日志",
      resetLogsDesc: "永久删除所有存储的请求日志记录。",
      resetLogsBtn: "重置日志",
      resetLogsConfirm: "确定要删除所有请求日志吗？此操作不可恢复。",
      rotateToken: "重置本地 API 令牌",
      rotateTokenDesc: "生成新的令牌，所有已连接的客户端都需要重新配置。",
      rotateTokenConfirm: "确定要生成新的本地 API 令牌吗？已有客户端需要重新配置。",
    },
    systemInfo: {
      title: "系统信息",
      description: "服务器运行时、数据库状态与备份",
      uptime: "运行时间",
      sinceStart: "自启动以来",
      dbSize: "数据库大小",
      sqlite: "SQLite 文件",
      logCount: "请求日志",
      stored: "已存储行数",
      channels: "渠道",
      enabledTotal: "启用 / 总数",
      apiKeys: "API 密钥",
      configured: "已配置",
      modelMaps: "模型映射",
      active: "活跃",
      server: "服务",
      listenAddress: "监听地址",
      localToken: "本地 API 令牌",
      maxRetries: "最大重试",
      upstreamTimeout: "上游超时",
      maxConcurrent: "最大并发",
      routeRules: "路由规则",
      runtime: "运行时",
      version: "版本",
      platform: "ƽ̨",
      architecture: "架构",
      rustc: "Rust 编译器",
      buildProfile: "构建模式",
      startedAt: "启动时间",
      backup: "备份",
      backupDesc: "下载本地数据库的快照",
      backupTitle: "创建数据库备份",
      backupSub: "通过 SQLite VACUUM INTO 生成 .db 文件。服务运行时可安全执行。",
      backupBtn: "下载备份",
      backupPending: "正在创建…",
      backupConfirm: "确定要立即生成数据库备份吗？",
      loadError: "无法加载系统信息。",
    },
    playground: {
      title: "演练场",
      description: "在浏览器中测试聊天、嵌入与图像生成",
      tabChat: "聊天",
      tabEmbeddings: "嵌入",
      tabImage: "图像",
      noChannel: "暂无启用的渠道。请先添加一个渠道以使用演练场。",
      goChannels: "前往渠道",
      settings: "设置",
      channel: "渠道",
      model: "模型",
      temperature: "温度",
      maxTokens: "鏈€澶?Token",
      systemPrompt: "系统提示",
      size: "尺寸",
      startChat: "在下方输入消息开始对话。",
      you: "你",
      assistant: "助手",
      thinking: "鎬濊€冧腑…",
      inputPlaceholder: "输入消息… (⌘+Enter 发送)",
      send: "发送",
      cancel: "取消",
      clear: "清空",
      latency: "延迟",
      tokens: "Tokens",
      estCost: "预估成本",
      textToEmbed: "待嵌入文本",
      embed: "嵌入",
      prompt: "提示",
      generate: "生成",
      openImage: "打开生成的图像",
      stream: "流式",
      buffered: "缓冲",
      streamMode: "输出方式",
      regenerate: "重新生成",
      messageActions: "消息操作",
      requestFailed: "请求失败",
    },
    commandPalette: {
      title: "命令面板",
      placeholder: "杈撳叆鍛戒护…",
      navigation: "导航",
      actions: "操作",
      addChannel: "添加渠道",
      addKey: "添加密钥",
      addRoute: "添加路由",
      clearLogs: "清空日志",
      refreshAll: "全部刷新",
      noResults: "无匹配结果",
    },
    analytics: {
      title: "模型调用分析",
      description: "模型调用分析、分布与用量统计",
      loadError: "分析数据加载失败。",
      tabModel: "模型调用分析",
      tabDistribution: "分布",
      tabUsers: "用户",
      totalRequests: "总请求数",
      totalTokens: "总 Token 数",
      avgRPM: "平均 RPM",
      avgTPM: "平均 TPM",
      successRate: "成功率",
      statRequests: "请求",
      statTokens: "Tokens",
      statRPM: "RPM",
      statTPM: "TPM",
      consumption: "用量分布",
      total: "总计",
      barChart: "柱状图",
      areaChart: "面积图",
      modelAnalysis: "模型调用分析",
      subTrend: "调用趋势",
      subDistribution: "占比",
      subRanking: "排行",
      filterTitle: "分析筛选",
      filter: "ɸѡ",
      quickRange: "快捷范围",
      applyFilter: "搴旂敤绛涢€夊櫒",
      preferences: "偏好设置",
      prefsTitle: "默认设置",
      prefsDefaultRange: "默认时间范围",
      prefsDefaultChart: "默认图表类型",
      distributionTitle: "用量分布",
      distributionEmpty: "有足够流量后，分布数据将显示在这里。",
      usersTitle: "用户分析",
      usersEmpty: "有足够流量后，用户统计将显示在这里。",
      ms: "ms",
      tabConsumers: "消费者",
      tabFlow: "流向",
      range: "范围",
      updated: "更新于",
      autoRefresh: "每 15 秒自动刷新",
      requests: "请求数",
      tokens: "Token 数",
      modelUsage: "模型使用情况",
      callTrend: "模型调用趋势",
      distribution: "分布",
      rankings: "模型排行",
      health: "模型健康",
      model: "模型",
      avgLatency: "平均延迟",
      consumerUsage: "消费者用量",
      consumerTrend: "消费者趋势",
      apiKey: "API 密钥",
      flowTitle: "请求链路",
      flowFilter: "过滤节点…",
      flowRequests: "按请求数",
      flowTokens: "按 Token 数",
      noData: "所选范围内暂无数据。",
      retry: "重试",
    },
    saas: {
      signIn: "登录",
    },
  },
} as const;

export type Strings = {
  app: {
    brand: string;
    version: string;
    localMode: string;
    openNav: string;
    closeNav: string;
    collapse: string;
    expand: string;
    workspace: string;
  };
  common: {
    save: string;
    saving: string;
    saved: string;
    cancel: string;
    create: string;
    edit: string;
    delete: string;
    refresh: string;
    search: string;
    copy: string;
    copied: string;
    minutes: string;
    hours: string;
    days: string;
    enabled: string;
    disabled: string;
    yes: string;
    no: string;
    confirm: string;
    language: string;
    backToList: string;
    notFound: string;
  };
  nav: {
    overview: string;
    dashboard: string;
    analytics: string;
    channels: string;
    keys: string;
    routes: string;
    logs: string;
    settings: string;
    system: string;
    playground: string;
    models: string;
    modelsMeta: string;
    systemSettings: string;
    wallet: string;
    plans: string;
    subscriptions: string;
    users: string;
    billing: string;
  };
  dashboard: {
    title: string;
    description: string;
    eyebrow: string;
    heading: string;
    intro: string;
    endpoint: string;
    copyAddress: string;
    activeChannels: (n: number) => string;
    bannerTitle: string;
    bannerReady: string;
    bannerSub: string;
    createApiKey: string;
    addChannel: string;
    viewLogs: string;
    usageOverview: string;
    monitorBalance: string;
    todayConsumption: string;
    last24h: string;
    totalUsage: string;
    totalConsumption: string;
    requestCount: string;
    totalRequests: string;
    remainingBalance: string;
    statusNormal: string;
    availableTime: string;
    noUsage: string;
    wallet: string;
    performanceHealth: string;
    last24hPerf: string;
    successRate: string;
    avgLatency: string;
    throughput: string;
    requestsToday: string;
    quickActions: string;
    createApiKeyDesc: string;
    viewLogsDesc: string;
    viewAnalyticsDesc: string;
    configureDesc: string;
    apiInfo: string;
    routingEnabled: string;
    currentDomain: string;
    authConfigured: string;
    requiresApiKey: string;
    noKeys: string;
    modelSelected: string;
    mappingsActive: string;
    noMappings: string;
    announcements: string;
    noAnnouncements: string;
    metrics: {
      total: string;
      success: string;
      latency: string;
      channels: string;
      today: string;
      across: string;
      failed: string;
      healthy: string;
    };
    byModel: string;
    byChannel: string;
    localData: string;
    recent: string;
    recentSub: string;
    live: string;
    updated: string;
    localNote: string;
    getStarted: string;
    getStartedSub: string;
    step1: { title: string; desc: string };
    step2: { title: string; desc: string };
    step3: { title: string; desc: string };
    services: string;
    servicesSub: string;
    servicesProxy: string;
    servicesStorage: string;
    servicesLog: string;
    servicesTokens: string;
    channelsRoute: string;
    keysRoute: string;
    logsRoute: string;
    noRequests: string;
    noData: string;
    noModels: string;
    noChannels: string;
    errors: string;
    noErrors: string;
    ms: string;
  };
  channels: {
    title: string;
    description: string;
    add: string;
    empty: string;
    new: string;
    edit: string;
    name: string;
    provider: string;
    baseUrl: string;
    apiKey: string;
    priority: string;
    weight: string;
    testModel: string;
    syncModels: string;
    modelsSynced: string;
    modelsSyncFailed: string;
    modelsAvailable: string;
    multiKeys: string;
    multiKeysDesc: string;
    keysEnabled: string;
    keyMode: string;
    keyModeRandom: string;
    keyModePolling: string;
    enableKey: string;
    disableKey: string;
    enableAllKeys: string;
    disableAllKeys: string;
    autoDisabled: string;
    keyActionFailed: string;
    appendKeys: string;
    appendKeysBtn: string;
    deleteDisabledKeys: string;
    test: string;
    testAll: string;
    testing: string;
    delete: string;
    deleteConfirm: (name: string) => string;
    namePlaceholder: string;
    baseUrlPlaceholder: string;
    ok: string;
    err: string;
    search: string;
    showDisabled: string;
    allProviders: string;
    enabledLabel: string;
    copyUrl: string;
    clickEnable: string;
    clickDisable: string;
    selected: string;
    enableSelected: string;
    disableSelected: string;
    deleteSelected: string;
    deleteSelectedConfirm: string;
    allGroups: string;
    group: string;
    models: string;
    modelSearchPlaceholder: string;
    fillModels: string;
    clearModels: string;
    customModelPlaceholder: string;
    batchMode: string;
    batchKeyPlaceholder: string;
    advancedSettings: string;
    modelMapping: string;
    systemPrompt: string;
    systemPromptPlaceholder: string;
  };
  keys: {
    title: string;
    description: string;
    add: string;
    empty: string;
    noMatch: string;
    new: string;
    name: string;
    key: string;
    delete: string;
    deleteConfirm: (name: string) => string;
    namePlaceholder: string;
    keyPlaceholder: string;
    search: string;
    copyKey: string;
    reveal: string;
    colName: string;
    colStatus: string;
    colKey: string;
    colQuota: string;
    colUsage: string;
    colModels: string;
    colIp: string;
    colGroup: string;
    colCreated: string;
    expired: string;
    view: string;
    totalLabel: string;
    created: string;
    createdDesc: string;
    deleted: string;
    totalKeys: string;
    totalRequests: string;
    successRate: string;
    totalTokens: string;
    requests: string;
    tokens: string;
  };
  routes: {
    title: string;
    description: string;
    addMap: string;
    addRule: string;
    modelMaps: string;
    modelMapsDesc: string;
    rules: string;
    rulesDesc: string;
    ruleName: string;
    pattern: string;
    target: string;
    patternPlaceholder: string;
    targetPlaceholder: string;
    newMap: string;
    newRule: string;
    ruleType: string;
    autoHeuristic: string;
    keywordMatch: string;
    typeRouting: string;
    emptyMap: string;
    emptyRule: string;
    selectChannel: string;
  };
  logs: {
    title: string;
    description: string;
    empty: string;
    time: string;
    method: string;
    path: string;
    model: string;
    channel: string;
    status: string;
    duration: string;
    ago: string;
    filterAll: string;
    filter2xx: string;
    filter4xx: string;
    filter5xx: string;
    filterError: string;
    allModels: string;
    allChannels: string;
    allKeys: string;
    customRange: string;
    startTime: string;
    endTime: string;
    statTotal: string;
    searchPlaceholder: string;
    noError: string;
    detail: {
      channel: string;
      tokens: string;
      requestId: string;
    };
    limit: string;
    export: string;
    live: string;
    connecting: string;
    connected: string;
    disconnected: string;
  };
  models: {
    title: string;
    description: string;
    search: string;
    allProviders: string;
    context: string;
    type: string;
    copyName: string;
    copied: string;
    allTypes: string;
    typeChat: string;
    typeEmbedding: string;
    typeImage: string;
    typeAudio: string;
  };
  systemSettings: {
    title: string;
    description: string;
    entries: string;
    empty: string;
    saved: string;
    saveFailed: string;
    resetDefault: string;
    sections: {
      site: string;
      auth: string;
      routing: string;
      billing: string;
      operations: string;
      security: string;
      models: string;
    };
  };
  modelsMeta: {
    title: string;
    description: string;
    create: string;
    edit: string;
    modelName: string;
    description2: string;
    vendor: string;
    icon: string;
    tags: string;
    nameRule: string;
    endpoints: string;
    statusEnabled: string;
    syncOfficial: string;
    sync: string;
    syncHint: string;
    syncDone: string;
    syncCreated: string;
    syncFailed: string;
    missingHint: string;
    search: string;
    entries: string;
    empty: string;
    created: string;
    updated: string;
    deleted: string;
  };
  settings: {
    title: string;
    description: string;
    ui: string;
    uiDesc: string;
    tabGeneral: string;
    tabDanger: string;
    theme: string;
    themeDesc: string;
    themeDark: string;
    themeLight: string;
    language: string;
    languageDesc: string;
    server: string;
    listenHost: string;
    listenHostDesc: string;
    listenPort: string;
    listenPortDesc: string;
    openBrowser: string;
    openBrowserDesc: string;
    routing: string;
    maxRetries: string;
    maxRetriesDesc: string;
    retryDelay: string;
    retryDelayDesc: string;
    retryBackoff: string;
    retryBackoffDesc: string;
    backoffFixed: string;
    backoffLinear: string;
    backoffExp: string;
    upstream: string;
    upstreamTimeout: string;
    upstreamTimeoutDesc: string;
    userAgent: string;
    userAgentDesc: string;
    concurrency: string;
    maxConcurrent: string;
    maxConcurrentDesc: string;
    retention: string;
    logRetention: string;
    logRetentionDesc: string;
    security: string;
    localToken: string;
    localTokenDesc: string;
    showToken: string;
    hideToken: string;
    logging: string;
    logLevel: string;
    saved: string;
    advanced: string;
    advancedDesc: string;
    dangerZoneTitle: string;
    dangerZoneSub: string;
    resetLogs: string;
    resetLogsDesc: string;
    resetLogsBtn: string;
    resetLogsConfirm: string;
    rotateToken: string;
    rotateTokenDesc: string;
    rotateTokenConfirm: string;
  };
  systemInfo: {
    title: string;
    description: string;
    uptime: string;
    sinceStart: string;
    dbSize: string;
    sqlite: string;
    logCount: string;
    stored: string;
    channels: string;
    enabledTotal: string;
    apiKeys: string;
    configured: string;
    modelMaps: string;
    active: string;
    server: string;
    listenAddress: string;
    localToken: string;
    maxRetries: string;
    upstreamTimeout: string;
    maxConcurrent: string;
    routeRules: string;
    runtime: string;
    version: string;
    platform: string;
    architecture: string;
    rustc: string;
    buildProfile: string;
    startedAt: string;
    backup: string;
    backupDesc: string;
    backupTitle: string;
    backupSub: string;
    backupBtn: string;
    backupPending: string;
    backupConfirm: string;
    loadError: string;
  };
  playground: {
    title: string;
    description: string;
    tabChat: string;
    tabEmbeddings: string;
    tabImage: string;
    noChannel: string;
    goChannels: string;
    settings: string;
    channel: string;
    model: string;
    temperature: string;
    maxTokens: string;
    systemPrompt: string;
    size: string;
    startChat: string;
    you: string;
    assistant: string;
    thinking: string;
    inputPlaceholder: string;
    send: string;
    cancel: string;
    clear: string;
    latency: string;
    tokens: string;
    estCost: string;
    textToEmbed: string;
    embed: string;
    prompt: string;
    generate: string;
    openImage: string;
    stream: string;
    buffered: string;
    streamMode: string;
    regenerate: string;
    messageActions: string;
    requestFailed: string;
  };
  commandPalette: {
    title: string;
    placeholder: string;
    navigation: string;
    actions: string;
    addChannel: string;
    addKey: string;
    addRoute: string;
    clearLogs: string;
    refreshAll: string;
    noResults: string;
  };
  analytics: {
    title: string;
    description: string;
    loadError: string;
    tabModel: string;
    tabDistribution: string;
    tabUsers: string;
    totalRequests: string;
    totalTokens: string;
    avgRPM: string;
    avgTPM: string;
    successRate: string;
    statRequests: string;
    statTokens: string;
    statRPM: string;
    statTPM: string;
    consumption: string;
    total: string;
    barChart: string;
    areaChart: string;
    modelAnalysis: string;
    subTrend: string;
    subDistribution: string;
    subRanking: string;
    filterTitle: string;
    filter: string;
    quickRange: string;
    applyFilter: string;
    preferences: string;
    prefsTitle: string;
    prefsDefaultRange: string;
    prefsDefaultChart: string;
    distributionTitle: string;
    distributionEmpty: string;
    usersTitle: string;
    usersEmpty: string;
    ms: string;
    tabConsumers: string;
    tabFlow: string;
    range: string;
    updated: string;
    autoRefresh: string;
    requests: string;
    tokens: string;
    modelUsage: string;
    callTrend: string;
    distribution: string;
    rankings: string;
    health: string;
    model: string;
    avgLatency: string;
    consumerUsage: string;
    consumerTrend: string;
    apiKey: string;
    flowTitle: string;
    flowFilter: string;
    flowRequests: string;
    flowTokens: string;
    noData: string;
    retry: string;
  };
  saas: {
    signIn: string;
    signUp: string;
    wallet: string;
    plans: string;
    subscriptions: string;
    users: string;
    billing: string;
    adminRequired: string;
    plansDescription: string; signInToPurchase: string; loadingPlans: string; noPlans: string; subscribe: string; confirmPurchase: string; forTerm: string; purchasePrompt: string; purchaseSuccess: string; purchaseFailed: string; quota: string; duration: string; signInToViewSubscriptions: string; subscriptionsDescription: string; browsePlans: string; loadingSubscriptions: string; noSubscriptions: string; plan: string; status: string; started: string; expires: string;
  };
};
