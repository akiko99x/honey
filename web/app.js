(() => {
  "use strict";

  const $ = (selector, root = document) => root.querySelector(selector);
  const $$ = (selector, root = document) => [...root.querySelectorAll(selector)];
  const icon = (name) => `<svg aria-hidden="true"><use href="#i-${name}"/></svg>`;
  const esc = (value) => String(value ?? "").replace(/[&<>"']/g, (char) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;"
  })[char]);
  const readStore = (store, key) => { try { return store.getItem(key) || ""; } catch { return ""; } };
  const writeStore = (store, key, value) => { try { value ? store.setItem(key, value) : store.removeItem(key); } catch {} };

  // Untranslated strings render in English and can be localized incrementally.
  const I18N = {
    ru: {
      "Overview": "Обзор", "Issues": "Проблемы", "Nodes": "Ноды", "Inbounds": "Инбаунды",
      "Users": "Пользователи", "Groups": "Группы", "Traffic": "Трафик",
      "Subscriptions": "Подписки", "Domains": "Домены", "SSL/TLS": "SSL/TLS",
      "Rules": "Правила", "Logs": "Логи", "Settings": "Настройки", "Live": "Онлайн",
      "Active issues": "Активные проблемы", "Issue history": "История проблем",
      "Management": "Управление", "Fleet health": "Состояние нод", "Core versions": "Версии ядер",
      "Enrollment": "Подключение", "Catalog": "Каталог", "Protocols": "Протоколы",
      "Security": "Безопасность", "Create inbound": "Создать inbound", "Directory": "Список",
      "Access": "Доступ", "Quotas": "Квоты", "Lifecycle": "Жизненный цикл",
      "Coverage": "Покрытие", "Policies": "Политики", "Analytics": "Аналитика",
      "Live connections": "Активные соединения", "Quota usage": "Использование квот",
      "Geography": "География", "Client links": "Ссылки клиентов", "Client formats": "Форматы клиентов",
      "Delivery health": "Доставка", "Registry": "Реестр", "DNS status": "Состояние DNS",
      "ACME": "ACME", "REALITY": "REALITY",
      "Routing profiles": "Профили маршрутизации", "Client delivery": "Доставка клиентам",
      "Runtime": "Runtime", "Audit log": "Аудит", "Scheduled operations": "Запланированные операции",
      "General": "Общие", "Integrations": "Интеграции", "Software": "Программа",
      "Appearance": "Внешний вид",
      "self-hosted": "self-hosted", "checking": "проверка", "sign in": "войти",
      "sign out": "выйти", "master-agent": "master-agent",
      "master online": "master онлайн", "master offline": "master офлайн",
      "Quick search...": "Быстрый поиск...", "navigate": "навигация", "open": "открыть",
      "Cancel": "Отмена", "Save": "Сохранить", "Add": "Добавить", "Delete": "Удалить",
      "Edit": "Изменить", "Create": "Создать", "Close": "Закрыть",
      "Notifications": "Уведомления", "No unread alerts": "Нет непрочитанных",
      "Mark all read": "Прочитать все", "All severity": "Любая важность",
      "Critical": "Критичные", "Warning": "Предупреждения", "Info": "Инфо",
      "All events": "Все события", "Node down": "Нода упала", "Push failed": "Пуш не прошёл",
      "Certificate expiry": "Истечение сертификата", "Quota reset": "Сброс квоты",
      "Traffic spike": "Всплеск трафика", "Device limit": "Лимит устройств",
      "Config drift": "Дрейф конфига", "unread": "непрочит.",
      "Certificates": "Сертификаты", "Push history": "История пушей", "Activity": "Активность",
      "Access & routing": "Доступ и маршрутизация", "Subscription": "Подписка",
      "Security & TLS": "Безопасность и TLS", "Transport": "Транспорт",
      "Actions": "Действия", "System": "Система", "Pages": "Страницы",
    },
  };
  let lang = readStore(localStorage, "honey-lang");
  // English is the stable product default. A Russian choice is still respected
  // when explicitly saved from Settings.
  if (lang !== "ru" && lang !== "en") lang = "en";
  const t = (key) => (lang === "en" ? key : (I18N[lang] && I18N[lang][key]) || key);
  function applyStaticI18n(root = document) {
    root.querySelectorAll("[data-i18n]").forEach((el) => { el.textContent = t(el.dataset.i18n); });
    root.querySelectorAll("[data-i18n-ph]").forEach((el) => { el.placeholder = t(el.dataset.i18nPh); });
  }
  function setLang(next) {
    if (next !== "ru" && next !== "en") return;
    lang = next;
    writeStore(localStorage, "honey-lang", next);
    document.documentElement.lang = next;
    applyStaticI18n();
    buildMainNav();
    restoreGroups();
    ensureGroupOpen(state?.route || "overview");
    if (typeof state !== "undefined" && state.loaded) { updateCounts(); render(); updateSidebarScope(); }
  }
  const secretBackendLabel = (backend) => ({
    env: "environment variable", file: "mounted file", vault: "HashiCorp Vault",
    command: "external command", none: "not configured",
  })[backend] || backend || "not configured";
  const listRoutes = new Set(["overview", "issues", "nodes", "inbounds", "users", "groups", "traffic", "live", "subscriptions", "domains", "ssltls", "rules", "logs", "settings", "new-inbound", "new-node", "new-user"]);
  const detailKinds = new Set(["nodes", "inbounds", "users"]);
  // Top-level sections expand in place, like Cloudflare's dashboard navigation.
  // Each child is a real, focused screen; Overview stays the workspace home.
  const categorySections = {
    issues: [
      { key: "active", label: "Active issues", view: "issues" },
      { key: "history", label: "Issue history", view: "issue-history" },
    ],
    nodes: [
      { key: "management", label: "Management", view: "nodes" },
      { key: "health", label: "Fleet health", view: "node-health" },
      { key: "versions", label: "Core versions", view: "node-versions" },
      { key: "enrollment", label: "Enrollment", view: "node-enrollment" },
    ],
    inbounds: [
      { key: "catalog", label: "Catalog", view: "inbounds" },
      { key: "protocols", label: "Protocols", view: "inbound-protocols" },
      { key: "security", label: "Security", view: "inbound-security" },
      { key: "new", label: "Create inbound", view: "new-inbound" },
    ],
    users: [
      { key: "directory", label: "Directory", view: "users" },
      { key: "new", label: "Create users", view: "new-user" },
      { key: "access", label: "Access", view: "user-access" },
      { key: "quotas", label: "Quotas", view: "user-quotas" },
      { key: "lifecycle", label: "Lifecycle", view: "user-lifecycle" },
    ],
    groups: [
      { key: "directory", label: "Directory", view: "groups" },
      { key: "coverage", label: "Coverage", view: "group-coverage" },
      { key: "policies", label: "Policies", view: "group-policies" },
    ],
    traffic: [
      { key: "analytics", label: "Analytics", view: "traffic" },
      { key: "live", label: "Live connections", view: "live" },
      { key: "quotas", label: "Quota usage", view: "traffic-quotas" },
      { key: "geography", label: "Geography", view: "traffic-geography" },
    ],
    subscriptions: [
      { key: "links", label: "Client links", view: "subscriptions" },
      { key: "formats", label: "Client formats", view: "subscription-formats" },
      { key: "delivery", label: "Delivery health", view: "subscription-delivery" },
    ],
    domains: [
      { key: "registry", label: "Registry", view: "domains" },
      { key: "dns", label: "DNS status", view: "domain-dns" },
      { key: "certificates", label: "Certificates", view: "domain-certificates" },
    ],
    ssltls: [
      { key: "certificates", label: "Certificates", view: "tls-certificates" },
      { key: "acme", label: "ACME", view: "tls-acme" },
      { key: "reality", label: "REALITY", view: "tls-reality" },
    ],
    rules: [
      { key: "profiles", label: "Routing profiles", view: "rules" },
      { key: "coverage", label: "Coverage", view: "rule-coverage" },
      { key: "delivery", label: "Client delivery", view: "rule-delivery" },
    ],
    logs: [
      { key: "runtime", label: "Runtime", view: "logs" },
      { key: "audit", label: "Audit log", view: "audit-page" },
      { key: "scheduled", label: "Scheduled operations", view: "scheduled-page" },
    ],
    settings: [
      { key: "general", label: "General", view: "settings" },
      { key: "automation", label: "Automation", view: "settings" },
      { key: "security", label: "Security", view: "settings" },
      { key: "integrations", label: "Integrations", view: "settings" },
      { key: "software", label: "Software", view: "settings" },
      { key: "appearance", label: "Appearance", view: "settings" },
    ],
  };
  const navCategories = [
    { route: "issues", label: "Issues", icon: "issues", count: "nav-issues" },
    { route: "nodes", label: "Nodes", icon: "node", count: "nav-nodes" },
    { route: "inbounds", label: "Inbounds", icon: "inbound", count: "nav-inbounds" },
    { route: "users", label: "Users", icon: "users", count: "nav-users" },
    { route: "groups", label: "Groups", icon: "groups" },
    { route: "traffic", label: "Traffic", icon: "chart" },
    { route: "subscriptions", label: "Subscriptions", icon: "link" },
    { route: "domains", label: "Domains", icon: "globe" },
    { route: "ssltls", label: "SSL/TLS", icon: "lock" },
    { route: "rules", label: "Rules", icon: "rules" },
    { route: "logs", label: "Logs", icon: "logs" },
    { route: "settings", label: "Settings", icon: "settings" },
  ];
  const categoryFor = (route) => categorySections[route] || [];
  const categoryView = (route, section) => {
    const sections = categoryFor(route);
    return sections.find((s) => s.key === section)?.view || sections[0]?.view || route;
  };
  const detailSections = {
    nodes: [
      { key: "overview", label: "Overview", icon: "home" },
      { key: "inbounds", label: "Inbounds", icon: "inbound" },
      { key: "certs", label: "Certificates", icon: "lock" },
      { key: "pushes", label: "Push history", icon: "chart" },
      { key: "logs", label: "Activity", icon: "logs" },
    ],
    users: [
      { key: "overview", label: "Overview", icon: "home" },
      { key: "access", label: "Access & routing", icon: "groups" },
      { key: "subscription", label: "Subscription", icon: "link" },
    ],
    inbounds: [
      { key: "overview", label: "Overview", icon: "home" },
      { key: "security", label: "Security & TLS", icon: "lock" },
      { key: "transport", label: "Transport", icon: "link" },
    ],
  };
  const sectionsFor = (kind) => detailSections[kind] || [];
  const initialRoute = parseHash();
  let runtimeLogTimer = null;
  let runtimeLogSearchTimer = null;
  let notificationTimer = null;
  let liveTimer = null;
  const state = {
    route: initialRoute.route,
    detailId: initialRoute.id,
    detailSection: initialRoute.section,
    categorySection: initialRoute.categorySection,
    admin: null,
    nodes: [], users: [], inbounds: [], domains: [], profiles: [], groups: [], admins: [], nodeEdit: null,
    selection: { nodes: new Set(), users: new Set(), inbounds: new Set() },
    settings: null, branding: null, customRoles: [],
    userWiz: null,
    resultReturnRoute: "",
    issueReport: { counts: { total: 0, critical: 0, warning: 0, info: 0 }, issues: [] },
    issueFilters: { severity: "", kind: "", node: "" },
    savedViews: [],
    activeSavedViews: { nodes: "", inbounds: "", users: "", issues: "" },
    tableViews: {
      nodes: { search: "", labels: [], sort: "name", columns: ["name", "address", "status", "labels", "transport", "version", "last_seen", "actions"] },
      inbounds: { search: "", labels: [], sort: "tag", columns: ["tag", "node", "protocol", "labels", "core", "listen", "security", "status", "reach", "actions"] },
      users: { search: "", labels: [], sort: "username", columns: ["username", "uuid", "status", "labels", "traffic", "expires", "actions"] },
      issues: { search: "", labels: [], sort: "severity", columns: ["severity", "code", "issue", "type", "entity", "labels", "node", "detected", "actions"] },
    },
    loaded: false, loading: false,
    commandEntries: [], commandMatches: [], commandIndex: 0,
    agentLogs: Object.create(null), agentLogCursor: Object.create(null),
    agentLogStatus: Object.create(null), agentLogPolls: Object.create(null),
    logFilters: { node: "", level: "", code: "" },
    runtimeLogFilters: { level: "", code: "", query: "" },
    logPollInFlight: false, activityItems: [],
    notifications: [], notificationUnread: 0,
    notificationFilters: { severity: "", event: "", unread: false },
    onboarding: { completed: 0, total: 0, steps: [] },
    trafficAnalytics: { range: "24h", data: null, loading: false, error: "", node_id: "", user_id: "", core: "" },
    live: { data: null, loading: false, error: "" },
    geo: { data: null, loading: false, error: "" },
    nodeMetrics: Object.create(null),
    nodeDrift: Object.create(null),
  };

  const view = $("#view");
  const commandDialog = $("#command-dialog");
  const commandInput = $("#command-input");
  const commandResults = $("#command-results");
  const formDialog = $("#form-dialog");
  const entityForm = $("#entity-form");

  function parseHash() {
    const raw = location.hash.replace(/^#\/?/, "").split("?")[0];
    const parts = raw.split("/");
    const base = parts[0], route = listRoutes.has(base) ? base : "overview";
    if (detailKinds.has(route)) {
      if (categoryFor(route).some((s) => s.key === parts[1])) {
        return { route, id: null, section: null, categorySection: parts[1] };
      }
      return { route, id: parts[1] || null, section: parts[2] || null, categorySection: null };
    }
    const categorySection = parts[1] || null;
    return { route, id: null, section: null, categorySection };
  }

  function go(route, id, section) {
    if (!listRoutes.has(route)) route = "overview";
    const hash = detailKinds.has(route) && id
      ? (section ? `${route}/${id}/${section}` : `${route}/${id}`)
      : (section ? `${route}/${section}` : route);
    if (location.hash !== `#${hash}`) history.pushState(null, "", `#${hash}`);
    const isDetail = detailKinds.has(route) && Boolean(id);
    applyRoute(route, isDetail ? id : null, isDetail ? section : null, isDetail ? null : section);
  }

  function applyRoute(route, id, section, categorySection) {
    if (!listRoutes.has(route)) route = "overview";
    if (route !== state.route) clearAllSelections();
    state.route = route;
    state.detailId = detailKinds.has(route) ? (id || null) : null;
    // scoped section applies to any detail drill-in; default to overview.
    state.detailSection = detailKinds.has(state.route) && state.detailId
      ? (sectionsFor(state.route).some((s) => s.key === section) ? section : "overview")
      : null;
    state.categorySection = !state.detailId
      ? (categoryFor(state.route).some((s) => s.key === categorySection)
          ? categorySection
          : (categoryFor(state.route)[0]?.key || null))
      : null;
    ensureGroupOpen(route);
    updateMainNavState();
    document.body.classList.remove("sidebar-open");
    render();
    if (runtimeLogTimer) { clearInterval(runtimeLogTimer); runtimeLogTimer = null; }
    const activeView = categoryView(state.route, state.categorySection);
    if (activeView === "logs" || activeView === "issue-history" || (state.route === "nodes" && state.detailId)) {
      runtimeLogTimer = setInterval(pollVisibleLogs, 5000);
    }
    if (liveTimer) { clearInterval(liveTimer); liveTimer = null; }
    if (activeView === "live") {
      loadLiveConnections();
      loadGeo();
      liveTimer = setInterval(() => { loadLiveConnections(); loadGeo(); }, 5000);
    } else if (activeView === "traffic-geography") {
      loadGeo();
      liveTimer = setInterval(loadGeo, 5000);
    }
    if (state.route === "nodes" && state.detailId && (state.detailSection || "overview") === "overview") {
      loadNodeMetrics(state.detailId);
      loadNodeDrift(state.detailId);
    }
    view.focus({ preventScroll: true });
  }

  function ensureGroupOpen(route) {
    $(`.nav-item[data-route="${route}"]`)?.closest(".nav-group")?.classList.add("open");
    const group = $(`.nav-group[data-group="${route}"]`);
    group?.classList.add("open");
    group?.querySelector("[data-group-toggle]")?.setAttribute("aria-expanded", "true");
  }

  function toggleGroup(name) {
    const group = $(`.nav-group[data-group="${name}"]`);
    if (!group) return;
    group.classList.toggle("open");
    group.querySelector("[data-group-toggle]")?.setAttribute("aria-expanded", String(group.classList.contains("open")));
    let closed = {};
    try { closed = JSON.parse(readStore(localStorage, "honey-groups") || "{}"); } catch {}
    closed[name] = !group.classList.contains("open");
    writeStore(localStorage, "honey-groups", JSON.stringify(closed));
  }

  function restoreGroups() {
    let closed = {};
    try { closed = JSON.parse(readStore(localStorage, "honey-groups") || "{}"); } catch {}
    $$(".nav-group").forEach((group) => {
      const open = closed[group.dataset.group] === false || group.dataset.group === state.route;
      group.classList.toggle("open", open);
      group.querySelector("[data-group-toggle]")?.setAttribute("aria-expanded", String(open));
    });
  }

  function buildMainNav() {
    const root = $("#category-nav-root");
    if (!root) return;
    root.innerHTML = navCategories.map((category) => {
      const children = categoryFor(category.route).map((section) =>
        `<button class="nav-item nav-child" data-route="${esc(category.route)}" data-section="${esc(section.key)}"><span>${esc(t(section.label))}</span></button>`
      ).join("");
      const count = category.count ? `<b id="${category.count}">0</b>` : "";
      return `<section class="nav-group" data-group="${esc(category.route)}" data-nav-category="${esc(category.route)}">
        <button class="nav-group-head nav-category-head" data-group-toggle="${esc(category.route)}" aria-expanded="true">
          <svg class="grp-ico"><use href="#i-${category.icon}"/></svg>
          <span>${esc(t(category.label))}</span>${count}
          <svg class="grp-chev"><use href="#i-chevron"/></svg>
        </button>
        <div class="nav-group-body">${children}</div>
      </section>`;
    }).join("");
  }

  function updateMainNavState() {
    const activeSection = state.categorySection || categoryFor(state.route)[0]?.key || "";
    $$(".nav-item[data-route]").forEach((item) => {
      const active = item.dataset.route === state.route
        && (!item.dataset.section || item.dataset.section === activeSection);
      item.classList.toggle("active", active);
    });
    $$("[data-nav-category]").forEach((group) => {
      const active = group.dataset.navCategory === state.route;
      group.classList.toggle("active", active);
      group.querySelector(".nav-category-head")?.classList.toggle("active", active);
    });
  }

  async function api(path, options = {}) {
    const headers = new Headers(options.headers || {});
    headers.set("accept", "application/json");
    if (options.body && !headers.has("content-type")) headers.set("content-type", "application/json");
    const response = await fetch(path, { credentials: "same-origin", ...options, headers });
    if (response.status === 401) {
      const error = new Error("sign in required");
      error.auth = true;
      throw error;
    }
    const type = response.headers.get("content-type") || "";
    const data = response.status === 204 ? null : type.includes("json") ? await response.json() : await response.text();
    if (!response.ok) throw new Error(data?.error || data || `request failed (${response.status})`);
    return data;
  }

  async function checkHealth() {
    try {
      const response = await fetch("/health", { headers: { accept: "application/json" } });
      if (!response.ok) throw new Error();
      const dot = $("#health-dot");
      if (dot) dot.className = "health-dot ok";
      $("#health-copy").textContent = t("master online");
    } catch {
      const dot = $("#health-dot");
      if (dot) dot.className = "health-dot bad";
      $("#health-copy").textContent = t("master offline");
    }
  }

  function closeNotifications() {
    const panel = $("#notification-panel");
    const button = $("#notification-button");
    if (panel) panel.hidden = true;
    if (button) button.setAttribute("aria-expanded", "false");
  }

  function updateNotificationChrome() {
    const badge = $("#notification-badge");
    const summary = $("#notification-summary");
    const count = Math.max(0, Number(state.notificationUnread || 0));
    if (badge) {
      badge.hidden = count === 0;
      badge.textContent = count > 99 ? "99+" : String(count);
    }
    if (summary) summary.textContent = count ? `${count} unread alert${count === 1 ? "" : "s"}` : "No unread alerts";
  }

  function renderNotificationPanel() {
    const list = $("#notification-list");
    if (!list) return;
    list.innerHTML = state.notifications.length ? state.notifications.map((item) => {
      const read = Boolean(item.read_at);
      const repeats = Number(item.occurrence_count || 1) > 1 ? ` · ${item.occurrence_count} occurrences` : "";
      return `<button class="notification-item ${read ? "read" : ""}" data-action="open-notification" data-id="${esc(item.id)}" data-resource-type="${esc(item.resource_type || "")}" data-resource-id="${esc(item.resource_id || "")}">
        <i class="notification-severity ${esc(item.severity)}"></i>
        <span class="notification-copy"><b>${esc(item.title)}</b><span>${esc(item.body)}</span><small>${esc(item.code)} · ${relativeTime(item.last_seen_at || item.created_at)}${repeats}</small></span>
        ${read ? "" : '<i class="notification-unread"></i>'}
      </button>`;
    }).join("") : '<div class="notification-empty">No notifications match these filters.</div>';
    updateNotificationChrome();
  }

  async function refreshNotifications({ quiet = false } = {}) {
    if (!state.admin || state.admin.role === "reseller") return;
    const query = new URLSearchParams({ limit: "50" });
    if (state.notificationFilters.severity) query.set("severity", state.notificationFilters.severity);
    if (state.notificationFilters.event) query.set("event", state.notificationFilters.event);
    if (state.notificationFilters.unread) query.set("unread", "true");
    try {
      const [items, count] = await Promise.all([
        api(`/notifications?${query}`),
        api("/notifications/unread-count"),
      ]);
      state.notifications = items;
      state.notificationUnread = Number(count.unread || 0);
      renderNotificationPanel();
    } catch (error) {
      if (!quiet && !error.auth) toast(error.message, true);
    }
  }

  function startNotificationPolling() {
    if (notificationTimer) clearInterval(notificationTimer);
    notificationTimer = setInterval(() => refreshNotifications({ quiet: true }), 30000);
  }

  function stopNotificationPolling() {
    if (notificationTimer) clearInterval(notificationTimer);
    notificationTimer = null;
    closeNotifications();
  }

  async function toggleNotifications() {
    const panel = $("#notification-panel");
    const button = $("#notification-button");
    if (!panel || !panel.hidden) { closeNotifications(); return; }
    panel.hidden = false;
    button?.setAttribute("aria-expanded", "true");
    await refreshNotifications({ quiet: true });
  }

  async function markNotificationRead(id) {
    const item = state.notifications.find((entry) => entry.id === id);
    if (item && !item.read_at) {
      await api(`/notifications/${encodeURIComponent(id)}/read`, { method: "POST" });
      item.read_at = new Date().toISOString();
      state.notificationUnread = Math.max(0, state.notificationUnread - 1);
      if (state.notificationFilters.unread) state.notifications = state.notifications.filter((entry) => entry.id !== id);
      renderNotificationPanel();
    }
  }

  async function markAllNotificationsRead() {
    await api("/notifications/read-all", { method: "POST" });
    state.notificationUnread = 0;
    state.notifications.forEach((item) => { item.read_at = item.read_at || new Date().toISOString(); });
    if (state.notificationFilters.unread) state.notifications = [];
    renderNotificationPanel();
    toast("notifications marked read");
  }

  async function openNotification(element) {
    await markNotificationRead(element.dataset.id);
    closeNotifications();
    const type = element.dataset.resourceType;
    const id = element.dataset.resourceId;
    if (type === "node" && id) return go("nodes", id);
    if (type === "user" && id) return go("users", id);
    if (type === "domain") return go("domains");
    return go("issues");
  }

  // Resellers get a deliberately narrow panel over their own users. Overview
  // remains available for the scoped onboarding checklist; subscriptions only
  // operate on the already-scoped user list.
  function applyRoleScope(role) {
    const allowed = role === "reseller" ? new Set(["overview", "users", "traffic", "live", "subscriptions"]) : null;
    $("#notification-center").hidden = Boolean(allowed);
    $$(".nav-item[data-route]").forEach((item) => {
      item.hidden = allowed ? !allowed.has(item.dataset.route) : false;
    });
    $$("[data-nav-category]").forEach((group) => {
      group.hidden = allowed ? !allowed.has(group.dataset.navCategory) : false;
    });
    if (allowed && !allowed.has(state.route)) {
      state.route = "overview";
      state.detailId = null;
      location.hash = "#overview";
    }
  }

  // apply white-label branding to the panel chrome (accent + brand name).
  function applyBranding(b) {
    if (!b) return;
    if (b.accent_color) document.documentElement.style.setProperty("--blue", b.accent_color);
    const name = b.brand_name || "honey";
    document.title = `${name} — panel`;
    const brandText = document.querySelector(".brand > span:not(.brand-mark)");
    if (brandText) brandText.textContent = name;
  }

  async function loadData({ quiet = false } = {}) {
    if (state.loading) return;
    state.loading = true;
    if (!quiet) view.innerHTML = '<div class="page-loading"><span></span><span></span><span></span></div>';
    try {
      const admin = await api("/auth/me");
      state.admin = admin;
      applyRoleScope(admin.role);
      $("#token-button").textContent = `${admin.username} ? ${admin.role}`;
      const reseller = admin.role === "reseller";
      const [nodes, users, domains, profiles, ngroups, issueReport, savedViews, settings, onboarding, branding, customRoles] = await Promise.all([
        reseller ? Promise.resolve([]) : api("/nodes").catch(() => []),
        api("/users").catch(() => []),
        reseller ? Promise.resolve([]) : api("/domains").catch(() => []),
        reseller ? Promise.resolve([]) : api("/routing-profiles").catch(() => []),
        api("/groups").catch(() => []),
        reseller ? Promise.resolve({ counts: { total: 0, critical: 0, warning: 0, info: 0 }, issues: [] }) : api("/issues").catch(() => ({ unavailable: true, counts: { total: 0, critical: 0, warning: 0, info: 0 }, issues: [] })),
        api("/saved-views").catch(() => []),
        admin.role === "owner" ? api("/settings").catch(() => null) : Promise.resolve(null),
        api("/onboarding"),
        api("/branding").catch(() => null),
        admin.role === "owner" ? api("/custom-roles").catch(() => []) : Promise.resolve([]),
      ]);
      const groups = reseller ? [] : await Promise.all(nodes.map((node) =>
        api(`/nodes/${encodeURIComponent(node.id)}/inbounds`).catch(() => [])
      ));
      state.nodes = nodes;
      state.users = users;
      state.inbounds = groups.flat();
      state.domains = domains;
      state.profiles = profiles;
      state.groups = ngroups;
      state.issueReport = issueReport;
      state.savedViews = savedViews;
      state.onboarding = onboarding;
      if (settings) state.settings = settings;
      if (branding) { state.branding = branding; applyBranding(branding); }
      state.customRoles = customRoles || [];
      state.loaded = true;
      document.body.classList.remove("auth-locked");
      updateCounts();
      buildCommandEntries();
      render();
      if (reseller) {
        stopNotificationPolling();
      } else {
        startNotificationPolling();
        await refreshNotifications({ quiet: true });
      }
    } catch (error) {
      if (error.auth) {
        stopNotificationPolling();
        $("#notification-center").hidden = true;
        renderLocked();
      } else {
        toast(error.message, true);
        renderError(error.message);
      }
    } finally {
      state.loading = false;
    }
  }

  function updateCounts() {
    $("#nav-nodes").textContent = state.nodes.length;
    $("#nav-users").textContent = state.users.length;
    $("#nav-inbounds").textContent = state.inbounds.length;
    const actionable = Number(state.issueReport?.counts?.critical || 0) + Number(state.issueReport?.counts?.warning || 0);
    $("#nav-issues").textContent = state.issueReport?.unavailable ? "!" : actionable;
    $("#nav-issues").classList.toggle("hot", actionable > 0 || Boolean(state.issueReport?.unavailable));
  }

  // scoped drill-in: swap the main sidebar for an entity-scoped nav on entry.
  function detailEntity(kind, id) {
    if (kind === "nodes") return state.nodes.find((n) => n.id === id);
    if (kind === "users") return state.users.find((u) => u.id === id);
    if (kind === "inbounds") return state.inbounds.find((i) => i.id === id);
    return null;
  }
  function scopedMeta(kind, e) {
    if (kind === "nodes") {
      const online = isNodeOnline(e);
      return { title: e.name, sub: e.address, cls: online ? "ok" : e.enabled ? "warn" : "bad" };
    }
    if (kind === "users") return { title: e.username, sub: e.uuid, cls: e.active ? "ok" : "bad" };
    return { title: e.tag, sub: `${e.kind} · :${e.listen_port}`, cls: e.enabled ? "ok" : "bad" };
  }
  function scopedNavHtml(kind, e) {
    const { title, sub, cls } = scopedMeta(kind, e);
    const items = sectionsFor(kind).map((s) => `<button class="nav-item ${state.detailSection === s.key ? "active" : ""}" data-scope="${s.key}"><svg><use href="#i-${s.icon}"/></svg><span>${esc(t(s.label))}</span></button>`).join("");
    return `<button class="nav-item scoped-back" data-open="${kind}"><svg><use href="#i-back"/></svg><span>Back</span></button>
      <div class="scoped-head"><span class="status ${cls}"></span><div><b>${esc(title)}</b><small>${esc(sub)}</small></div></div>
      ${items}`;
  }
  function updateSidebarScope() {
    const mainNav = document.querySelector(".sidebar .nav");
    if (!mainNav) return;
    let scopedNav = document.getElementById("scoped-nav");
    const entity = detailKinds.has(state.route) && state.detailId ? detailEntity(state.route, state.detailId) : null;
    if (entity) {
      if (!scopedNav) {
        scopedNav = document.createElement("nav");
        scopedNav.id = "scoped-nav";
        scopedNav.className = "nav scoped-nav";
        scopedNav.setAttribute("aria-label", "Scoped navigation");
        mainNav.after(scopedNav);
      }
      scopedNav.innerHTML = scopedNavHtml(state.route, entity);
      mainNav.hidden = true;
      scopedNav.hidden = false;
    } else {
      mainNav.hidden = false;
      if (scopedNav) scopedNav.remove();
    }
  }
  // shared shell for a scoped detail: back + header + section tabs + body.
  function scopedShell(kind, title, statusHtml, sub, actions, body) {
    const section = state.detailSection || "overview";
    const tabs = sectionsFor(kind).map((s) => `<button class="scoped-tab ${section === s.key ? "active" : ""}" data-scope="${s.key}">${esc(t(s.label))}</button>`).join("");
    return `<div class="page">${backLink(kind, "All " + kind)}${detailHead(title, statusHtml, sub, actions)}<div class="scoped-tabs">${tabs}</div>${body}</div>`;
  }

  function render() {
    if (!state.loaded) return;
    const pages = {
      overview: renderOverview, issues: renderIssues, nodes: renderNodes, users: renderUsers,
      inbounds: renderInbounds, groups: renderGroups, traffic: renderTraffic, live: renderLive,
      subscriptions: renderSubscriptions, domains: renderDomains, ssltls: renderSslTls, rules: renderRules,
      logs: renderLogs, settings: renderSettings,
      "issue-history": renderIssueHistory,
      "node-health": renderNodeHealth, "node-versions": renderNodeVersions, "node-enrollment": renderNodeEnrollment,
      "inbound-protocols": renderInboundProtocols, "inbound-security": renderInboundSecurity,
      "user-access": renderUserAccess, "user-quotas": renderUserQuotas, "user-lifecycle": renderUserLifecycle,
      "group-coverage": renderGroupCoverage, "group-policies": renderGroupPolicies,
      "traffic-quotas": renderTrafficQuotas, "traffic-geography": renderTrafficGeography,
      "subscription-formats": renderSubscriptionFormats, "subscription-delivery": renderSubscriptionDelivery,
      "domain-dns": renderDomainDns, "domain-certificates": renderDomainCertificates,
      "tls-certificates": renderTlsCertificates, "tls-acme": renderTlsAcme, "tls-reality": renderTlsReality,
      "rule-coverage": renderRuleCoverage, "rule-delivery": renderRuleDelivery,
      "audit-page": renderAuditPage, "scheduled-page": renderScheduledPage,
      "new-inbound": renderNewInbound, "new-node": renderNewNode, "new-user": renderNewUser,
    };
    updateMainNavState();
    updateSidebarScope();
    if (state.detailId && detailKinds.has(state.route)) {
      view.innerHTML = renderDetail(state.route, state.detailId);
      if (state.route === "nodes") {
        const section = state.detailSection || "overview";
        if (section === "logs") loadNodeLogs(state.detailId);
        else if (section === "certs") loadNodeCerts(state.detailId);
        else if (section === "pushes") loadNodePushes(state.detailId);
      } else if (state.route === "inbounds" && (state.detailSection || "overview") === "security") {
        loadReachHistory(state.detailId);
      }
    } else {
      const activeView = categoryView(state.route, state.categorySection);
      view.innerHTML = (pages[activeView] || renderOverview)()
        .replace("ACME automation (sing-box)", "ACME automation")
        .replace("xray inbounds still use manual paths.", "Xray can use Honey-managed HTTP-01 certificates.");
      if (activeView === "logs" || activeView === "issue-history") loadLogs();
      if (activeView === "traffic-geography" && !state.geo.data && !state.geo.loading) loadGeo();
    }
    enhanceLabelColumn();
    bindTableFilter();
  }

  function enhanceLabelColumn() {
    if (!state.detailId && (state.route === "users" || state.route === "inbounds")) {
      const table = $(".table-shell table", view);
      if (!table) return;
      const insertAt = 4; // +1 for the leading multi-select checkbox column
      const heading = document.createElement("th");
      heading.textContent = "Labels";
      table.tHead.rows[0].insertBefore(heading, table.tHead.rows[0].cells[insertAt]);
      const emptyCell = table.tBodies[0]?.querySelector("td[colspan]");
      if (emptyCell) emptyCell.colSpan = Number(emptyCell.colSpan) + 1;
      $$(`tbody tr[data-search]`, table).forEach((row) => {
        const id = row.querySelector("[data-id]")?.dataset.id;
        const entity = state.route === "users" ? state.users.find((item) => item.id === id) : state.inbounds.find((item) => item.id === id);
        const cell = document.createElement("td");
        cell.innerHTML = labelChips(entity?.labels);
        row.insertBefore(cell, row.cells[insertAt]);
        row.dataset.labels = labelsOf(entity).join("|");
      });
    }
  }

  function backLink(route, label) {
    return `<button class="back-link" data-open="${esc(route)}">${icon("back")}<span>${esc(label)}</span></button>`;
  }
  function detailHead(title, statusHtml, sub, actions) {
    return `<div class="detail-head"><div><h1>${esc(title)}${statusHtml || ""}</h1><p class="sub">${sub || ""}</p></div><div class="detail-actions">${actions || ""}</div></div>`;
  }
  function prop(label, valueHtml) {
    return `<div class="prop-row"><span>${esc(label)}</span><div>${valueHtml}</div></div>`;
  }
  function railAction(action, id, iconName, label, extra = "") {
    return `<button class="link" data-action="${esc(action)}" data-id="${esc(id)}" ${extra}>${icon(iconName)}${esc(label)}</button>`;
  }
  function statusPill(cls, label) {
    return `<span class="status ${cls}" style="font-size:12px">${esc(label)}</span>`;
  }
  function reachBadge(inbound) {
    if (inbound.reachable === true) return '<span class="status ok">reachable</span>';
    if (inbound.reachable === false) return `<span class="status bad" title="${esc(inbound.reach_error || "")}">unreachable</span>`;
    if (["hysteria2", "tuic"].includes(inbound.kind)) {
      return '<span class="status warn" title="UDP/QUIC cannot be verified with the master TCP probe. Use an external vantage checker.">UDP · external probe</span>';
    }
    return '<span class="status warn" title="No reachability result has been recorded yet. Run a probe or wait for the next refresh.">not checked</span>';
  }
  async function probeInbound(id) {
    try {
      toast("probing…");
      const updated = await api(`/inbounds/${id}/reach`, { method: "POST" });
      const i = state.inbounds.findIndex((x) => x.id === id);
      if (i >= 0) state.inbounds[i] = updated;
      await loadData({ quiet: true });
      toast(updated.reachable === true ? "endpoint reachable" : updated.reachable === false ? "endpoint unreachable" : "UDP/QUIC needs an external reachability report");
    } catch (error) { toast(error.message, true); }
  }

  function renderDetail(route, id) {
    if (route === "nodes") {
      const node = state.nodes.find((n) => n.id === id);
      return node ? renderNodeDetail(node) : detailMissing("nodes", "Node");
    }
    if (route === "users") {
      const user = state.users.find((u) => u.id === id);
      return user ? renderUserDetail(user) : detailMissing("users", "User");
    }
    const inbound = state.inbounds.find((i) => i.id === id);
    return inbound ? renderInboundDetail(inbound) : detailMissing("inbounds", "Inbound");
  }

  function detailMissing(route, label) {
    return `<div class="page narrow">${backLink(route, "Back")}<div class="panel empty"><div><div class="empty-icon">${icon("search")}</div><h3>${esc(label)} not found</h3><p>It may have been deleted or renamed.</p><button class="button primary" data-open="${route}">Back to ${route}</button></div></div></div>`;
  }

  function renderNodeDetail(node) {
    const online = isNodeOnline(node);
    const status = node.enabled && node.maintenance
      ? statusPill("warn", "maintenance")
      : statusPill(online ? "ok" : node.enabled ? "warn" : "bad", node.enabled ? (online ? "online" : "not seen") : "disabled");
    const section = state.detailSection || "overview";
    const actions = `<button class="button primary" data-action="push-node" data-id="${node.id}">${icon("refresh")} Push</button><button class="button" data-action="edit-node" data-id="${node.id}">Edit</button>`;
    const body = section === "inbounds" ? nodeInboundsSection(node)
      : section === "certs" ? nodeCertsSection(node)
      : section === "pushes" ? nodePushesSection(node)
      : section === "logs" ? nodeLogsSection(node)
      : nodeOverviewSection(node, status);
    return scopedShell("nodes", node.name, status, `${esc(node.address)}:${esc(node.grpc_port)}`, actions, body);
  }

  function nodeOverviewSection(node, status) {
    const inbounds = state.inbounds.filter((i) => i.node_id === node.id);
    return `<div class="detail-grid"><div class="detail-main">
        <div class="panel"><div class="panel-title">Configuration</div><div class="prop-list">
          ${prop("Status", status)}
          ${prop("Address", `<span class="mono">${esc(node.address)}</span>`)}
          ${prop("gRPC port", `<span class="mono">${esc(node.grpc_port)}</span>`)}
          ${prop("Transport", `<span class="chip">${esc(node.transport)}</span>`)}
          ${prop("Labels", `${labelChips(node.labels)} <button class="row-button" data-action="edit-labels" data-kind="node" data-id="${node.id}">edit</button>`)}
          ${prop("TLS server name", `<span class="mono">${esc(node.tls_server_name || "—")}</span>`)}
          ${prop("Agent version", esc(node.agent_version || "—"))}
          ${prop("sing-box", esc(node.singbox_version || "—"))}
          ${prop("xray", esc(node.xray_version || "—"))}
          ${prop("Last seen", esc(relativeTime(node.last_seen)))}
          ${prop("Config", driftBadge(node))}
        </div></div>
        ${nodeMetricsPanel(node)}
      </div><aside class="rail">
        <div class="rail-section"><h3>Quick actions</h3><div class="rail-actions">
          ${railAction("push-node", node.id, "refresh", "Review & push desired state")}
          ${railAction("dry-run-node", node.id, "check", "Validate without applying")}
          ${railAction("preflight-node", node.id, "check", "Preflight (probe ports)")}
          ${railAction("benchmark-node", node.id, "chart", "Speed test (control path)")}
          ${railAction("enroll-node", node.id, "key", "Issue enrollment token")}
          ${railAction("manage-wg", node.id, "link", "WireGuard / AmneziaWG")}
          ${railAction("manage-services", node.id, "link", "Services (MTProto / Naive)")}
          ${railAction("node-groups", node.id, "groups", "Manage groups")}
          ${railAction("edit-node", node.id, "settings", "Edit node")}
          ${railAction("edit-labels", node.id, "settings", "Edit labels", 'data-kind="node"')}
          ${railAction("schedule-op", node.id, "chart", "Schedule operation…", 'data-kind="node"')}
          ${railAction("entity-history", node.id, "logs", "Change history", 'data-kind="node"')}
          ${railAction("toggle-maintenance", node.id, "refresh", node.maintenance ? "End maintenance (serve)" : "Maintenance (drain)", `data-maint="${node.maintenance}"`)}
          ${railAction("toggle-node", node.id, "refresh", node.enabled ? "Disable" : "Enable", `data-enabled="${node.enabled}"`)}
          <button class="link danger" data-action="delete-node" data-id="${node.id}">${icon("x")}Delete node</button>
        </div></div>
        <div class="rail-section"><h3>Details</h3><div class="rail-kv">
          <div><span>Inbounds</span><b>${inbounds.length}</b></div>
          <div><span>Enabled</span><b>${node.enabled ? "yes" : "no"}</b></div>
          <div><span>Maintenance</span><b>${node.maintenance ? "draining" : "no"}</b></div>
          <div><span>Monthly cost</span><b>${node.monthly_cost_cents ? "$" + (Number(node.monthly_cost_cents) / 100).toFixed(2) : "—"}</b></div>
          <div><span>Created</span><b>${esc(dateLabel(node.created_at))}</b></div>
        </div></div>
      </aside></div>`;
  }

  function nodeInboundsSection(node) {
    const inbounds = state.inbounds.filter((i) => i.node_id === node.id);
    const rows = inbounds.length
      ? inbounds.map((i) => `<div class="list-row" style="padding:0 14px"><div class="list-row-icon">${icon("inbound")}</div><div class="list-row-main"><b><button class="cell-link" data-open="inbounds/${i.id}">${esc(i.tag)}</button></b><small>${esc(i.kind)} · ${esc(i.core)} · :${esc(i.listen_port)} · ${i.enabled ? "enabled" : "disabled"}</small></div><button class="link" data-open="inbounds/${i.id}">open ${icon("chevron")}</button></div>`).join("")
      : `<div class="panel-body"><p class="form-note">No inbounds on this node yet.</p></div>`;
    return `<div class="panel"><div class="panel-title">Inbounds <span class="chip">${inbounds.length}</span><span class="toolbar-spacer"></span><button class="button" data-action="add-inbound">${icon("plus")} Add inbound</button></div>${rows}</div>`;
  }

  function nodeCertsSection(node) {
    return `<div class="panel"><div class="panel-title">Certificates</div><div id="node-certs-body" class="log-wrap"><div class="page-loading" style="min-height:110px"><span></span><span></span><span></span></div></div></div>
      <div class="rail-inline"><button class="button" data-action="enroll-node" data-id="${node.id}">${icon("key")} Issue enrollment token</button></div>`;
  }

  function nodePushesSection(node) {
    return `<div class="panel"><div class="panel-title">Push history</div><div id="node-pushes-body" class="log-wrap"><div class="page-loading" style="min-height:110px"><span></span><span></span><span></span></div></div></div>
      <div class="rail-inline"><button class="button primary" data-action="push-node" data-id="${node.id}">${icon("refresh")} Review & push</button> <button class="button" data-action="dry-run-node" data-id="${node.id}">${icon("check")} Dry-run</button></div>`;
  }

  function nodeLogsSection(node) {
    return `<div class="panel"><div class="panel-title">Activity <span class="chip" id="node-log-count">·</span></div><div id="node-logs" class="log-wrap"><div class="page-loading" style="min-height:110px"><span></span><span></span><span></span></div></div></div>`;
  }

  async function loadNodeCerts(nodeId) {
    const el = document.getElementById("node-certs-body");
    if (!el) return;
    try {
      const rows = await api(`/nodes/${nodeId}/certificates`);
      el.innerHTML = rows.length
        ? `<div class="check-list">${rows.map((cert) => `<div class="check-row"><span><b>${esc(cert.serial_number)}</b><small>expires ${dateLabel(cert.not_after)} · ${cert.revoked_at ? "revoked" : "active"} · ${esc(cert.fingerprint_sha256)}</small></span></div>`).join("")}</div>`
        : '<div class="empty"><div><h3>No issued certificates</h3><p>Use enroll to issue the first node identity.</p></div></div>';
    } catch (error) { el.innerHTML = `<p class="form-note">${esc(error.message)}</p>`; }
  }

  async function loadNodePushes(nodeId) {
    const el = document.getElementById("node-pushes-body");
    if (!el) return;
    try {
      const rows = await api(`/nodes/${nodeId}/pushes`);
      el.innerHTML = rows.length
        ? `<div class="check-list">${rows.map((push) => `<div class="check-row"><span><b>${esc(push.status)} · ${esc(push.source)}</b><small>${dateLabel(push.started_at)} · ${esc(push.desired_hash.slice(0, 16))}…${push.message ? " · " + esc(push.message) : ""}</small></span></div>`).join("")}</div>`
        : '<div class="empty"><div><h3>No pushes recorded</h3></div></div>';
    } catch (error) { el.innerHTML = `<p class="form-note">${esc(error.message)}</p>`; }
  }

  function renderUserDetail(user) {
    const status = statusPill(user.active ? "ok" : "bad", user.active ? "active" : (user.suppressed_reason || "offline"));
    const section = state.detailSection || "overview";
    const actions = `<button class="button primary" data-action="reveal-sub" data-id="${user.id}">${icon("link")} Subscription</button><button class="button" data-action="preview-sub" data-id="${user.id}">Preview</button><button class="button" data-action="edit-user" data-id="${user.id}">Edit</button>`;
    const body = section === "access" ? userAccessSection(user)
      : section === "subscription" ? userSubscriptionSection(user)
      : userOverviewSection(user, status);
    return scopedShell("users", user.username, status, `uuid ${esc(user.uuid)}`, actions, body);
  }

  function userOverviewSection(user, status) {
    const used = Number(user.used_traffic_bytes || 0), limit = Number(user.traffic_limit_bytes || 0);
    const ratio = limit > 0 ? Math.min(100, used / limit * 100) : 0;
    return `<div class="detail-grid"><div class="detail-main">
        <div class="panel"><div class="panel-title">Account</div><div class="prop-list">
          ${prop("Status", status)}
          ${prop("UUID", `<span class="mono">${esc(user.uuid)}</span>`)}
          ${prop("Labels", `${labelChips(user.labels)} <button class="row-button" data-action="edit-labels" data-kind="user" data-id="${user.id}">edit</button>`)}
          ${prop("Traffic", `<div class="usage-cell" style="min-width:220px"><div class="progress"><i style="width:${ratio}%"></i></div><span>${bytes(used)} / ${limit ? bytes(limit) : "∞"}</span></div>`)}
          ${prop("Quota", limit ? Math.round(ratio) + "%" : "unlimited")}
          ${prop("Device limit", Number(user.device_limit || 0) > 0 ? `${user.device_limit} device${user.device_limit === 1 ? "" : "s"} <span class="muted">(distinct IPs)</span>` : "unlimited")}
          ${prop("Expires", esc(dateLabel(user.expires_at)))}
          ${prop("Created", esc(dateLabel(user.created_at)))}
        </div></div>
      </div><aside class="rail">
        <div class="rail-section"><h3>Quick actions</h3><div class="rail-actions">
          ${railAction("edit-user", user.id, "settings", "Edit user")}
          ${railAction("edit-labels", user.id, "settings", "Edit labels", 'data-kind="user"')}
          ${railAction("rotate-credentials", user.id, "key", "Rotate uuid & password")}
          ${railAction("reset-traffic", user.id, "refresh", "Reset traffic")}
          ${railAction("schedule-op", user.id, "chart", "Schedule operation…", 'data-kind="user"')}
          ${railAction("entity-history", user.id, "logs", "Change history", 'data-kind="user"')}
          ${railAction("toggle-user", user.id, "refresh", user.enabled ? "Disable" : "Enable", `data-enabled="${user.enabled}"`)}
          ${railAction("gdpr-export", user.id, "logs", "GDPR data export")}
          <button class="link danger" data-action="delete-user" data-id="${user.id}">${icon("x")}Delete user</button>
          <button class="link danger" data-action="gdpr-erase" data-id="${user.id}">${icon("x")}GDPR erase (forget)</button>
        </div></div>
        <div class="rail-section"><h3>Details</h3><div class="rail-kv">
          <div><span>Enabled</span><b>${user.enabled ? "yes" : "no"}</b></div>
          <div><span>Active</span><b>${user.active ? "yes" : "no"}</b></div>
          <div><span>Created</span><b>${esc(dateLabel(user.created_at))}</b></div>
        </div></div>
      </aside></div>`;
  }

  function userAccessSection(user) {
    return `<div class="detail-main">
        <div class="panel"><div class="panel-title">Group access</div><div class="panel-body">
          <p class="form-note">This user reaches ungrouped (universal) nodes plus nodes sharing any of its groups.</p>
          <button class="button" data-action="user-groups" data-id="${user.id}">${icon("groups")} Manage groups</button>
        </div></div>
        <div class="panel"><div class="panel-title">Routing profile</div><div class="panel-body">
          <select class="rail-select" data-user-profile="${user.id}">
            <option value="" ${!user.routing_profile_id ? "selected" : ""}>Default profile</option>
            ${state.profiles.map((p) => `<option value="${p.id}" ${user.routing_profile_id === p.id ? "selected" : ""}>${esc(p.name)}${p.is_default ? " (default)" : ""}</option>`).join("")}
          </select>
          <p class="form-note" style="margin-top:8px">Routing rules ship inside this user's subscription.</p>
        </div></div>
        <div class="panel"><div class="panel-title">Quota window</div><div class="panel-body">
          <select class="rail-select" data-user-quota="${user.id}">
            ${["none", "daily", "weekly"].map((v) => `<option value="${v}" ${(user.quota_interval || "none") === v ? "selected" : ""}>${v === "none" ? "lifetime (no reset)" : v}</option>`).join("")}
          </select>
          <p class="form-note" style="margin-top:8px">Reset the traffic limit every day / week.</p>
        </div></div>
      </div>`;
  }

  function userSubscriptionSection(user) {
    const alias = user.subscription_alias;
    const uni = alias ? `${location.origin}/s/${alias}` : "";
    const permanent = `${location.origin}/sub/${user.id}`;
    return `<div class="detail-main">
        <div class="panel"><div class="panel-title">Subscription</div><div class="prop-list">
          ${prop("State", `<span class="status ${user.active ? "ok" : "bad"}">${user.active ? "available" : "unavailable"}</span>`)}
          ${prop("Permanent link", `<span class="mono">${esc(permanent)}</span> <button class="row-button" data-copy="${esc(permanent)}">copy</button>`)}
          ${prop("Revocable token", "optional · stored encrypted at rest")}
          ${prop("Client title", esc(user.subscription_title || user.username))}
          ${prop("Client group", esc(user.subscription_group || "global default"))}
          ${prop("Traffic row", esc(user.subscription_traffic_policy || "inherit"))}
          ${prop("Short alias", alias ? `<span class="mono">${esc(alias)}</span>` : "—")}
          ${alias ? prop("Universal link", `<span class="mono">${esc(uni)}</span> <button class="row-button" data-copy="${esc(uni)}">copy</button>`) : ""}
          ${prop("Suppression", esc(user.suppressed_reason || "none"))}
        </div><div class="panel-body"><div class="rail-actions">
          ${railAction("reveal-sub", user.id, "link", "Show permanent link")}
          ${railAction("preview-sub", user.id, "check", "Preview client profiles")}
          ${railAction("manage-subs", user.id, "link", "Named links (per-device)")}
          ${railAction("set-alias", user.id, "settings", alias ? "Change short alias" : "Set short alias")}
          ${railAction("rotate-sub", user.id, "refresh", "Create/rotate optional revocable link")}
          ${railAction("rotate-credentials", user.id, "key", "Rotate protocol credentials (optional)")}
        </div><p class="form-note" style="margin-top:10px">The permanent UUID link remains valid across credential and optional token rotations. Client-specific links apply compatibility presets without changing the server inbound.</p></div></div>
      </div>`;
  }
  async function manageServices(nodeId) {
    let svcs = [];
    try { svcs = await api(`/nodes/${nodeId}/services`); } catch (error) { toast(error.message, true); return; }
    const rows = svcs.length
      ? svcs.map((s) => `<div class="check-row"><span><b>${esc(s.name)}</b> <span class="chip blue">${esc(s.kind)}</span><small>:${esc(s.listen_port)}${s.enabled ? "" : " · disabled"}</small></span><span class="row-actions"><button class="row-button" data-action="toggle-service" data-id="${nodeId}" data-svcid="${s.id}" data-enabled="${s.enabled}">${s.enabled ? "disable" : "enable"}</button><button class="row-button danger" data-action="delete-service" data-id="${nodeId}" data-svcid="${s.id}">delete</button></span></div>`).join("")
      : `<p class="form-note">No managed services on this node. MTProto (Telegram proxy via mtg) and NaiveProxy (Caddy) run as their own daemons; users get a client link on their subscription page.</p>`;
    showList("Services (MTProto / Naive)", `<div class="form-body">
      <div class="check-list">${rows}</div>
      <div class="section-label">ADD SERVICE</div>
      <div class="form-row"><label><span>Type</span><select id="svc-kind" data-svc-kind><option value="mtproto">MTProto (Telegram)</option><option value="naive">NaiveProxy</option></select></label><label><span>Name</span><input id="svc-name" placeholder="mtproto-1" autocomplete="off"></label></div>
      <div class="form-row"><label><span>Listen port</span><input id="svc-port" type="number" min="1" max="65535" value="443"></label><label id="svc-mt-host"><span>Fake-TLS host</span><input id="svc-host" value="www.cloudflare.com" autocomplete="off"></label></div>
      <div id="svc-mt-fields">
        <div class="form-row"><label>${helpLabel("Max connections", "mtg 'concurrency': global cap on simultaneous connections for this proxy. 0 = mtg default (~8192). This is a global limit, not per-user (MTProto has no user accounts).")}<input id="svc-mt-conc" type="number" min="0" max="1000000" value="0" placeholder="0 = default"></label><label>${helpLabel("IP preference", "mtg 'prefer-ip': which address family to use when connecting upstream to Telegram.")}<select id="svc-mt-ip"><option value="">default</option><option value="prefer-ipv6">prefer IPv6</option><option value="prefer-ipv4">prefer IPv4</option><option value="only-ipv4">only IPv4</option><option value="only-ipv6">only IPv6</option></select></label></div>
        <div class="form-row"><label>${helpLabel("Domain-fronting port", "mtg 'domain-fronting-port': port used for the fake-TLS domain-fronting handshake. 0 = mtg default (443).")}<input id="svc-mt-dfport" type="number" min="0" max="65535" value="0" placeholder="0 = 443"></label><label><span>Anti-replay protection</span><select id="svc-mt-replay"><option value="false">off</option><option value="true">on</option></select></label></div>
      </div>
      <div class="form-row" id="svc-naive-fields" hidden><label><span>Username</span><input id="svc-user" value="user" autocomplete="off"></label><label><span>TLS domain</span><input id="svc-domain" placeholder="proxy.example.com" autocomplete="off"></label></div>
      <button class="button primary" data-action="create-service" data-id="${nodeId}">Create service</button>
      <p class="field-error" id="svc-err"></p>
      <p class="form-note">Requires the daemon on the node: <code>mtg</code> for MTProto, <code>caddy</code> (with forwardproxy) for NaiveProxy. Secret/password is generated and stored encrypted; the agent runs the daemon best-effort.</p>
    </div>`);
    const kindSel = document.getElementById("svc-kind");
    const toggle = () => {
      const mt = kindSel.value === "mtproto";
      const h = document.getElementById("svc-mt-host"); if (h) h.hidden = !mt;
      const f = document.getElementById("svc-mt-fields"); if (f) f.hidden = !mt;
      const n = document.getElementById("svc-naive-fields"); if (n) n.hidden = mt;
    };
    if (kindSel) { kindSel.addEventListener("change", toggle); toggle(); }
  }
  async function manageWg(nodeId) {
    let ifaces = [];
    try { ifaces = await api(`/nodes/${nodeId}/wireguard`); } catch (error) { toast(error.message, true); return; }
    const rows = ifaces.length
      ? ifaces.map((w) => `<div class="check-row"><span><b>${esc(w.name)}</b> ${w.amnezia ? '<span class="chip blue">AmneziaWG</span>' : '<span class="chip">WireGuard</span>'}<small>:${esc(w.listen_port)} · ${esc(w.address_cidr)} · MTU ${esc(w.mtu)}${w.enabled ? "" : " · disabled"}</small></span><span class="row-actions"><button class="row-button" data-action="toggle-wg" data-id="${nodeId}" data-wgid="${w.id}" data-enabled="${w.enabled}">${w.enabled ? "disable" : "enable"}</button><button class="row-button danger" data-action="delete-wg" data-id="${nodeId}" data-wgid="${w.id}">delete</button></span></div>`).join("")
      : `<p class="form-note">No WireGuard interfaces on this node yet. Each interface runs a wg/awg server; every user with access to the node gets a peer automatically.</p>`;
    showList("WireGuard / AmneziaWG", `<div class="form-body">
      <div class="check-list">${rows}</div>
      <div class="section-label">ADD INTERFACE</div>
      <div class="form-row"><label><span>Name</span><input id="wg-name" placeholder="wg0" autocomplete="off"></label><label><span>Listen port (UDP)</span><input id="wg-port" type="number" min="1" max="65535" value="51820"></label></div>
      <div class="form-row"><label><span>Address pool (CIDR)</span><input id="wg-cidr" value="10.7.0.0/24" autocomplete="off"></label><label><span>Type</span><select id="wg-amnezia"><option value="false">WireGuard</option><option value="true">AmneziaWG (obfuscated)</option></select></label></div>
      <div class="form-row"><label><span>DNS</span><input id="wg-dns" value="1.1.1.1" autocomplete="off"></label><label><span>MTU</span><input id="wg-mtu" type="number" min="1280" max="1500" value="1420"></label></div>
      <div class="form-row"><label><span>Endpoint host (optional)</span><input id="wg-endpoint" placeholder="overrides node address" autocomplete="off"></label><label><span>&nbsp;</span><span class="form-note" style="padding-top:9px">clients connect to this:port</span></label></div>
      <button class="button primary" data-action="create-wg" data-id="${nodeId}">Create interface</button>
      <p class="field-error" id="wg-err"></p>
      <p class="form-note">Requires wireguard-tools (or amneziawg-tools) on the node. The agent brings the interface up and NATs the pool; clients get a <code>.conf</code> on their subscription page.</p>
    </div>`);
  }
  async function manageNamedSubs(id) {
    let subs = [];
    try { subs = await api(`/users/${id}/subscriptions`); } catch (error) { toast(error.message, true); return; }
    const rows = subs.length
      ? subs.map((s) => `<div class="check-row"><span><b>${esc(s.name)}</b><small>created ${dateLabel(s.created_at)}</small></span><span class="row-actions"><button class="row-button" data-action="reveal-named-sub" data-id="${id}" data-sid="${s.id}">reveal</button><button class="row-button danger" data-action="delete-named-sub" data-id="${id}" data-sid="${s.id}">revoke</button></span></div>`).join("")
      : `<p class="form-note">No named links yet. Each named link is a separate, independently revocable token for the same user config — one per device (work laptop, phone, home router).</p>`;
    showList("Named subscription links", `<div class="form-body">
      <div class="check-list">${rows}</div>
      <div class="form-row" style="margin-top:12px"><label style="flex:1"><span>New link name</span><input id="named-sub-input" placeholder="phone" autocomplete="off" spellcheck="false" maxlength="25"></label></div>
      <button class="button primary" data-action="create-named-sub" data-id="${id}">Create link</button>
      <p class="field-error" id="named-sub-err"></p>
      <p class="form-note">Revoking a named link invalidates only that token; other links and the primary subscription keep working.</p>
    </div>`);
  }
  async function previewSubscription(id) {
    const clients = ["happ-android", "happ-desktop", "karing", "generic"];
    try {
      const previews = await Promise.all(clients.map((client) => api(`/users/${id}/subscription-preview?client=${encodeURIComponent(client)}`)));
      const cards = previews.map((preview) => {
        const url = location.origin + preview.profile_path;
        const endpoints = (preview.endpoints || []).map((endpoint) =>
          `<div class="check-row"><span><b>${esc(endpoint.name)}</b><small>${esc(endpoint.protocol)} · ${esc(endpoint.network || "tcp")}${endpoint.xhttp_mode ? ` · ${esc(endpoint.xhttp_mode)}` : ""}${endpoint.fingerprint ? ` · fp ${esc(endpoint.fingerprint)}` : ""}${endpoint.warning ? ` · ${esc(endpoint.warning)}` : ""}</small></span></div>`
        ).join("");
        return `<div class="panel" style="margin-bottom:12px"><div class="panel-title">${esc(preview.client)}</div><div class="panel-body">
          <div class="prop-list">
            ${prop("Title", esc(preview.title || ""))}
            ${prop("Group", esc(preview.group || "—"))}
            ${prop("Traffic header", preview.traffic_header ? "shown" : "hidden")}
            ${prop("Update interval", `${preview.profile_update_interval_hours} h`)}
            ${prop("Profile URL", `<span class="mono">${esc(url)}</span> <button class="row-button" data-copy="${esc(url)}">copy</button>`)}
          </div>
          <div class="check-list" style="margin-top:10px">${endpoints || '<p class="form-note">No accessible endpoints.</p>'}</div>
        </div></div>`;
      }).join("");
      showList("Subscription preview", `<div class="form-body">${cards}<p class="form-note">Preview applies client-side compatibility overrides only; server inbounds are unchanged.</p></div>`);
    } catch (error) { toast(error.message, true); }
  }
  function manageAlias(id) {
    const user = state.users.find((u) => u.id === id);
    const cur = user?.subscription_alias || "";
    showList("Subscription alias", `<div class="form-body">
      <p class="form-note">A short, memorable subscription path: <code>${esc(location.origin)}/s/&lt;alias&gt;</code>. 3–40 chars: letters, digits, - or _. Empty to remove.</p>
      <label><span>Alias</span><input id="alias-input" value="${esc(cur)}" placeholder="alice-home" autocomplete="off" spellcheck="false"></label>
      <button class="button primary" data-action="save-alias" data-id="${id}">Save</button>
      <p class="field-error" id="alias-err"></p>
    </div>`);
  }
  async function saveAlias(id) {
    const alias = document.getElementById("alias-input").value.trim();
    try {
      await api(`/users/${id}/alias`, { method: "PUT", body: JSON.stringify({ alias: alias || null }) });
      formDialog.close();
      toast(alias ? "alias saved" : "alias removed");
      await loadData({ quiet: true });
    } catch (error) { const el = document.getElementById("alias-err"); if (el) el.textContent = error.message; }
  }

  function inboundCert(inbound) {
    const source = inbound.certificate_source || (inbound.reality ? "reality" : (inbound.extra && inbound.extra.acme) ? "acme" : inbound.tls_enabled ? "manual" : "none");
    const statusVal = inbound.certificate_status || (source === "acme" ? "managed" : source === "manual" && inbound.cert_path && inbound.key_path ? "configured" : source === "manual" ? "missing" : "not_applicable");
    const line = source === "reality"
      ? `<span class="chip">REALITY · not applicable</span>`
      : source === "acme" ? `<span class="chip blue">ACME · managed</span>`
      : source === "manual" && statusVal === "configured" ? `<span class="chip blue">manual · configured</span> <span class="mono">${esc(inbound.cert_path)}</span>`
      : source === "manual" ? `<span class="status bad">manual · missing paths</span>`
      : "none";
    return { source, statusVal, line };
  }

  function renderInboundDetail(inbound) {
    const status = statusPill(inbound.enabled ? "ok" : "bad", inbound.enabled ? "enabled" : "disabled");
    const section = state.detailSection || "overview";
    const actions = `<button class="button" data-action="edit-inbound" data-id="${inbound.id}">Edit</button>`;
    const body = section === "security" ? inboundSecuritySection(inbound)
      : section === "transport" ? inboundTransportSection(inbound)
      : inboundOverviewSection(inbound, status);
    return scopedShell("inbounds", inbound.tag, status, `${esc(inbound.kind)} · ${esc(inbound.core)} · :${esc(inbound.listen_port)}`, actions, body);
  }

  function inboundOverviewSection(inbound, status) {
    const node = state.nodes.find((n) => n.id === inbound.node_id);
    const security = inbound.reality ? "reality" : inbound.tls_enabled ? "tls" : "none";
    const nodeLink = node ? `<button class="cell-link" data-open="nodes/${node.id}">${esc(node.name)}</button>` : "unknown";
    return `<div class="detail-grid"><div class="detail-main">
        <div class="panel"><div class="panel-title">Listener</div><div class="prop-list">
          ${prop("Status", status)}
          ${prop("Node", nodeLink)}
          ${prop("Protocol", `<span class="chip">${esc(inbound.kind)}</span>`)}
          ${prop("Core", esc(inbound.core))}
          ${prop("Labels", `${labelChips(inbound.labels)} <button class="row-button" data-action="edit-labels" data-kind="inbound" data-id="${inbound.id}">edit</button>`)}
          ${prop("Listen", `<span class="mono">${esc(inbound.listen)}:${esc(inbound.listen_port)}</span>`)}
          ${prop("Network", `<span class="chip">${esc(inbound.network || "tcp")}</span>`)}
          ${prop("Security", `<span class="chip ${security !== "none" ? "blue" : ""}">${security}</span>`)}
          ${prop("Reachability", reachBadge(inbound))}
          ${inbound.flow ? prop("Flow", `<span class="mono">${esc(inbound.flow)}</span>`) : ""}
          ${(Number(inbound.up_mbps || 0) > 0 || Number(inbound.down_mbps || 0) > 0) ? prop("Speed cap", `<span class="chip blue">↓ ${Number(inbound.down_mbps) || "∞"} / ↑ ${Number(inbound.up_mbps) || "∞"} Mbps</span>`) : ""}
          ${inbound.upstream_inbound_id ? prop("Multihop exit", (() => { const e = (state.inbounds || []).find((i) => i.id === inbound.upstream_inbound_id); const n = e ? ((state.nodes.find((x) => x.id === e.node_id) || {}).name || "node") : ""; return `<span class="chip blue">→ ${e ? esc(n + " · " + e.tag) : "unknown"}</span>`; })()) : ""}
          ${(inbound.cdn_pool || []).length ? prop("CDN pool", `<span class="chip">${(inbound.cdn_pool || []).length} host${inbound.cdn_pool.length === 1 ? "" : "s"} · auto-rotate by ping</span>`) : ""}
        </div></div>
      </div><aside class="rail">
        <div class="rail-section"><h3>Quick actions</h3><div class="rail-actions">
          ${railAction("probe-inbound", inbound.id, "refresh", "Probe reachability")}
          ${railAction("edit-inbound", inbound.id, "settings", "Edit inbound")}
          ${railAction("edit-labels", inbound.id, "settings", "Edit labels", 'data-kind="inbound"')}
          ${railAction("schedule-op", inbound.id, "chart", "Schedule operation…", 'data-kind="inbound"')}
          ${railAction("entity-history", inbound.id, "logs", "Change history", 'data-kind="inbound"')}
          ${node ? `<button class="link" data-open="nodes/${node.id}">${icon("node")}Open node</button>` : ""}
          <button class="link danger" data-action="delete-inbound" data-id="${inbound.id}">${icon("x")}Delete inbound</button>
        </div></div>
        <div class="rail-section"><h3>Details</h3><div class="rail-kv">
          <div><span>Enabled</span><b>${inbound.enabled ? "yes" : "no"}</b></div>
          <div><span>Reality</span><b>${inbound.reality ? "on" : "off"}</b></div>
          <div><span>TLS</span><b>${inbound.tls_enabled ? "on" : "off"}</b></div>
        </div></div>
      </aside></div>`;
  }

  function inboundSecuritySection(inbound) {
    const security = inbound.reality ? "reality" : inbound.tls_enabled ? "tls" : "none";
    const cert = inboundCert(inbound);
    return `<div class="detail-grid"><div class="detail-main">
        <div class="panel"><div class="panel-title">Security &amp; TLS</div><div class="prop-list">
          ${prop("Security", `<span class="chip ${security !== "none" ? "blue" : ""}">${security}</span>`)}
          ${prop("TLS", inbound.tls_enabled ? "on" : "off")}
          ${prop("Certificate", cert.line)}
          ${prop("Cert source", esc(cert.source))}
          ${prop("Cert status", esc(cert.statusVal))}
          ${inbound.server_name ? prop("Server name (SNI)", `<span class="mono">${esc(inbound.server_name)}</span>`) : ""}
          ${prop("REALITY", inbound.reality ? "on" : "off")}
          ${inbound.reality && inbound.reality_public_key ? prop("REALITY public key", `<span class="mono">${esc(inbound.reality_public_key)}</span>`) : ""}
          ${inbound.reality && inbound.reality_handshake_server ? prop("Handshake / target", `<span class="mono">${esc(inbound.reality_handshake_server)}${inbound.reality_handshake_port ? ":" + esc(inbound.reality_handshake_port) : ""}</span>`) : ""}
          ${inbound.reality && (inbound.reality_short_ids || []).length ? prop("Short IDs", `<span class="mono">${esc((inbound.reality_short_ids || []).join(", "))}</span>`) : ""}
          ${inbound.utls_fingerprint ? prop("uTLS fingerprint", esc(inbound.utls_fingerprint)) : ""}
          ${inbound.ech ? prop("ECH", "on") : ""}
          ${inbound.shadowtls_handshake_server ? prop("ShadowTLS handshake", `<span class="mono">${esc(inbound.shadowtls_handshake_server)}${inbound.shadowtls_handshake_port ? ":" + esc(inbound.shadowtls_handshake_port) : ""}</span>`) : ""}
        </div></div>
        <div class="panel"><div class="panel-title">RF-resilience</div><div class="prop-list">
          ${prop("Reachability", reachBadge(inbound))}
          ${prop("Fallback CDN host", inbound.fallback_host ? `<span class="mono">${esc(inbound.fallback_host)}</span>` : "—")}
          ${prop("SNI pool", (inbound.sni_pool || []).length ? `<span class="mono">${esc((inbound.sni_pool || []).join(", "))}</span> <button class="row-button" data-action="rotate-sni" data-id="${inbound.id}">rotate SNI</button>` : "—")}
        </div><div class="panel-title" style="border-top:1px solid var(--line)">Vantage reachability <span class="chip" id="reach-count">·</span></div><div id="reach-history" class="log-wrap"><div class="page-loading" style="min-height:90px"><span></span><span></span><span></span></div></div></div>
      </div><aside class="rail">
        <div class="rail-section"><h3>Actions</h3><div class="rail-actions">
          ${railAction("edit-inbound", inbound.id, "settings", "Edit inbound")}
          ${railAction("probe-inbound", inbound.id, "refresh", "Probe reachability")}
          ${(inbound.sni_pool || []).length ? railAction("rotate-sni", inbound.id, "refresh", "Rotate SNI") : ""}
        </div></div>
      </aside></div>`;
  }

  async function loadReachHistory(inboundId) {
    const el = document.getElementById("reach-history");
    if (!el) return;
    try {
      const rows = await api(`/inbounds/${inboundId}/reachability`);
      const cnt = document.getElementById("reach-count");
      if (cnt) cnt.textContent = rows.length;
      el.innerHTML = rows.length
        ? `<div class="check-list">${rows.map((r) => `<div class="check-row"><span class="status ${r.reachable ? "ok" : "bad"}"></span><span><b>${r.reachable ? "reachable" : "blocked"} · ${esc(r.source)}</b><small>${dateLabel(r.created_at)}${r.latency_ms != null ? " · " + r.latency_ms + "ms" : ""}${r.error ? " · " + esc(r.error) : ""}</small></span></div>`).join("")}</div>`
        : '<div class="empty"><div><h3>No vantage reports yet</h3><p>External checkers POST to <code>/inbounds/:id/reachability</code>.</p></div></div>';
    } catch (error) { el.innerHTML = `<p class="form-note">${esc(error.message)}</p>`; }
  }

  function inboundTransportSection(inbound) {
    const net = inbound.network || "tcp";
    return `<div class="detail-grid"><div class="detail-main">
        <div class="panel"><div class="panel-title">Transport</div><div class="prop-list">
          ${prop("Network", `<span class="chip">${esc(net)}</span>`)}
          ${net === "tcp" ? prop("Options", "raw tcp — no extra transport options") : ""}
          ${inbound.transport_path ? prop("Path", `<span class="mono">${esc(inbound.transport_path)}</span>`) : ""}
          ${inbound.transport_host ? prop("Host header", `<span class="mono">${esc(inbound.transport_host)}</span>`) : ""}
          ${inbound.transport_service_name ? prop("gRPC service", `<span class="mono">${esc(inbound.transport_service_name)}</span>`) : ""}
          ${inbound.transport_mode ? prop("xHTTP mode", esc(inbound.transport_mode)) : ""}
        </div></div>
      </div><aside class="rail">
        <div class="rail-section"><h3>Actions</h3><div class="rail-actions">
          ${railAction("edit-inbound", inbound.id, "settings", "Edit inbound")}
        </div></div>
      </aside></div>`;
  }

  function renderLogs() {
    return `<div class="page">${pageHeader("Logs", "Master runtime log and desired-state activity across every node.", refreshButton())}
      <div class="panel" style="margin-bottom:18px"><div class="panel-title">Master runtime <span class="chip" id="runtime-count">·</span></div><div class="panel-body">
        <div class="filter-row" style="margin-bottom:12px">
          <select class="rail-select" data-runtime-log-filter="level" aria-label="Filter runtime logs by level"><option value="">All levels</option><option value="error" ${state.runtimeLogFilters.level === "error" ? "selected" : ""}>error</option><option value="warn" ${state.runtimeLogFilters.level === "warn" ? "selected" : ""}>warn</option><option value="info" ${state.runtimeLogFilters.level === "info" ? "selected" : ""}>info</option><option value="debug" ${state.runtimeLogFilters.level === "debug" ? "selected" : ""}>debug</option><option value="trace" ${state.runtimeLogFilters.level === "trace" ? "selected" : ""}>trace</option></select>
          <input class="rail-select" data-runtime-log-filter="code" value="${esc(state.runtimeLogFilters.code)}" placeholder="Code, e.g. M0406" aria-label="Filter runtime logs by code">
          <input class="rail-select" data-runtime-log-filter="query" value="${esc(state.runtimeLogFilters.query)}" placeholder="Search message, target or request_id" aria-label="Search runtime logs">
        </div>
        <div id="runtime-body" class="log-wrap"><div class="page-loading" style="min-height:120px"><span></span><span></span><span></span></div></div>
      </div></div>
      <div class="panel"><div class="panel-title">System activity</div><div class="panel-body">
        <div class="filter-row" style="margin-bottom:12px">
          <select class="rail-select" data-log-filter="node"><option value="">All nodes</option>${state.nodes.map((n) => `<option value="${esc(n.id)}" ${state.logFilters.node === n.id ? "selected" : ""}>${esc(n.name)}</option>`).join("")}</select>
          <select class="rail-select" data-log-filter="level"><option value="">All levels</option>${["error", "warn", "info", "debug"].map((level) => `<option value="${level}" ${state.logFilters.level === level ? "selected" : ""}>${level}</option>`).join("")}</select>
          <input class="rail-select" data-log-filter="code" value="${esc(state.logFilters.code)}" placeholder="Filter code or text">
        </div>
        <div id="logs-body"><div class="page-loading" style="min-height:150px"><span></span><span></span><span></span></div></div>
      </div></div>
    </div>`;
  }

  function levelClass(level) {
    if (level === "error") return "bad";
    if (level === "warn") return "warn";
    if (level === "info") return "ok";
    return "";
  }
  function renderRuntime(records) {
    if (!records.length) return '<div class="empty" style="min-height:120px"><div><h3>No matching runtime logs</h3><p>Try another level, code or search term.</p></div></div>';
    return `<div class="loglines">${records.map((r) => `<div class="logline"><span class="lvl ${levelClass(r.level)}">${esc(r.level)}</span>${r.code ? `<span class="lcode">${esc(r.code)}</span>` : `<span class="lcode dim">—</span>`}<span class="lmsg">${esc(r.message)}${r.fields ? ` <span class="lfields">${esc(r.fields)}</span>` : ""}</span><time>${esc(new Date(r.ts).toLocaleTimeString())}</time></div>`).join("")}</div>`;
  }
  async function loadRuntimeLogs() {
    const el = document.getElementById("runtime-body");
    if (!el) return;
    try {
      const params = new URLSearchParams({ limit: "500" });
      const filters = state.runtimeLogFilters;
      if (filters.level) params.set("level", filters.level);
      if (filters.code.trim()) params.set("code", filters.code.trim().toUpperCase());
      if (filters.query.trim()) params.set("q", filters.query.trim());
      const records = await api("/system/logs?" + params);
      el.innerHTML = renderRuntime(records);
      const count = document.getElementById("runtime-count");
      if (count) count.textContent = records.length;
    } catch (error) {
      el.innerHTML = `<p class="form-note">${esc(error.message)}</p>`;
    }
  }

  function scheduleRuntimeLogSearch() {
    if (runtimeLogSearchTimer) clearTimeout(runtimeLogSearchTimer);
    runtimeLogSearchTimer = setTimeout(() => {
      runtimeLogSearchTimer = null;
      loadRuntimeLogs();
    }, 180);
  }

  function logStatusClass(status) {
    if (status === "applied") return "ok";
    if (status === "failed") return "bad";
    if (status === "pending") return "warn";
    return "";
  }
  function auditToLog(ev) {
    return {
      ts: ev.created_at, cls: "", tag: "audit",
      title: `${ev.action} · ${ev.resource_type}`,
      sub: `${ev.actor_name || "system"}${ev.resource_id ? " · " + ev.resource_id : ""}`,
      source: "audit", nodeId: ev.resource_type === "node" ? ev.resource_id : "", level: "", code: ev.action,
    };
  }
  function pushToLog(push, nodeName, nodeId = "") {
    return {
      ts: push.started_at, cls: logStatusClass(push.status), tag: `push · ${push.status}`,
      title: `${nodeName ? nodeName + " · " : ""}push (${push.source})`,
      sub: push.message || (push.desired_hash ? "spec " + push.desired_hash.slice(0, 12) : ""),
      source: "push", nodeId, level: push.status === "failed" ? "error" : push.status === "pending" ? "warn" : "info", code: push.status,
    };
  }
  function agentToLog(entry, nodeName, nodeId = "") {
    const cls = entry.level === "error" ? "bad" : entry.level === "warn" ? "warn" : entry.level === "info" ? "ok" : "";
    return {
      ts: entry.at_unix_ms, cls, tag: `agent · ${entry.code}`,
      title: `${nodeName ? nodeName + " · " : ""}${entry.message}`,
      sub: entry.level,
      source: "agent", nodeId, level: entry.level, code: entry.code,
    };
  }
  function offlineToLog(node) {
    return {
      ts: Date.now(), cls: "bad", tag: "agent · offline", level: "error", code: "A0307",
      title: `${node.name} · agent logs unavailable`, sub: "Node is offline or does not support log streaming.",
      source: "agent", nodeId: node.id,
    };
  }
  function byNewest(a, b) { return new Date(b.ts) - new Date(a.ts); }
  function renderTimeline(items) {
    if (!items.length) return '<div class="empty" style="min-height:120px"><div><h3>Nothing logged yet</h3></div></div>';
    return `<div class="timeline">${items.map((it) => `<div class="tl-row"><span class="tl-dot ${it.cls}"></span><div class="tl-body"><b>${esc(it.title)}</b><small>${esc(it.sub)}</small></div><div class="tl-meta"><span class="tl-tag ${it.cls}">${esc(it.tag)}</span><time>${esc(dateLabel(it.ts))}</time></div></div>`).join("")}</div>`;
  }

  function filteredLogs(items) {
    const { node, level, code } = state.logFilters;
    const needle = code.trim().toLowerCase();
    return items.filter((item) => {
      if (node && item.nodeId !== node) return false;
      if (level && item.level !== level) return false;
      if (needle && !`${item.code || ""} ${item.title || ""} ${item.sub || ""}`.toLowerCase().includes(needle)) return false;
      return true;
    });
  }

  function renderFilteredActivity() {
    const el = document.getElementById("logs-body");
    if (el) el.innerHTML = renderTimeline(filteredLogs(state.activityItems).slice(0, 300));
  }

  async function fetchAgentLogs(node, limit = 100) {
    const polls = (state.agentLogPolls[node.id] || 0) + 1;
    state.agentLogPolls[node.id] = polls;
    const cursor = state.agentLogCursor[node.id] || 0;
    const query = new URLSearchParams({ limit: String(limit) });
    if (cursor && polls % 12 !== 0) query.set("after_seq", String(cursor));
    try {
      const records = await api(`/nodes/${node.id}/logs?${query}`);
      const existing = state.agentLogs[node.id] || [];
      const merged = new Map(existing.map((entry) => [`${entry.at_unix_ms}:${entry.seq}:${entry.code}`, entry]));
      records.forEach((entry) => merged.set(`${entry.at_unix_ms}:${entry.seq}:${entry.code}`, entry));
      state.agentLogs[node.id] = [...merged.values()]
        .sort((a, b) => Number(a.at_unix_ms) - Number(b.at_unix_ms)).slice(-500);
      if (records.length) state.agentLogCursor[node.id] = Math.max(...records.map((entry) => Number(entry.seq) || 0));
      state.agentLogStatus[node.id] = "online";
    } catch {
      state.agentLogStatus[node.id] = "offline";
    }
    const items = (state.agentLogs[node.id] || []).map((entry) => agentToLog(entry, node.name, node.id));
    if (state.agentLogStatus[node.id] === "offline") items.push(offlineToLog(node));
    return items;
  }

  async function loadLogs() {
    loadRuntimeLogs();
    const el = $("#logs-body");
    if (!el) return;
    try {
      const audit = await api("/audit");
      const pushLists = await Promise.all(
        state.nodes.map((n) =>
          api(`/nodes/${n.id}/pushes`).then((r) => r.map((p) => pushToLog(p, n.name, n.id))).catch(() => []))
      );
      const agentLists = await Promise.all(
        state.nodes.map((n) => fetchAgentLogs(n, 100))
      );
      const items = [...audit.map(auditToLog), ...pushLists.flat(), ...agentLists.flat()].sort(byNewest).slice(0, 500);
      state.activityItems = items;
      renderFilteredActivity();
    } catch (error) {
      el.innerHTML = `<p class="form-note">${esc(error.message)}</p>`;
    }
  }

  async function loadNodeLogs(nodeId) {
    const el = document.getElementById("node-logs");
    if (!el) return;
    try {
      const [pushes, audit, agent] = await Promise.all([
        api(`/nodes/${nodeId}/pushes`).catch(() => []),
        api("/audit").catch(() => []),
        fetchAgentLogs(state.nodes.find((n) => n.id === nodeId) || { id: nodeId, name: "node" }, 200),
      ]);
      const items = [
        ...pushes.map((p) => pushToLog(p, null, nodeId)),
        ...audit.filter((e) => e.resource_id === nodeId).map(auditToLog),
        ...agent,
      ].sort(byNewest);
      el.innerHTML = renderTimeline(items);
      const count = document.getElementById("node-log-count");
      if (count) count.textContent = items.length;
    } catch (error) {
      el.innerHTML = `<p class="form-note">${esc(error.message)}</p>`;
    }
  }

  async function pollVisibleLogs() {
    if (state.logPollInFlight) return;
    state.logPollInFlight = true;
    try {
      if (categoryView(state.route, state.categorySection) === "logs") await loadLogs();
      else if (state.route === "nodes" && state.detailId) await loadNodeLogs(state.detailId);
    } finally {
      state.logPollInFlight = false;
    }
  }

  function pageHeader(title, copy, actions = "") {
    return `<header class="page-header"><div><h1>${esc(title)}</h1><p>${esc(copy)}</p></div><div class="page-actions">${actions}</div></header>`;
  }

  function addButton(kind, label) {
    return `<button class="button primary" data-action="add-${kind}">${icon("plus")} ${esc(label)}</button>`;
  }

  function refreshButton() {
    return `<button class="button secondary" data-action="refresh">${icon("refresh")} Refresh</button>`;
  }

  function exportButton(resource) {
    return `<button class="row-button" data-action="export-csv" data-resource="${resource}" title="Download the current list as CSV">${icon("logs")} CSV</button>`;
  }

  function onboardingChecklist() {
    const setup = state.onboarding || { completed: 0, total: 0, steps: [] };
    if (!setup.steps.length) return "";
    const complete = setup.completed === setup.total;
    const pct = Math.round(setup.completed / Math.max(setup.total, 1) * 100);
    const rows = setup.steps.map((step, index) => {
      const next = !complete && index === setup.steps.findIndex((item) => !item.complete);
      const cta = step.complete ? `<span class="onb-tag">done</span>`
        : step.action ? `<button class="button ${next ? "primary" : "secondary"}" data-action="${esc(step.action)}">${next ? "Continue" : "Add"}</button>`
        : `<button class="button ${next ? "primary" : "secondary"}" data-route="${esc(step.route)}">Open</button>`;
      return `<div class="onb-row ${step.complete ? "done" : ""} ${next ? "next" : ""}"><span class="onb-check">${step.complete ? icon("check") : index + 1}</span><div class="onb-copy"><b>${esc(step.label)}</b><small>${esc(step.description)}</small></div>${cta}</div>`;
    }).join("");
    return `<section class="panel onboarding"><div class="onb-head"><div><span class="eyebrow">first run</span><h2>${complete ? "Honey is ready" : "Set up honey"}</h2><p>${complete ? "Every part of the first subscription exists in current master state." : "Follow the real control-plane state from domain to client import."}</p></div><div class="onb-progress"><div class="onb-bar"><i style="width:${pct}%"></i></div><span>${setup.completed}/${setup.total}</span></div></div><div class="onb-list">${rows}</div></section>`;
  }

  function helpLabel(label, help) {
    return `<span class="field-label">${esc(label)}<button type="button" class="help-tip" data-tip="${esc(help)}" aria-label="Help: ${esc(label)}">?</button></span>`;
  }

  function renderOverview() {
    const active = state.users.filter((user) => user.active).length;
    const used = state.users.reduce((sum, user) => sum + Number(user.used_traffic_bytes || 0), 0);
    if (state.admin?.role === "reseller") {
      const userShortcuts = state.users.slice(0, 4).map((user) =>
        shortcut(user.username, user.active ? "active user" : user.suppressed_reason || "offline", "users", "users")
      ).join("") || shortcut("Add your first user", "create credentials", "plus", null, "add-user");
      return `<div class="page narrow">
        ${onboardingChecklist()}
        <section class="dashboard-hero reseller-overview">
          <div class="hero-kicker"><span>reseller workspace</span></div>
          <h1>What r we serving today?</h1>
          <button class="hero-search" data-command-open>${icon("search")}<span>Search users, subscriptions or run an action...</span><kbd>Ctrl K</kbd></button>
          <div class="shortcut-grid">
            <div class="shortcut-column"><header><span>Users</span><span>${state.users.length}</span></header>${userShortcuts}</div>
            <div class="shortcut-column"><header><span>Quick actions</span></header>
              ${shortcut("Add user", "issue fresh credentials", "plus", null, "add-user")}
              ${shortcut("Subscriptions", "reveal or rotate links", "link", "subscriptions")}
            </div>
          </div>
        </section>
        <section class="analytics">
          <div class="section-head"><div><h2>Snapshot</h2><p>Your current customer scope</p></div><div class="section-tools">${refreshButton()}</div></div>
          <div class="metric-grid">
            ${metric("Active users", active, `${state.users.length - active} suppressed`, state.users.map((user) => user.active ? 1 : 0), true)}
            ${metric("Traffic used", bytes(used), "current total", state.users.map((user) => Number(user.used_traffic_bytes || 0)), true)}
          </div>
        </section>
      </div>`;
    }
    const online = state.nodes.filter(isNodeOnline).length;
    const protocols = new Set(state.inbounds.map((inbound) => inbound.kind));
    const nodeShortcuts = state.nodes.slice(0, 3).map((node) =>
      shortcut(node.name, `node · ${node.address}`, "node", "nodes")
    ).join("") || shortcut("Add your first node", "start connecting agents", "plus", null, "add-node");
    const userShortcuts = state.users.slice(0, 3).map((user) =>
      shortcut(user.username, user.active ? "active user" : user.suppressed_reason || "offline", "users", "users")
    ).join("") || shortcut("Add your first user", "create credentials", "plus", null, "add-user");

    return `<div class="page narrow">
      ${onboardingChecklist()}
      <section class="dashboard-hero">
        <div class="hero-kicker"><span>master connected · sing-box first</span></div>
        <h1>What r we running today?</h1>
        <button class="hero-search" data-command-open>${icon("search")}<span>Search honey, jump anywhere or run an action...</span><kbd>Ctrl K</kbd></button>
        <div class="shortcut-grid">
          <div class="shortcut-column"><header><span>Nodes</span><span>${state.nodes.length}</span></header>${nodeShortcuts}</div>
          <div class="shortcut-column"><header><span>Users</span><span>${state.users.length}</span></header>${userShortcuts}</div>
          <div class="shortcut-column"><header><span>Quick actions</span></header>
            ${shortcut("Add node", "connect another server", "plus", null, "add-node")}
            ${shortcut("Add user", "issue fresh credentials", "plus", null, "add-user")}
            ${shortcut("Add inbound", "open a protocol listener", "plus", null, "add-inbound")}
          </div>
        </div>
      </section>
      <section class="analytics">
        <div class="section-head"><div><h2>Snapshot</h2><p>Live state from master and all known agents</p></div><div class="section-tools">${refreshButton()}</div></div>
        <div class="metric-grid">
          ${metric("Healthy nodes", `${online} / ${state.nodes.length}`, "seen in the last 2m", state.nodes.map((node) => isNodeOnline(node) ? 1 : 0), true)}
          ${metric("Active users", active, `${state.users.length - active} suppressed`, state.users.map((user) => user.active ? 1 : 0))}
          ${metric("Traffic used", bytes(used), "current total", state.users.map((user) => Number(user.used_traffic_bytes || 0)))}
          ${metric("Protocols", protocols.size, [...protocols].join(", ") || "no inbounds", [...protocols].map((_, i) => i + 1))}
        </div>
      </section>
    </div>`;
  }

  function issueAction(issue) {
    const actions = [];
    if (issue.action === "retry_push") actions.push(`<button class="row-button" data-action="push-node" data-id="${esc(issue.node_id)}">review &amp; retry</button>`);
    if (issue.action === "probe_inbound") actions.push(`<button class="row-button" data-action="probe-inbound" data-id="${esc(issue.entity_id)}">probe now</button>`);
    if (issue.action === "verify_domain") actions.push(`<button class="row-button" data-action="verify-domain" data-id="${esc(issue.entity_id)}">verify now</button>`);
    if (issue.code) actions.push(`<button class="row-button" data-action="open-log-search" data-code="${esc(issue.code)}">logs</button>`);
    return actions.join("");
  }

  function issueDrill(issue) {
    if (issue.entity_type === "node") return `nodes/${issue.entity_id}`;
    if (issue.entity_type === "inbound") return `inbounds/${issue.entity_id}`;
    if (issue.entity_type === "user") return `users/${issue.entity_id}`;
    if (issue.entity_type === "domain") return "domains";
    return issue.node_id ? `nodes/${issue.node_id}` : "issues";
  }

  function renderIssues() {
    const report = state.issueReport || { counts: {}, issues: [] };
    const filters = state.issueFilters;
    const nodeMap = new Map(state.nodes.map((node) => [node.id, node.name]));
    const kinds = [...new Set((report.issues || []).map((issue) => issue.kind))].sort();
    const filtered = sortResource("issues", (report.issues || []).filter((issue) =>
      (!filters.severity || issue.severity === filters.severity) &&
      (!filters.kind || issue.kind === filters.kind) &&
      (!filters.node || issue.node_id === filters.node)
    ));
    const rows = filtered.map((issue) => {
      const severityClass = issue.severity === "critical" ? "bad" : issue.severity === "warning" ? "warn" : "";
      const nodeName = issue.node_id ? (nodeMap.get(issue.node_id) || "unknown") : "global";
      const search = [issue.code, issue.kind, issue.title, issue.message, issue.entity_label, nodeName, ...labelsOf(issue)].join(" ").toLowerCase();
      return `<tr data-search="${esc(search)}" data-labels="${esc(labelsOf(issue).join("|"))}">
        <td><span class="status ${severityClass}">${esc(issue.severity)}</span></td>
        <td class="mono">${esc(issue.code)}</td>
        <td class="issue-copy"><button class="cell-link" data-open="${esc(issueDrill(issue))}">${esc(issue.title)}</button><small>${esc(issue.message)}</small></td>
        <td><span class="chip">${esc(issue.kind)}</span></td>
        <td class="primary-cell">${esc(issue.entity_label)}</td>
        <td>${labelChips(issue.labels)}</td>
        <td>${esc(nodeName)}</td>
        <td class="secondary-cell">${relativeTime(issue.detected_at)}</td>
        <td><div class="row-actions">${issueAction(issue)}<button class="row-button" data-open="${esc(issueDrill(issue))}">open</button></div></td>
      </tr>`;
    }).join("");
    const severityOptions = [["", "All severities"], ["critical", "Critical"], ["warning", "Warning"], ["info", "Info"]];
    const nodeOptions = [["", "All nodes"], ...state.nodes.map((node) => [node.id, node.name])];
    const select = (name, options) => `<select data-issue-filter="${name}" aria-label="Filter issues by ${name}">${options.map(([value, label]) => `<option value="${esc(value)}" ${filters[name] === value ? "selected" : ""}>${esc(label)}</option>`).join("")}</select>`;
    const unavailable = report.unavailable ? `<div class="panel issue-unavailable"><div class="panel-body"><span class="status bad">snapshot unavailable</span><p class="form-note">Master could not derive the current health snapshot. Retry, then inspect Logs if it persists.</p></div></div>` : "";
    const emptyCopy = report.unavailable ? "The health snapshot is unavailable." : (report.counts?.total || 0) > 0 ? "No current issues match these filters." : "Everything in the current health snapshot is clear.";
    return `<div class="page">${pageHeader("Issues", "Current fleet conditions that may need operator attention.", refreshButton())}${unavailable}
      <div class="metric-grid issue-metrics">
        ${metric("Critical", report.counts?.critical || 0, "act now", [0, report.counts?.critical || 0], true)}
        ${metric("Warnings", report.counts?.warning || 0, "check soon", [0, report.counts?.warning || 0])}
        ${metric("Informational", report.counts?.info || 0, "intentional state", [0, report.counts?.info || 0])}
      </div>
      ${savedViewToolbar("issues", "Search issues", "", "", `${select("severity", severityOptions)}${select("kind", [["", "All types"], ...kinds.map((kind) => [kind, kind])])}${select("node", nodeOptions)}`)}
      ${tableShell(`<b>${filtered.length}</b> of <b>${report.counts?.total || 0}</b> current issues. Conditions clear automatically after a successful refresh.`, ["Severity", "Code", "Issue", "Type", "Entity", "Labels", "Node", "Detected", ""], rows, emptyCopy)}
    </div>`;
  }

  function shortcut(label, detail, iconName, route, action) {
    const attrs = route ? `data-route="${route}"` : `data-action="${action}"`;
    return `<button class="shortcut" ${attrs}>${icon(iconName)}<span><b>${esc(label)}</b><small class="secondary-cell"> · ${esc(detail)}</small></span>${icon("chevron")}</button>`;
  }

  function metric(label, value, note, points, wide = false) {
    return `<article class="metric-card ${wide ? "wide" : ""}">
      <div class="metric-label"><span>${esc(label)}</span><span>•••</span></div>
      <div class="metric-value">${esc(value)}<small>${esc(note)}</small></div>
      ${sparkline(points) || '<div class="metric-empty">Not enough data</div>'}
    </article>`;
  }

  function sparkline(values) {
    const points = values.map(Number).filter(Number.isFinite);
    if (points.length < 2) return "";
    const max = Math.max(...points, 1);
    const coords = points.map((value, index) => {
      const x = index * (100 / (points.length - 1));
      const y = 52 - (value / max) * 42;
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    }).join(" ");
    return `<div class="sparkline"><svg viewBox="0 0 100 60" preserveAspectRatio="none"><path class="grid" d="M0 52H100M0 30H100"/><polyline points="${coords}"/></svg></div>`;
  }

  const tableColumns = {
    nodes: [["name", "Name"], ["address", "Address"], ["status", "Status"], ["labels", "Labels"], ["transport", "Transport"], ["version", "sing-box"], ["last_seen", "Last seen"], ["actions", ""]],
    inbounds: [["tag", "Tag"], ["node", "Node"], ["protocol", "Protocol"], ["labels", "Labels"], ["core", "Core"], ["listen", "Listen"], ["security", "Security"], ["status", "Status"], ["reach", "Reach"], ["actions", ""]],
    users: [["username", "Username"], ["uuid", "UUID"], ["status", "Status"], ["labels", "Labels"], ["traffic", "Traffic"], ["expires", "Expires"], ["actions", ""]],
    issues: [["severity", "Severity"], ["code", "Code"], ["issue", "Issue"], ["type", "Type"], ["entity", "Entity"], ["labels", "Labels"], ["node", "Node"], ["detected", "Detected"], ["actions", ""]],
  };

  const tableSorts = {
    nodes: [["name", "Name"], ["status", "Status"], ["last_seen", "Last seen"]],
    inbounds: [["tag", "Tag"], ["node", "Node"], ["port", "Port"]],
    users: [["username", "Username"], ["status", "Status"], ["traffic", "Traffic"]],
    issues: [["severity", "Severity"], ["detected", "Detected"], ["entity", "Entity"]],
  };

  function viewConfig(resource) { return state.tableViews[resource]; }
  function labelsOf(item) { return Array.isArray(item?.labels) ? item.labels : []; }
  function labelChips(labels) {
    return labelsOf({ labels }).length ? `<span class="label-list">${labels.map((label) => `<span class="label-chip">${esc(label)}</span>`).join("")}</span>` : '<span class="secondary-cell">—</span>';
  }
  function columnCell(resource, key, html, cls = "") {
    const visible = viewConfig(resource).columns.includes(key);
    return `<td data-col="${key}" class="${cls}" ${visible ? "" : "hidden"}>${html}</td>`;
  }
  function compareText(left, right) { return String(left || "").localeCompare(String(right || ""), undefined, { numeric: true, sensitivity: "base" }); }
  function sortResource(resource, items) {
    const sort = viewConfig(resource).sort;
    const nodeName = (id) => state.nodes.find((node) => node.id === id)?.name || "";
    const severity = { critical: 0, warning: 1, info: 2 };
    return [...items].sort((a, b) => {
      if (resource === "nodes") return sort === "last_seen" ? compareText(b.last_seen, a.last_seen) : sort === "status" ? Number(isNodeOnline(b)) - Number(isNodeOnline(a)) : compareText(a.name, b.name);
      if (resource === "inbounds") return sort === "port" ? Number(a.listen_port) - Number(b.listen_port) : sort === "node" ? compareText(nodeName(a.node_id), nodeName(b.node_id)) : compareText(a.tag, b.tag);
      if (resource === "users") return sort === "traffic" ? Number(b.used_traffic_bytes) - Number(a.used_traffic_bytes) : sort === "status" ? Number(b.active) - Number(a.active) : compareText(a.username, b.username);
      if (resource === "issues") return sort === "detected" ? compareText(b.detected_at, a.detected_at) : sort === "entity" ? compareText(a.entity_label, b.entity_label) : (severity[a.severity] ?? 9) - (severity[b.severity] ?? 9);
      return 0;
    });
  }

  function savedViewToolbar(resource, placeholder, addKind, addLabel, extra = "") {
    const config = viewConfig(resource);
    const views = state.savedViews.filter((item) => item.resource === resource);
    const active = state.activeSavedViews[resource];
    const options = views.map((item) => `<option value="${item.id}" ${active === item.id ? "selected" : ""}>${esc(item.name)}</option>`).join("");
    const columns = tableColumns[resource].filter(([key]) => key !== "actions").map(([key, label]) => `<label><input type="checkbox" data-view-column="${key}" ${config.columns.includes(key) ? "checked" : ""}> ${esc(label)}</label>`).join("");
    const labelSource = resource === "issues" ? [...state.nodes, ...state.inbounds, ...state.users] : resource === "nodes" ? state.nodes : resource === "inbounds" ? state.inbounds : state.users;
    const knownLabels = [...new Set(labelSource.flatMap(labelsOf))].sort();
    return `<div class="toolbar saved-view-toolbar" data-view-resource="${resource}">
      <label class="table-search">${icon("search")}<input data-table-filter value="${esc(config.search)}" placeholder="${esc(placeholder)}"></label>
      <label class="label-filter">${icon("settings")}<input data-label-filter list="${resource}-labels" value="${esc(config.labels.join(", "))}" placeholder="labels"></label><datalist id="${resource}-labels">${knownLabels.map((label) => `<option value="${esc(label)}">`).join("")}</datalist>
      ${extra}
      <select data-view-sort aria-label="Sort">${tableSorts[resource].map(([value, label]) => `<option value="${value}" ${config.sort === value ? "selected" : ""}>${esc(label)}</option>`).join("")}</select>
      <select data-saved-view aria-label="Saved view"><option value="">Unsaved view</option>${options}</select>
      <details class="column-picker"><summary>Columns</summary><div>${columns}</div></details>
      <button class="row-button" data-action="new-saved-view" data-resource="${resource}">save as</button>
      ${active ? `<button class="row-button" data-action="update-saved-view" data-resource="${resource}" data-id="${active}">update</button><button class="row-button" data-action="rename-saved-view" data-resource="${resource}" data-id="${active}">rename</button><button class="row-button danger" data-action="delete-saved-view" data-resource="${resource}" data-id="${active}">delete</button>` : ""}
      <span class="toolbar-spacer"></span>${refreshButton()}${addKind ? addButton(addKind, addLabel) : ""}
    </div>`;
  }

  function toolbar(placeholder, addKind, addLabel) {
    return `<div class="toolbar"><label class="table-search">${icon("search")}<input data-table-filter placeholder="${esc(placeholder)}"></label><span class="toolbar-spacer"></span>${refreshButton()}${addButton(addKind, addLabel)}</div>`;
  }

  function tableShell(meta, headers, rows, emptyCopy, resource = "") {
    const normalized = headers.map((header) => Array.isArray(header) ? header : ["", header]);
    const visible = resource ? viewConfig(resource).columns : [];
    const count = resource ? normalized.filter(([key]) => !key || visible.includes(key)).length : normalized.length;
    const body = rows || `<tr><td colspan="${count}"><div class="empty"><div><div class="empty-icon">${icon("search")}</div><h3>Nothing here yet</h3><p>${esc(emptyCopy)}</p></div></div></td></tr>`;
    const head = normalized.map(([key, label]) => `<th ${resource && !visible.includes(key) ? "hidden" : ""} data-col="${key}">${label}</th>`).join("");
    return `<div class="table-shell"><div class="table-meta">${meta}</div><div class="table-scroll"><table><thead><tr>${head}</tr></thead><tbody>${body}</tbody></table></div><div class="table-foot">Showing ${rows ? rows.split("<tr").length - 1 : 0} records</div></div>`;
  }

  function selectionSet(resource) { return state.selection[resource] || (state.selection[resource] = new Set()); }
  function selTh(resource) { return ["", `<input type="checkbox" class="row-sel-all" data-sel-all="${resource}" aria-label="Select all">`]; }
  function selTd(resource, id) {
    const on = selectionSet(resource).has(id);
    return `<td class="sel-cell"><input type="checkbox" class="row-sel" data-sel="${resource}" data-id="${id}" ${on ? "checked" : ""} aria-label="Select row"></td>`;
  }
  function batchBar(resource) {
    const n = selectionSet(resource).size;
    const btn = (op, label, cls = "") => `<button class="button ${cls}" data-action="batch-${op}" data-resource="${resource}">${label}</button>`;
    const ops = resource === "users"
      ? btn("enable", "Enable") + btn("disable", "Disable") + btn("reset", "Reset traffic") + btn("rotate", "New sub") + btn("delete", "Delete", "danger")
      : resource === "nodes"
        ? btn("enable", "Enable") + btn("disable", "Disable") + btn("push", "Push") + btn("delete", "Delete", "danger")
        : btn("enable", "Enable") + btn("disable", "Disable") + btn("delete", "Delete", "danger");
    return `<div class="batch-bar" data-batch="${resource}" ${n ? "" : "hidden"}><span class="batch-info"><b class="batch-count">${n}</b> selected</span><span class="batch-actions">${ops}</span><button class="row-button" data-action="batch-clear" data-resource="${resource}">clear</button></div>`;
  }
  function updateBatchBar(resource) {
    const bar = document.querySelector(`.batch-bar[data-batch="${resource}"]`);
    if (!bar) return;
    const n = selectionSet(resource).size;
    bar.hidden = n === 0;
    const cnt = bar.querySelector(".batch-count");
    if (cnt) cnt.textContent = n;
  }
  function toggleSelection(resource, id, on) {
    const set = selectionSet(resource);
    on ? set.add(id) : set.delete(id);
    const cb = document.querySelector(`.row-sel[data-sel="${resource}"][data-id="${id}"]`);
    cb?.closest("tr")?.classList.toggle("row-selected", on);
    updateBatchBar(resource);
  }
  function toggleSelectAll(resource, on) {
    const set = selectionSet(resource);
    document.querySelectorAll(`.row-sel[data-sel="${resource}"]`).forEach((cb) => {
      const tr = cb.closest("tr");
      if (tr && tr.hidden) return; // respect the text/label filter
      cb.checked = on;
      on ? set.add(cb.dataset.id) : set.delete(cb.dataset.id);
      tr?.classList.toggle("row-selected", on);
    });
    updateBatchBar(resource);
  }
  function clearAllSelections() {
    Object.values(state.selection).forEach((set) => set.clear());
  }
  async function runBatch(op, resource) {
    const set = selectionSet(resource);
    if (op === "clear") { set.clear(); return render(); }
    const ids = [...set];
    if (!ids.length) return;
    if (op === "delete" && !confirm(`Delete ${ids.length} ${resource === "inbounds" ? "inbound" : resource.slice(0, -1)}(s)? This cannot be undone.`)) return;
    const patch = (path, body) => api(path, { method: "PATCH", body: JSON.stringify(body) });
    const call = (id) => {
      const base = `/${resource === "inbounds" ? "inbounds" : resource}/${id}`;
      if (op === "enable") return patch(base, { enabled: true });
      if (op === "disable") return patch(base, { enabled: false });
      if (op === "delete") return api(base, { method: "DELETE" });
      if (resource === "users" && op === "reset") return api(`${base}/reset-traffic`, { method: "POST" });
      if (resource === "users" && op === "rotate") return api(`${base}/rotate-sub`, { method: "POST" });
      if (resource === "nodes" && op === "push") return api(`${base}/push`, { method: "POST" });
      return Promise.reject(new Error("unsupported batch op"));
    };
    const results = await Promise.allSettled(ids.map(call));
    const ok = results.filter((r) => r.status === "fulfilled").length;
    const fail = results.length - ok;
    set.clear();
    toast(`${op}: ${ok} ok${fail ? `, ${fail} failed` : ""}`, fail > 0);
    await loadData({ quiet: true });
  }

  function renderNodes() {
    const rows = sortResource("nodes", state.nodes).map((node) => {
      const online = isNodeOnline(node);
      const search = [node.name, node.address, node.transport, node.agent_version, node.singbox_version, ...labelsOf(node)].join(" ");
      return `<tr data-search="${esc(search.toLowerCase())}" data-labels="${esc(labelsOf(node).join("|"))}">
        ${selTd("nodes", node.id)}<td class="primary-cell"><button class="cell-link" data-open="nodes/${node.id}">${esc(node.name)}</button></td><td class="mono">${esc(node.address)}:${esc(node.grpc_port)}</td>
        <td>${node.enabled && node.maintenance ? '<span class="status warn">maintenance</span>' : `<span class="status ${online ? "ok" : node.enabled ? "warn" : "bad"}">${node.enabled ? online ? "online" : "not seen" : "disabled"}</span>`}</td>
        <td>${labelChips(node.labels)}</td>
        <td>${esc(node.transport)}</td><td class="secondary-cell">${esc(node.singbox_version || "—")}</td>
        <td class="secondary-cell">${relativeTime(node.last_seen)}</td>
        <td><div class="row-actions"><button class="row-button" data-action="push-node" data-id="${node.id}">push</button><button class="row-button" data-action="node-history" data-id="${node.id}">history</button><button class="row-button" data-action="node-certs" data-id="${node.id}">certs</button><button class="row-button" data-action="enroll-node" data-id="${node.id}">enroll</button><button class="row-button" data-action="edit-node" data-id="${node.id}">edit</button><button class="row-button" data-action="toggle-node" data-id="${node.id}" data-enabled="${node.enabled}">${node.enabled ? "disable" : "enable"}</button><button class="row-button danger" data-action="delete-node" data-id="${node.id}">delete</button></div></td>
      </tr>`;
    }).join("");
    return `<div class="page">${pageHeader("Nodes", "Servers running honey agent and one or more VPN cores.")}
      ${savedViewToolbar("nodes", "Search nodes", "node", "Add node", exportButton("nodes"))}
      ${batchBar("nodes")}
      ${tableShell(`You have <b>${state.nodes.length}</b> nodes connected to this master.${(() => { const c = state.nodes.reduce((s, n) => s + Number(n.monthly_cost_cents || 0), 0); return c ? ` Infra cost <b>$${(c / 100).toFixed(2)}</b>/mo.` : ""; })()}`, [selTh("nodes"), "Name", "Address", "Status", "Labels", "Transport", "sing-box", "Last seen", ""], rows, "Add a node to connect your first honey agent.")}
    </div>`;
  }

  function renderUsers() {
    const rows = sortResource("users", state.users).map((user) => {
      const used = Number(user.used_traffic_bytes || 0), limit = Number(user.traffic_limit_bytes || 0);
      const ratio = limit > 0 ? Math.min(100, used / limit * 100) : 0;
      const search = [user.username, user.uuid, user.suppressed_reason, ...labelsOf(user)].join(" ");
      return `<tr data-search="${esc(search.toLowerCase())}">${selTd("users", user.id)}<td class="primary-cell"><button class="cell-link" data-open="users/${user.id}">${esc(user.username)}</button></td><td class="mono">${esc(user.uuid)}</td><td><span class="status ${user.active ? "ok" : "bad"}">${user.active ? "active" : esc(user.suppressed_reason || "offline")}</span></td><td><div class="usage-cell"><div class="progress"><i style="width:${ratio}%"></i></div><span>${bytes(used)} / ${limit ? bytes(limit) : "∞"}</span></div></td><td class="secondary-cell">${dateLabel(user.expires_at)}</td><td><div class="row-actions"><button class="row-button" data-action="edit-user" data-id="${user.id}">edit</button><button class="row-button" data-action="rotate-credentials" data-id="${user.id}">credentials</button><button class="row-button" data-action="reset-traffic" data-id="${user.id}">reset</button><button class="row-button" data-action="rotate-sub" data-id="${user.id}">new sub</button><button class="row-button" data-action="toggle-user" data-id="${user.id}" data-enabled="${user.enabled}">${user.enabled ? "disable" : "enable"}</button><button class="row-button danger" data-action="delete-user" data-id="${user.id}">delete</button></div></td></tr>`;
    }).join("");
    const accountAction = state.admin?.role === "reseller" ? '<button class="button secondary" data-action="manage-sessions">Sessions & login history</button>' : "";
    return `<div class="page">${pageHeader("Users", "Credentials, quotas and access lifecycle across every node.", accountAction)}${savedViewToolbar("users", "Search users", "user", "Add user", exportButton("users"))}${batchBar("users")}${tableShell(`You have <b>${state.users.length}</b> users; <b>${state.users.filter((u) => u.active).length}</b> are active.`, [selTh("users"), "Username", "UUID", "Status", "Traffic", "Expires", ""], rows, "Add a user to issue credentials and a subscription.")}</div>`;
  }

  function renderInbounds() {
    const nodeMap = new Map(state.nodes.map((node) => [node.id, node]));
    const rows = sortResource("inbounds", state.inbounds).map((inbound) => {
      const node = nodeMap.get(inbound.node_id);
      const search = [inbound.tag, inbound.kind, inbound.core, node?.name, inbound.listen_port, ...labelsOf(inbound)].join(" ");
      return `<tr data-search="${esc(search.toLowerCase())}">${selTd("inbounds", inbound.id)}<td class="primary-cell"><button class="cell-link" data-open="inbounds/${inbound.id}">${esc(inbound.tag)}</button></td><td>${esc(node?.name || "unknown")}</td><td><span class="badge">${esc(inbound.kind)}</span></td><td>${esc(inbound.core)}</td><td class="mono">${esc(inbound.listen)}:${esc(inbound.listen_port)}</td><td>${inbound.tls_enabled ? "TLS" : "plain"}${inbound.reality ? " · reality" : ""}</td><td><span class="status ${inbound.enabled ? "ok" : "bad"}">${inbound.enabled ? "enabled" : "disabled"}</span></td><td>${reachBadge(inbound)}</td><td><div class="row-actions"><button class="row-button" data-action="edit-inbound" data-id="${inbound.id}">edit</button><button class="row-button danger" data-action="delete-inbound" data-id="${inbound.id}">delete</button></div></td></tr>`;
    }).join("");
    return `<div class="page">${pageHeader("Inbounds", "Protocol listeners distributed across sing-box and xray.")}${savedViewToolbar("inbounds", "Search tags, protocols or nodes", "inbound", "Add inbound", exportButton("inbounds"))}${batchBar("inbounds")}${tableShell(`You have <b>${state.inbounds.length}</b> inbounds on <b>${state.nodes.length}</b> nodes.`, [selTh("inbounds"), "Tag", "Node", "Protocol", "Core", "Listen", "Security", "Status", "Reach", ""], rows, "Add an inbound after connecting at least one node.")}</div>`;
  }

  function renderTraffic() {
    const a = state.trafficAnalytics;
    const rangeButtons = [["24h", "24 hours"], ["7d", "7 days"], ["30d", "30 days"]].map(([key, label]) => `<button class="seg-btn ${a.range === key ? "on" : ""}" data-action="traffic-range" data-range="${key}">${label}</button>`).join("");
    const filters = `<div class="traffic-filters"><label><span>User</span><select data-traffic-filter="user_id"><option value="">All users</option>${state.users.map((u) => `<option value="${esc(u.id)}" ${a.user_id === u.id ? "selected" : ""}>${esc(u.username)}</option>`).join("")}</select></label>${state.admin?.role === "reseller" ? "" : `<label><span>Node</span><select data-traffic-filter="node_id"><option value="">All nodes</option>${state.nodes.map((n) => `<option value="${esc(n.id)}" ${a.node_id === n.id ? "selected" : ""}>${esc(n.name)}</option>`).join("")}</select></label>`}<label><span>Core</span><select data-traffic-filter="core"><option value="">All cores</option><option value="singbox" ${a.core === "singbox" ? "selected" : ""}>sing-box</option><option value="xray" ${a.core === "xray" ? "selected" : ""}>xray</option></select></label></div>`;
    const head = pageHeader("Traffic analytics", "Historical usage from restart-safe counters. Protocol attribution is not inferred from core totals.", `<div class="seg">${rangeButtons}</div><button class="button secondary" data-action="traffic-csv">${icon("logs")} CSV</button><button class="button secondary" data-action="traffic-report">${icon("logs")} Report</button>`);
    if (a.loading) return `<div class="page">${head}<div class="panel empty"><div><div class="empty-icon">${icon("refresh")}</div><h3>Loading history</h3><p>Reading the selected period from master.</p></div></div></div>`;
    if (a.error) return `<div class="page">${head}<div class="panel empty"><div><div class="empty-icon">${icon("issues")}</div><h3>Could not load history</h3><p>${esc(a.error)}</p><button class="button primary" data-action="traffic-refresh">Try again</button></div></div></div>`;
    if (!a.data) return `<div class="page">${head}<div class="panel empty"><div><h3>No history yet</h3><p>Traffic appears after the first agent stats sample.</p></div></div></div>`;
    const d = a.data, s = d.summary || {}, series = d.series || [];
    const totals = series.map((p) => Number(p.up_bytes || 0) + Number(p.down_bytes || 0));
    const change = s.change_percent == null ? "no previous baseline" : `${s.change_percent >= 0 ? "+" : ""}${Number(s.change_percent).toFixed(1)}% vs previous period`;
    const rankRows = (d.top_users || []).map((u) => `<tr><td class="primary-cell">${esc(u.name)}</td><td>${bytes(Number(u.up_bytes || 0) + Number(u.down_bytes || 0))}</td><td>${bytes(u.up_bytes)} up</td><td>${bytes(u.down_bytes)} down</td></tr>`).join("");
    const nodeRows = (d.top_nodes || []).map((n) => `<tr><td class="primary-cell">${esc(n.name)}</td><td>${bytes(Number(n.up_bytes || 0) + Number(n.down_bytes || 0))}</td><td>${bytes(n.up_bytes)} up</td><td>${bytes(n.down_bytes)} down</td></tr>`).join("");
    const coreRows = (d.cores || []).map((c) => `<div class="traffic-breakdown"><span><b>${esc(c.core === "singbox" ? "sing-box" : c.core)}</b><small>${bytes(Number(c.up_bytes || 0) + Number(c.down_bytes || 0))}</small></span><i style="width:${s.total_bytes ? Math.min(100, (Number(c.up_bytes || 0) + Number(c.down_bytes || 0)) / Number(s.total_bytes) * 100) : 0}%"></i></div>`).join("");
    const health = d.health ? `<section class="panel traffic-health"><header><div><b>Fleet health</b><span>Current state, independent of the selected period.</span></div><button class="link" data-route="issues">Open issues</button></header><div class="health-grid"><div><b>${d.health.nodes_online} / ${d.health.nodes_total}</b><span>nodes online</span></div><div><b>${d.health.failed_pushes}</b><span>failed pushes</span></div><div><b>${d.health.unreachable_endpoints}</b><span>unreachable inbounds</span></div></div></section>` : "";
    return `<div class="page">${head}<div class="traffic-range"><div><b>${new Date(d.from).toLocaleString()} вЂ” ${new Date(d.to).toLocaleString()}</b><span>${d.bucket} buckets В· ${d.retention_days} days retained В· ${esc(change)}</span></div>${filters}</div><div class="metric-grid traffic-metrics">${metric("Total", bytes(s.total_bytes), change, totals, true)}${metric("Upload", bytes(s.up_bytes), "selected period", series.map((p) => Number(p.up_bytes || 0)))}${metric("Download", bytes(s.down_bytes), "selected period", series.map((p) => Number(p.down_bytes || 0)))}${metric("Buckets", series.length, "with traffic", series.map(() => 1))}</div><section class="panel traffic-chart-panel"><header><div><b>Usage over time</b><span>Hourly history is rolled up to day buckets for longer ranges.</span></div></header>${trafficChart(series)}</section><div class="split-grid traffic-lower"><section class="panel"><header><div><b>Top users</b><span>Combined upload and download</span></div></header>${rankRows ? `<div class="table-scroll"><table><thead><tr><th>User</th><th>Total</th><th>Upload</th><th>Download</th></tr></thead><tbody>${rankRows}</tbody></table></div>` : '<div class="panel-body empty-copy">No user traffic in this period.</div>'}</section>${state.admin?.role === "reseller" ? "" : `<section class="panel"><header><div><b>Top nodes</b><span>Fleet-wide usage</span></div></header>${nodeRows ? `<div class="table-scroll"><table><thead><tr><th>Node</th><th>Total</th><th>Upload</th><th>Download</th></tr></thead><tbody>${nodeRows}</tbody></table></div>` : '<div class="panel-body empty-copy">No node traffic in this period.</div>'}</section>`}</div><div class="split-grid traffic-lower"><section class="panel"><header><div><b>Core split</b><span>Agent-reported counters; not protocol attribution.</span></div></header><div class="panel-body traffic-breakdowns">${coreRows || '<span class="empty-copy">No core traffic in this period.</span>'}</div></section>${health}</div></div>`;
  }

  function trafficChart(series) {
    if (!series.length) return '<div class="traffic-chart-empty">No traffic samples in this period.</div>';
    const values = series.map((p) => Number(p.up_bytes || 0) + Number(p.down_bytes || 0));
    const max = Math.max(...values, 1), width = 100, height = 60;
    const points = values.map((value, index) => `${(index / Math.max(values.length - 1, 1) * width).toFixed(2)},${(height - value / max * 52 - 4).toFixed(2)}`).join(" ");
    const labels = [series[0], series[Math.floor(series.length / 2)], series[series.length - 1]].filter(Boolean).map((p) => new Date(p.bucket).toLocaleDateString()).join(" В· ");
    return `<div class="traffic-chart"><svg viewBox="0 0 100 60" preserveAspectRatio="none" role="img" aria-label="Traffic usage chart"><path class="traffic-grid" d="M0 56H100M0 32H100M0 8H100"/><polyline points="${points}"/></svg><div class="traffic-chart-labels"><span>${esc(labels)}</span><span>${bytes(max)} peak bucket</span></div></div>`;
  }

  function trafficQueryParams() {
    const durations = { "24h": 24 * 3600_000, "7d": 7 * 86400_000, "30d": 30 * 86400_000 };
    const to = new Date(), from = new Date(to.getTime() - durations[state.trafficAnalytics.range]);
    const query = new URLSearchParams({ from: from.toISOString(), to: to.toISOString(), bucket: state.trafficAnalytics.range === "30d" ? "day" : "hour" });
    ["node_id", "user_id", "core"].forEach((key) => { if (state.trafficAnalytics[key]) query.set(key, state.trafficAnalytics[key]); });
    return query;
  }

  async function loadTrafficAnalytics() {
    if (!state.admin || state.trafficAnalytics.loading) return;
    state.trafficAnalytics.loading = true;
    state.trafficAnalytics.error = "";
    render();
    try { state.trafficAnalytics.data = await api(`/analytics/traffic?${trafficQueryParams()}`); }
    catch (error) { state.trafficAnalytics.error = error.message; }
    finally { state.trafficAnalytics.loading = false; render(); }
  }

  const liveDuration = (ms) => {
    if (!ms || ms < 0) return "—";
    const s = Math.floor(ms / 1000);
    if (s < 60) return `${s}s`;
    const m = Math.floor(s / 60);
    if (m < 60) return `${m}m`;
    const h = Math.floor(m / 60);
    return h < 24 ? `${h}h ${m % 60}m` : `${Math.floor(h / 24)}d ${h % 24}h`;
  };
  const statCard = (label, value, note) => `<article class="metric-card"><div class="metric-label"><span>${esc(label)}</span></div><div class="metric-value">${esc(value)}<small>${esc(note)}</small></div></article>`;

  function renderLive() {
    const st = state.live;
    const head = pageHeader("Live connections", "Who is connected right now, read live from each node's core (sing-box Clash API).", `<button class="button secondary" data-action="live-refresh">${icon("refresh")} Refresh</button>`);
    if (!st.data && st.loading) return `<div class="page">${head}<div class="panel"><div class="panel-body empty-copy">Loading live connections…</div></div></div>`;
    if (st.error) return `<div class="page">${head}<div class="panel"><div class="panel-body empty-copy">${esc(st.error)}</div></div></div>`;
    const conns = st.data || [];
    const users = new Set(conns.filter((c) => c.user).map((c) => c.user));
    const devices = new Set(conns.filter((c) => c.source_ip).map((c) => `${c.user}|${c.source_ip}`));
    const now = Date.now();
    const rows = conns.slice().sort((a, b) => (b.up_bytes + b.down_bytes) - (a.up_bytes + a.down_bytes)).map((c) => {
      const who = c.user ? (c.user_id ? `<button class="cell-link" data-open="users/${c.user_id}">${esc(c.user)}</button>` : esc(c.user)) : '<span class="muted">—</span>';
      return `<tr><td class="primary-cell">${who}</td><td class="secondary-cell">${esc(c.node || "—")}</td><td class="mono">${esc(c.source_ip || "—")}</td><td class="secondary-cell">${esc(c.destination || "—")}</td><td>${esc(c.network || "")}</td><td>${bytes(c.up_bytes)} / ${bytes(c.down_bytes)}</td><td class="secondary-cell">${liveDuration(c.started_at ? now - c.started_at : 0)}</td><td class="secondary-cell">${esc(c.chain || "")}</td></tr>`;
    }).join("");
    const metrics = `<div class="metric-grid">${statCard("Active connections", conns.length, "right now")}${statCard("Online users", users.size, "distinct")}${statCard("Devices", devices.size, "distinct source IPs")}</div>`;
    return `<div class="page">${head}${metrics}${geoPanel()}${tableShell(`<b>${conns.length}</b> active connection${conns.length === 1 ? "" : "s"} across connected nodes. Auto-refreshes every 5s.`, ["User", "Node", "Source IP", "Destination", "Net", "↑ / ↓", "Duration", "Chain"], rows, "No active connections right now.")}</div>`;
  }

  function geoPanel() {
    const g = state.geo;
    const title = `<div class="panel-title">Geographic distribution <button class="row-button" data-action="geo-refresh">refresh</button></div>`;
    if (!g || (g.loading && !g.data)) return `<div class="panel" style="margin-bottom:18px">${title}<div class="panel-body empty-copy">Resolving source addresses…</div></div>`;
    if (g.error && !g.data) return `<div class="panel" style="margin-bottom:18px">${title}<div class="panel-body empty-copy">${esc(g.error)}</div></div>`;
    const d = g.data;
    const note = d.country_ranges > 0
      ? `<p class="form-note">Live snapshot of open connections, bucketed by source address. ${d.country_ranges} country ranges loaded.</p>`
      : `<p class="form-note"><b>No country table loaded.</b> Only reserved/private ranges are recognised, so public addresses show as <code>unknown</code>. Point <code>HONEY_GEOIP_FILE</code> at a CSV (<code>start_ip,end_ip,CC</code>, e.g. exported from MaxMind GeoLite2) and restart to get country attribution. No country ranges are guessed.</p>`;
    if (!d.buckets.length) return `<div class="panel" style="margin-bottom:18px">${title}<div class="panel-body empty-copy">No active connections to place.${note}</div></div>`;
    const max = Math.max(...d.buckets.map((b) => b.connections), 1);
    const rows = d.buckets.map((b) => `<div class="traffic-breakdown"><span><b>${esc(b.code)}</b><small>${b.connections} conn · ${b.users} user${b.users === 1 ? "" : "s"} · ${bytes(b.up_bytes + b.down_bytes)}</small></span><i style="width:${Math.round(b.connections / max * 100)}%"></i></div>`).join("");
    return `<div class="panel" style="margin-bottom:18px">${title}<div class="panel-body traffic-breakdowns">${rows}${note}</div></div>`;
  }
  async function loadGeo() {
    if (!state.admin || state.geo.loading) return;
    state.geo.loading = true;
    state.geo.error = "";
    try { state.geo.data = await api("/analytics/geo"); }
    catch (error) { state.geo.error = error.message; }
    finally {
      state.geo.loading = false;
      if (["live", "traffic-geography"].includes(categoryView(state.route, state.categorySection))) render();
    }
  }
  async function loadLiveConnections() {
    if (!state.admin || state.live.loading) return;
    state.live.loading = true;
    state.live.error = "";
    if (!state.live.data) render();
    try { state.live.data = await api("/live-connections"); }
    catch (error) { state.live.error = error.message; }
    finally { state.live.loading = false; if (categoryView(state.route, state.categorySection) === "live") render(); }
  }

  const uptimeLabel = (secs) => {
    secs = Number(secs) || 0;
    const d = Math.floor(secs / 86400), h = Math.floor((secs % 86400) / 3600), m = Math.floor((secs % 3600) / 60);
    if (d > 0) return `${d}d ${h}h`;
    if (h > 0) return `${h}h ${m}m`;
    return `${m}m`;
  };
  function nodeMetricsPanel(node) {
    const m = state.nodeMetrics[node.id];
    const title = `<div class="panel-title">System metrics <button class="row-button" data-action="refresh-metrics" data-id="${node.id}">refresh</button></div>`;
    if (!m || (m.loading && !m.data)) return `<div class="panel">${title}<div class="panel-body empty-copy">Reading node metrics…</div></div>`;
    if (m.error && !m.data) return `<div class="panel">${title}<div class="panel-body empty-copy">${esc(m.error)}</div></div>`;
    const d = m.data;
    if (!d.supported) return `<div class="panel">${title}<div class="panel-body empty-copy">Host metrics are only available on Linux nodes.</div></div>`;
    const memPct = d.mem_total ? Math.round(d.mem_used / d.mem_total * 100) : 0;
    const diskPct = d.disk_total ? Math.round(d.disk_used / d.disk_total * 100) : 0;
    const cores = `${d.cpu_cores} core${d.cpu_cores === 1 ? "" : "s"}`;
    const cell = (label, value, sub) => `<div class="mstat"><span class="mstat-l">${esc(label)}</span><b class="mstat-v">${esc(value)}</b><span class="mstat-s">${esc(sub)}</span></div>`;
    return `<div class="panel">${title}<div class="metric-mini-grid">
      ${cell("CPU", `${d.cpu_percent.toFixed(1)}%`, cores)}
      ${cell("Memory", `${memPct}%`, `${bytes(d.mem_used)} / ${bytes(d.mem_total)}`)}
      ${cell("Disk", `${diskPct}%`, `${bytes(d.disk_used)} / ${bytes(d.disk_total)}`)}
      ${cell("Network", `↓ ${bytes(d.net_rx_speed)}/s`, `↑ ${bytes(d.net_tx_speed)}/s`)}
      ${cell("Load 1m", d.load1.toFixed(2), cores)}
      ${cell("Uptime", uptimeLabel(d.uptime_secs), "since boot")}
    </div></div>`;
  }
  async function showUpdate() {
    const enabled = !!state.settings?.self_update_enabled;
    showList("Software update", `<div class="form-body"><div class="panel-body empty-copy">Checking GitHub…</div></div>`);
    let u;
    try { u = await api("/update"); } catch (error) {
      showList("Software update", `<div class="form-body"><p class="form-note">${esc(error.message)}</p></div>`);
      return;
    }
    const relLine = u.latest
      ? `<p class="form-note">Current <b>v${esc(u.current)}</b> · latest <b>${esc(u.latest)}</b>${u.published_at ? ` (${dateLabel(u.published_at)})` : ""} · repo <span class="mono">${esc(u.repo)}</span></p>`
      : `<p class="form-note">Could not determine the latest release.</p>`;
    let action;
    if (!u.newer) action = `<span class="status ok">up to date</span>`;
    else if (!u.asset) action = `<span class="status warn">newer release available, but it has no asset for this platform</span>`;
    else if (!enabled) action = `<span class="status warn">update available — enable self-update in Runtime settings to install from here</span>`;
    else action = `<button class="button primary" data-action="apply-update">Download & install ${esc(u.latest)}</button>`;
    const notes = u.notes ? `<div class="check-list" style="margin-top:10px"><div class="check-row"><span style="white-space:pre-wrap">${esc(u.notes.slice(0, 1200))}</span></div></div>` : "";
    showList("Software update", `<div class="form-body">
      ${relLine}
      <p>${action}</p>
      ${notes}
      <p class="form-note">The binary is SHA-256-verified against the release checksums and staged over the running one; a release without a checksums asset is refused. The process must restart to run it — with HA + a supervisor (systemd <code>Restart=always</code>) this is a rolling, zero-downtime upgrade. Enable one-click install in <b>Runtime settings → Software self-update</b>.</p>
    </div>`);
  }
  async function showHa() {
    let d;
    try { d = await api("/ha"); } catch (error) { toast(error.message, true); return; }
    const rows = (d.instances || []).map((i) => {
      const badges = `${i.leader ? '<span class="chip blue">leader</span>' : '<span class="chip">follower</span>'}${i.self_instance ? ' <span class="chip">this instance</span>' : ""}`;
      return `<div class="check-row"><span><b>${esc(i.hostname)}</b> ${badges}<small><span class="mono">${esc(i.instance_id)}</span> · v${esc(i.version)} · up since ${dateLabel(i.started_at)} · seen ${relativeTime(i.last_seen)}</small></span></div>`;
    }).join("");
    showList("High availability", `<div class="form-body">
      <div class="check-list">${rows || '<p class="form-note">No instances registered yet.</p>'}</div>
      <p class="form-note" style="margin-top:10px">This instance: <span class="mono">${esc(d.instance_id)}</span> — <b>${d.is_leader ? "leader" : "follower"}</b>${d.lease_expires_at ? ` · lease valid until ${dateLabel(d.lease_expires_at)}` : " · no live lease"}.</p>
      <p class="form-note">Every instance serves the API; exactly one holds the lease and runs the singleton background loops (reconcile, stats, quota, schedule, monitors, bot). A leader that cannot renew steps down immediately, so takeover only happens after the lease expires. Failover takes up to the lease TTL (<code>HONEY_HA_LEASE_SECS</code>, default 15s).</p>
      <p class="form-note"><b>Requirements:</b> all instances share one PostgreSQL, the same <code>HONEY_SECRET_KEY</code> and the same node certificate directory. <b>Known limits:</b> dial-mode (NAT) nodes must reach the leader — they connect out to one address, and only the leader pushes; panel ACME runs per instance, so terminate TLS at the load balancer for multi-instance setups.</p>
    </div>`);
  }
  async function runBenchmark(id) {
    toast("running speed test…");
    let r;
    try { r = await api(`/nodes/${id}/benchmark?mb=2`, { method: "POST" }); }
    catch (error) { toast(error.message, true); return; }
    showList("Speed test", `<div class="form-body">
      <div class="metric-mini-grid">
        <div class="mstat"><span class="mstat-l">Latency</span><b class="mstat-v">${r.latency_ms.toFixed(1)} ms</b><span class="mstat-s">control round-trip</span></div>
        <div class="mstat"><span class="mstat-l">Upload</span><b class="mstat-v">${r.up_mbps.toFixed(1)} Mbps</b><span class="mstat-s">master → node</span></div>
        <div class="mstat"><span class="mstat-l">Download</span><b class="mstat-v">${r.down_mbps.toFixed(1)} Mbps</b><span class="mstat-s">node → master</span></div>
      </div>
      <p class="form-note" style="margin-top:12px">Transferred ${r.size_mb} MiB per leg over the mTLS <b>control channel</b> — this is a capacity/quality signal for the master↔node path, not a line-rate test of the data plane. Each leg is bounded by the gRPC message limit; latency is measured with an empty leg and subtracted.</p>
    </div>`);
  }
  async function runPreflight(id) {
    let report;
    try { report = await api(`/nodes/${id}/preflight`); } catch (error) { toast(error.message, true); return; }
    const pill = (t) => t.reachable === true ? '<span class="status ok">open</span>'
      : t.reachable === false ? '<span class="status bad">unreachable</span>'
      : '<span class="status">not probeable</span>';
    const rows = (report.targets || []).map((t) => `<div class="check-row"><span><b>${esc(t.label)}</b><small>${esc(t.kind)} · <span class="mono">${esc(t.target)}</span> · ${esc(t.detail)}</small></span>${pill(t)}</div>`).join("");
    const verdict = report.ok
      ? '<span class="status ok">all probeable targets reachable</span>'
      : '<span class="status bad">some targets unreachable</span>';
    showList("Preflight", `<div class="form-body">
      <div class="check-list">${rows || '<p class="form-note">Nothing to probe.</p>'}</div>
      <p class="form-note" style="margin-top:10px">${verdict} · gate: <b>${esc(report.gate)}</b></p>
      <p class="form-note">A probe only proves the port is reachable <b>from the master's network</b> — it is not a guarantee of a "clean" address. Blocklist/reputation checks are a separate concern. UDP/QUIC inbounds can't be TCP-probed from here.</p>
    </div>`);
  }
  function driftBadge(node) {
    const d = state.nodeDrift[node.id];
    const refresh = `<button class="row-button" data-action="refresh-drift" data-id="${node.id}">recheck</button>`;
    if (!d || (d.loading && !d.data)) return `<span class="status">checking…</span> ${refresh}`;
    if (d.error && !d.data) return `<span class="status warn">unavailable</span> ${refresh}`;
    const v = d.data;
    if (v.pending_push) return `<span class="status warn">pending push</span> ${refresh}`;
    if (v.drifted) return `<span class="status bad">drift detected</span> ${refresh}`;
    return `<span class="status ok">in sync</span> ${refresh}`;
  }
  async function loadNodeDrift(id) {
    const cur = state.nodeDrift[id];
    if (cur && cur.loading) return;
    state.nodeDrift[id] = { data: cur ? cur.data : null, loading: true, error: "" };
    try {
      const data = await api(`/nodes/${id}/config-drift`);
      state.nodeDrift[id] = { data, loading: false, error: "" };
    } catch (error) {
      state.nodeDrift[id] = { data: cur ? cur.data : null, loading: false, error: error.message };
    }
    if (state.route === "nodes" && state.detailId === id) render();
  }
  async function loadNodeMetrics(id) {
    const cur = state.nodeMetrics[id];
    if (cur && cur.loading) return;
    state.nodeMetrics[id] = { data: cur ? cur.data : null, loading: true, error: "" };
    try {
      const data = await api(`/nodes/${id}/metrics`);
      state.nodeMetrics[id] = { data, loading: false, error: "" };
    } catch (error) {
      state.nodeMetrics[id] = { data: cur ? cur.data : null, loading: false, error: error.message };
    }
    if (state.route === "nodes" && state.detailId === id) render();
  }

  async function exportTrafficAnalytics() {
    try {
      const response = await fetch(`/analytics/traffic.csv?${trafficQueryParams()}`, { credentials: "same-origin" });
      if (!response.ok) throw new Error(`export failed (${response.status})`);
      const blob = await response.blob(), url = URL.createObjectURL(blob), link = document.createElement("a");
      link.href = url; link.download = "honey-traffic.csv"; link.click(); URL.revokeObjectURL(url); toast("traffic exported");
    } catch (error) { toast(error.message, true); }
  }

  function renderSubscriptions() {
    const rows = state.users.map((user) => `<tr data-search="${esc(user.username.toLowerCase())}"><td class="primary-cell"><button class="cell-link" data-open="users/${user.id}">${esc(user.username)}</button></td><td><span class="status ${user.active ? "ok" : "bad"}">${user.active ? "available" : esc(user.suppressed_reason || "unavailable")}</span></td><td class="secondary-cell">stored, encrypted</td><td><div class="row-actions"><button class="row-button" data-action="reveal-sub" data-id="${user.id}">reveal</button><button class="row-button" data-action="rotate-sub" data-id="${user.id}">rotate</button></div></td></tr>`).join("");
    return `<div class="page">${pageHeader("Subscriptions", "Revocable public links. Reveal shows the current link; rotate invalidates the old one.")}<div class="panel" style="margin-bottom:18px"><div class="panel-body"><p class="form-note">Subscription tokens are stored encrypted at rest and never returned in ordinary user responses. <b>Reveal</b> decrypts the current link for you; users created before this feature have no stored copy and must be rotated once to become revealable.</p></div></div>${toolbar("Search users", "user", "Add user")}${tableShell(`Subscriptions belong to <b>${state.users.length}</b> users.`, ["User", "State", "Current token", ""], rows, "Add a user to create the first subscription.")}</div>`;
  }

  function renderIssueHistory() {
    return `<div class="page">${pageHeader("Issue history", "Operational changes and failures that explain how current conditions evolved.", refreshButton())}
      <div class="panel"><div class="panel-title">Fleet timeline</div><div class="panel-body" id="logs-body">
        ${state.activityItems.length ? renderTimeline(state.activityItems.slice(0, 300)) : '<div class="page-loading" style="min-height:150px"><span></span><span></span><span></span></div>'}
      </div></div>
    </div>`;
  }

  function renderNodeHealth() {
    const rows = state.nodes.map((node) => {
      const online = isNodeOnline(node);
      return `<tr><td class="primary-cell"><button class="cell-link" data-open="nodes/${node.id}">${esc(node.name)}</button></td>
        <td><span class="status ${online ? "ok" : node.enabled ? "warn" : "bad"}">${online ? "online" : node.enabled ? "offline" : "disabled"}</span></td>
        <td>${node.maintenance ? '<span class="status warn">maintenance</span>' : "normal"}</td>
        <td><span class="status ${node.last_push_status === "failed" ? "bad" : node.last_push_status === "applied" ? "ok" : "warn"}">${esc(node.last_push_status || "not pushed")}</span></td>
        <td class="secondary-cell">${relativeTime(node.last_seen)}</td></tr>`;
    }).join("");
    return `<div class="page">${pageHeader("Fleet health", "Agent presence, maintenance state and the last desired-state result.", refreshButton())}
      ${tableShell(`<b>${state.nodes.filter(isNodeOnline).length}</b> of <b>${state.nodes.length}</b> nodes are online.`, ["Node", "Agent", "Mode", "Last push", "Last seen"], rows, "No nodes are registered.")}</div>`;
  }

  function renderNodeVersions() {
    const rows = state.nodes.map((node) => `<tr><td class="primary-cell"><button class="cell-link" data-open="nodes/${node.id}">${esc(node.name)}</button></td>
      <td class="mono">${esc(node.agent_version || "—")}</td><td class="mono">${esc(node.singbox_version || "—")}</td>
      <td class="mono">${esc(node.xray_version || "—")}</td><td class="secondary-cell">${relativeTime(node.last_seen)}</td></tr>`).join("");
    return `<div class="page">${pageHeader("Core versions", "Versions reported by each connected agent; no values are inferred.", refreshButton())}
      ${tableShell("Use this view before a fleet upgrade or protocol compatibility check.", ["Node", "Agent", "sing-box", "Xray", "Reported"], rows, "No version reports are available.")}</div>`;
  }

  function renderNodeEnrollment() {
    const rows = state.nodes.map((node) => `<tr><td class="primary-cell"><button class="cell-link" data-open="nodes/${node.id}">${esc(node.name)}</button></td>
      <td>${esc(node.transport || "serve")}</td><td class="mono">${esc(node.tls_server_name || "—")}</td>
      <td class="secondary-cell">${esc(node.address)}:${esc(node.grpc_port)}</td>
      <td><button class="row-button" data-action="enroll-node" data-id="${node.id}">issue token</button></td></tr>`).join("");
    return `<div class="page">${pageHeader("Node enrollment", "Issue one-time identities for honey-agent without exposing the master CA key.", '<button class="button primary" data-route="new-node">Add node</button>')}
      <div class="panel" style="margin-bottom:16px"><div class="panel-body"><p class="form-note">Tokens are single-use. The resulting certificate is bound to the node identity and transport SNI.</p></div></div>
      ${tableShell(`<b>${state.nodes.length}</b> registered node${state.nodes.length === 1 ? "" : "s"}.`, ["Node", "Transport", "TLS server name", "Agent endpoint", ""], rows, "Register a node before issuing an enrollment token.")}</div>`;
  }

  function renderInboundProtocols() {
    const grouped = new Map();
    state.inbounds.forEach((inbound) => {
      const key = `${inbound.kind}|${inbound.core}`;
      const current = grouped.get(key) || { protocol: inbound.kind, core: inbound.core, total: 0, enabled: 0, ports: [] };
      current.total += 1;
      current.enabled += inbound.enabled ? 1 : 0;
      current.ports.push(inbound.listen_port);
      grouped.set(key, current);
    });
    const rows = [...grouped.values()].map((item) => `<tr><td class="primary-cell">${esc(item.protocol)}</td><td>${esc(item.core)}</td>
      <td>${item.total}</td><td><span class="status ${item.enabled === item.total ? "ok" : "warn"}">${item.enabled}/${item.total} enabled</span></td>
      <td class="mono">${esc([...new Set(item.ports)].sort((a, b) => a - b).join(", "))}</td></tr>`).join("");
    return `<div class="page">${pageHeader("Protocols", "Actual protocol and core combinations currently deployed.", '<button class="button primary" data-route="new-inbound">Create inbound</button>')}
      ${tableShell(`<b>${grouped.size}</b> protocol/core combination${grouped.size === 1 ? "" : "s"}.`, ["Protocol", "Core", "Inbounds", "State", "Ports"], rows, "No protocol listeners are configured.")}</div>`;
  }

  function renderInboundSecurity() {
    const rows = state.inbounds.map((inbound) => {
      const mode = inbound.reality ? "REALITY" : inbound.tls_enabled ? "TLS" : "plain";
      return `<tr><td class="primary-cell"><button class="cell-link" data-open="inbounds/${inbound.id}">${esc(inbound.tag)}</button></td>
        <td><span class="status ${mode === "plain" ? "warn" : "ok"}">${mode}</span></td><td class="mono">${esc(inbound.server_name || "—")}</td>
        <td>${esc(inbound.network || "tcp")}</td><td>${esc(inbound.core)}</td></tr>`;
    }).join("");
    return `<div class="page">${pageHeader("Inbound security", "Transport security by listener, including TLS and REALITY server names.")}
      ${tableShell("Plain listeners may be intentional on trusted networks; review them before public exposure.", ["Inbound", "Security", "Server name", "Network", "Core"], rows, "No inbounds are configured.")}</div>`;
  }

  function renderUserAccess() {
    const profiles = new Map(state.profiles.map((profile) => [profile.id, profile.name]));
    const rows = state.users.map((user) => `<tr><td class="primary-cell"><button class="cell-link" data-open="users/${user.id}">${esc(user.username)}</button></td>
      <td>${esc(profiles.get(user.routing_profile_id) || "Default profile")}</td><td>${user.device_limit ? esc(user.device_limit) : "unlimited"}</td>
      <td><span class="status ${user.active ? "ok" : "bad"}">${user.active ? "allowed" : esc(user.suppressed_reason || "blocked")}</span></td></tr>`).join("");
    return `<div class="page">${pageHeader("User access", "Effective profile, device policy and account availability.")}
      ${tableShell("Group membership is managed from each user detail and controls node reachability.", ["User", "Routing profile", "Device limit", "Access"], rows, "No users are configured.")}</div>`;
  }

  function renderUserQuotas() {
    const rows = [...state.users].sort((a, b) => Number(b.used_traffic_bytes || 0) - Number(a.used_traffic_bytes || 0)).map((user) => {
      const used = Number(user.used_traffic_bytes || 0), limit = Number(user.traffic_limit_bytes || 0);
      const ratio = limit ? Math.min(100, used / limit * 100) : 0;
      return `<tr><td class="primary-cell"><button class="cell-link" data-open="users/${user.id}">${esc(user.username)}</button></td>
        <td><div class="usage-cell"><div class="progress"><i style="width:${ratio}%"></i></div><span>${bytes(used)}</span></div></td>
        <td>${limit ? bytes(limit) : "unlimited"}</td><td>${esc(user.quota_interval || "none")}</td>
        <td><button class="row-button" data-action="reset-traffic" data-id="${user.id}">reset</button></td></tr>`;
    }).join("");
    return `<div class="page">${pageHeader("User quotas", "Current restart-safe traffic counters and reset windows.", exportButton("traffic"))}
      ${tableShell(`<b>${state.users.filter((user) => Number(user.traffic_limit_bytes || 0) > 0).length}</b> users have an explicit traffic limit.`, ["User", "Used", "Limit", "Window", ""], rows, "No quota data is available.")}</div>`;
  }

  function renderUserLifecycle() {
    const rows = state.users.map((user) => `<tr><td class="primary-cell"><button class="cell-link" data-open="users/${user.id}">${esc(user.username)}</button></td>
      <td>${dateLabel(user.created_at)}</td><td>${user.expires_at ? dateLabel(user.expires_at) : "never"}</td>
      <td><span class="status ${user.active ? "ok" : "bad"}">${user.active ? "active" : esc(user.suppressed_reason || "inactive")}</span></td>
      <td><button class="row-button" data-action="edit-user" data-id="${user.id}">edit lifecycle</button></td></tr>`).join("");
    return `<div class="page">${pageHeader("User lifecycle", "Creation, expiry and suppression state without subscription secrets.")}
      ${tableShell(`<b>${state.users.filter((user) => user.active).length}</b> active users.`, ["User", "Created", "Expires", "State", ""], rows, "No users are configured.")}</div>`;
  }

  function renderGroupCoverage() {
    const rows = state.groups.map((group) => `<tr><td class="primary-cell">${esc(group.name)}</td>
      <td>${group.is_default ? '<span class="status ok">default</span>' : '<span class="status">explicit</span>'}</td>
      <td class="secondary-cell">${esc(group.note || "—")}</td><td><button class="row-button" data-action="edit-group" data-id="${group.id}">inspect</button></td></tr>`).join("");
    return `<div class="page">${pageHeader("Group coverage", "Placement domains used to match users with nodes.")}
      ${tableShell("A node without groups remains universal; grouped nodes require at least one shared group.", ["Group", "Scope", "Purpose", ""], rows, "No groups are configured.")}</div>`;
  }

  function renderGroupPolicies() {
    return `<div class="page narrow">${pageHeader("Group policies", "How honey resolves group-based node access.")}
      <div class="panel settings-grid">
        <div class="setting-row"><div class="setting-copy"><b>Universal nodes</b><span>A node with no assigned group is available to every active user.</span></div><span class="chip">default allow</span></div>
        <div class="setting-row"><div class="setting-copy"><b>Grouped nodes</b><span>A grouped node is available only when the user shares at least one group.</span></div><span class="chip">intersection</span></div>
        <div class="setting-row"><div class="setting-copy"><b>Inbound inheritance</b><span>Inbounds inherit access from their node; per-inbound group assignment is intentionally absent.</span></div><span class="chip">node scoped</span></div>
      </div></div>`;
  }

  function renderTrafficQuotas() {
    const limited = state.users.filter((user) => Number(user.traffic_limit_bytes || 0) > 0);
    const ranked = limited.map((user) => {
      const used = Number(user.used_traffic_bytes || 0), limit = Number(user.traffic_limit_bytes || 0);
      return { user, used, limit, ratio: limit ? used / limit * 100 : 0 };
    }).sort((a, b) => b.ratio - a.ratio);
    const near = ranked.filter((item) => item.ratio >= 80);
    const rows = ranked.map(({ user, used, limit, ratio }) => `<tr><td class="primary-cell"><button class="cell-link" data-open="users/${user.id}">${esc(user.username)}</button></td>
      <td><span class="status ${ratio >= 100 ? "bad" : ratio >= 80 ? "warn" : "ok"}">${Math.round(ratio)}%</span></td>
      <td>${bytes(used)}</td><td>${bytes(limit)}</td><td>${esc(user.quota_interval || "none")}</td></tr>`).join("");
    return `<div class="page">${pageHeader("Quota usage", "Fleet-level pressure view for users with explicit traffic limits.", exportButton("traffic"))}
      <div class="metric-grid">${statCard("Limited users", limited.length, "explicit quota")}${statCard("At 80%+", near.length, "needs attention")}${statCard("Unlimited", state.users.length - limited.length, "no traffic cap")}</div>
      ${tableShell("Users are ordered by quota utilisation, highest first.", ["User", "Used %", "Traffic", "Limit", "Window"], rows, "No users currently have an explicit traffic limit.")}</div>`;
  }

  function renderTrafficGeography() {
    return `<div class="page">${pageHeader("Traffic geography", "Live source distribution from the configured GeoIP range table.", '<button class="button secondary" data-action="geo-refresh">Refresh</button>')}
      ${geoPanel()}</div>`;
  }

  function renderSubscriptionFormats() {
    const formats = [
      ["Universal page", "/sub/:token", "Browser landing page with explicit client downloads", "all"],
      ["V2Ray links", "/sub/:token/v2ray", "Base64 share links with quota headers", "Happ, v2rayN"],
      ["Plain links", "/sub/:token/links", "One share link per line", "manual import"],
      ["sing-box", "/sub/:token/sing-box", "Ready client configuration", "sing-box, Karing"],
      ["sing-box TUN", "/sub/:token/sing-box-tun", "System-wide TUN preset", "sing-box"],
      ["Clash", "/sub/:token/clash", "Clash/Mihomo configuration with routing", "Mihomo clients"],
    ];
    const rows = formats.map(([name, path, detail, clients]) => `<tr><td class="primary-cell">${esc(name)}</td><td class="mono">${esc(path)}</td><td>${esc(detail)}</td><td class="secondary-cell">${esc(clients)}</td></tr>`).join("");
    return `<div class="page">${pageHeader("Client formats", "Explicit subscription outputs; clients should use the format they actually support.")}
      ${tableShell("The base URL is a human-facing landing page, not a universal JSON endpoint.", ["Format", "Path", "Output", "Typical clients"], rows, "")}</div>`;
  }

  function renderSubscriptionDelivery() {
    const endpoints = state.inbounds.filter((inbound) => inbound.enabled).length;
    const rows = state.users.map((user) => `<tr><td class="primary-cell"><button class="cell-link" data-open="users/${user.id}">${esc(user.username)}</button></td>
      <td><span class="status ${user.active ? "ok" : "bad"}">${user.active ? "deliverable" : esc(user.suppressed_reason || "blocked")}</span></td>
      <td>${endpoints}</td><td>${user.expires_at ? dateLabel(user.expires_at) : "never"}</td>
      <td><button class="row-button" data-action="reveal-sub" data-id="${user.id}">test link</button></td></tr>`).join("");
    return `<div class="page">${pageHeader("Subscription delivery", "Whether each user can receive the currently enabled endpoints.")}
      ${tableShell(`<b>${state.users.filter((user) => user.active).length}</b> deliverable subscriptions across <b>${endpoints}</b> enabled inbounds.`, ["User", "Delivery", "Endpoints", "Expires", ""], rows, "No subscriptions exist.")}</div>`;
  }

  function renderDomains() {
    const nodeMap = new Map(state.nodes.map((n) => [n.id, n.name]));
    const rows = state.domains.map((d) => {
      const nodeName = d.node_id ? (nodeMap.get(d.node_id) || "unknown") : "—";
      const checked = Boolean(d.last_checked_at);
      const good = d.dns_ok && d.reachable_443;
      const statusCls = !checked ? "" : good ? "ok" : (d.dns_ok || d.reachable_443) ? "warn" : "bad";
      const statusLabel = !checked ? "unchecked" : good ? "verified" : d.dns_ok ? "dns ok" : d.reachable_443 ? "reachable" : "failed";
      let certCell = "—";
      if (d.cert_not_after) {
        const days = Math.round((new Date(d.cert_not_after) - Date.now()) / 86400000);
        const certCls = !d.cert_ok ? "bad" : days <= 14 ? "warn" : "ok";
        certCell = `<span class="status ${certCls}">${d.cert_ok ? "in " + days + "d" : "expired"}</span>`;
      }
      const search = [d.host, nodeName, d.notes].join(" ").toLowerCase();
      return `<tr data-search="${esc(search)}">
        <td class="primary-cell"><span class="mono">${esc(d.host)}</span></td>
        <td>${d.proxied ? '<span class="chip blue">CDN</span>' : '<span class="chip">direct</span>'}</td>
        <td>${esc(nodeName)}</td>
        <td><span class="status ${statusCls}" title="${esc(d.check_error || "")}">${statusLabel}</span></td>
        <td class="secondary-cell mono">${d.resolved_ips && d.resolved_ips.length ? esc(d.resolved_ips.join(", ")) : "—"}</td>
        <td>${certCell}</td>
        <td class="secondary-cell">${checked ? relativeTime(d.last_checked_at) : "never"}</td>
        <td><div class="row-actions"><button class="row-button" data-action="verify-domain" data-id="${d.id}">verify</button><button class="row-button" data-action="edit-domain" data-id="${d.id}">edit</button><button class="row-button danger" data-action="delete-domain" data-id="${d.id}">delete</button></div></td>
      </tr>`;
    }).join("");
    return `<div class="page">${pageHeader("Domains", "Domains you own, pointed at nodes or a CDN — picked from a validated list when configuring inbounds.")}
      <div class="panel" style="margin-bottom:16px"><div class="panel-body"><p class="form-note">CDN uses its public hostname instead of the origin address, but is not an availability guarantee. REALITY SNI and <b>target</b> stay independent from this registry; see the deployment documentation for route-specific testing.</p></div></div>
      ${toolbar("Search domains", "domain", "Add domain")}
      ${tableShell(`You have <b>${state.domains.length}</b> managed domain${state.domains.length === 1 ? "" : "s"}.`, ["Host", "Kind", "Node", "Status", "Resolves to", "Cert", "Checked", ""], rows, "Register a domain you own to reference it from inbounds.")}
    </div>`;
  }

  function renderDomainDns() {
    const rows = state.domains.map((domain) => {
      const ok = domain.dns_ok;
      return `<tr><td class="primary-cell mono">${esc(domain.host)}</td>
        <td><span class="status ${ok ? "ok" : domain.last_checked_at ? "bad" : "warn"}">${ok ? "resolved" : domain.last_checked_at ? "failed" : "unchecked"}</span></td>
        <td class="mono">${domain.resolved_ips?.length ? esc(domain.resolved_ips.join(", ")) : "—"}</td>
        <td>${domain.proxied ? '<span class="chip blue">CDN</span>' : '<span class="chip">direct</span>'}</td>
        <td class="secondary-cell">${domain.last_checked_at ? relativeTime(domain.last_checked_at) : "never"}</td>
        <td><button class="row-button" data-action="verify-domain" data-id="${domain.id}">verify</button></td></tr>`;
    }).join("");
    return `<div class="page">${pageHeader("DNS status", "Resolved addresses and CDN mode from the latest explicit verification.", refreshButton())}
      ${tableShell(`<b>${state.domains.filter((domain) => domain.dns_ok).length}</b> of <b>${state.domains.length}</b> domains resolve successfully.`, ["Host", "DNS", "Addresses", "Mode", "Checked", ""], rows, "No managed domains are registered.")}</div>`;
  }

  function renderDomainCertificates() {
    const rows = state.domains.map((domain) => {
      const days = domain.cert_not_after ? Math.ceil((new Date(domain.cert_not_after) - Date.now()) / 86400000) : null;
      return `<tr><td class="primary-cell mono">${esc(domain.host)}</td>
        <td><span class="status ${domain.cert_ok ? days <= 14 ? "warn" : "ok" : "bad"}">${domain.cert_ok ? "valid" : "unavailable"}</span></td>
        <td>${days == null ? "—" : days < 0 ? `${Math.abs(days)}d expired` : `${days}d remaining`}</td>
        <td>${domain.reachable_443 ? '<span class="status ok">reachable</span>' : '<span class="status warn">not verified</span>'}</td>
        <td><button class="row-button" data-action="verify-domain" data-id="${domain.id}">refresh</button></td></tr>`;
    }).join("");
    return `<div class="page">${pageHeader("Domain certificates", "Certificate observations made through managed-domain verification.")}
      ${tableShell("This view reports public certificate state; private control-plane mTLS is managed per node.", ["Host", "Certificate", "Validity", "HTTPS", ""], rows, "No certificate observations are available.")}</div>`;
  }

  async function verifyDomain(id) {
    try {
      toast("verifying…");
      const updated = await api(`/domains/${id}/verify`, { method: "POST" });
      const i = state.domains.findIndex((d) => d.id === id);
      if (i >= 0) state.domains[i] = updated;
      await loadData({ quiet: true });
      const good = updated.dns_ok && updated.reachable_443;
      toast(good ? "domain verified" : (updated.check_error || "verify finished with warnings"), !good);
    } catch (error) {
      toast(error.message, true);
    }
  }

  function renderGroups() {
    const rows = state.groups.map((g) => `<tr data-search="${esc((g.name + " " + (g.note || "")).toLowerCase())}">
      <td class="primary-cell">${esc(g.name)}${g.is_default ? ' <span class="chip blue">default</span>' : ""}</td>
      <td class="secondary-cell">${esc(g.note || "—")}</td>
      <td><div class="row-actions"><button class="row-button" data-action="edit-group" data-id="${g.id}">edit</button>${g.is_default ? "" : `<button class="row-button danger" data-action="delete-group" data-id="${g.id}">delete</button>`}</div></td>
    </tr>`).join("");
    return `<div class="page">${pageHeader("Groups", "Access model — a node with no group is universal (all users reach it); a grouped node only serves users who share a group.")}
      <div class="panel" style="margin-bottom:16px"><div class="panel-body"><p class="form-note">Assign nodes to groups (node page → <b>Manage groups</b>) and grant users access (user page → <b>Manage groups</b>). Per-inbound assignment is gone — access is entirely group-based.</p></div></div>
      ${toolbar("Search groups", "group", "Add group")}
      ${tableShell(`You have <b>${state.groups.length}</b> group${state.groups.length === 1 ? "" : "s"}.`, ["Name", "Note", ""], rows, "Create a group, then assign nodes and grant users.")}
    </div>`;
  }

  async function manageGroups(kind, id) {
    try {
      const current = new Set(await api(`/${kind}s/${id}/groups`));
      const rows = state.groups.length
        ? state.groups.map((g) => `<label class="check-row"><input type="checkbox" data-group="${g.id}" ${current.has(g.id) ? "checked" : ""}><span><b>${esc(g.name)}</b><small>${g.is_default ? "default" : ""}</small></span></label>`).join("")
        : '<div class="check-row"><span><small>no groups yet — create one in Groups</small></span></div>';
      const entity = kind === "node" ? state.nodes.find((n) => n.id === id) : state.users.find((u) => u.id === id);
      const label = entity?.name || entity?.username || id;
      const note = kind === "node" ? "No groups selected = universal (every user reaches this node)." : "Groups this user can access. Ungrouped (universal) nodes are always reachable regardless.";
      showList(`Groups · ${label}`, `<div class="form-body"><p class="form-note">${note}</p><div class="check-list">${rows}</div><button class="button primary" data-action="save-groups" data-id="${id}" data-kind="${kind}" style="margin-top:12px">Save</button><p class="field-error" id="grp-err"></p></div>`);
    } catch (error) { toast(error.message, true); }
  }
  async function saveGroups(kind, id) {
    const ids = Array.from(document.querySelectorAll("[data-group]:checked")).map((el) => el.dataset.group);
    try {
      await api(`/${kind}s/${id}/groups`, { method: "PUT", body: JSON.stringify({ group_ids: ids }) });
      formDialog.close();
      toast("groups updated");
      await loadData({ quiet: true });
    } catch (error) { const el = document.getElementById("grp-err"); if (el) el.textContent = error.message; }
  }

  function renderSslTls() {
    const nodeCount = state.nodes.length;
    const realityCount = state.inbounds.filter((i) => i.reality).length;
    const tlsCount = state.inbounds.filter((i) => i.tls_enabled).length;
    const acmeCount = state.inbounds.filter((i) => i.extra && i.extra.acme).length;
    return `<div class="page narrow">${pageHeader("SSL/TLS", "Certificates and transport security across master, agents and inbounds.")}
      <div class="metric-grid" style="margin-bottom:22px">
        ${metric("TLS inbounds", tlsCount, "data-plane encryption", state.inbounds.map((i) => i.tls_enabled ? 1 : 0), true)}
        ${metric("ACME inbounds", acmeCount, "auto-issued & renewed", state.inbounds.map((i) => (i.extra && i.extra.acme) ? 1 : 0))}
        ${metric("REALITY inbounds", realityCount, "SNI camouflage", state.inbounds.map((i) => i.reality ? 1 : 0))}
      </div>
      <div class="panel settings-grid">
        <div class="setting-row"><div class="setting-copy"><b>Control-plane mTLS</b><span>Agents authenticate to master with per-node certificates issued through one-time enrollment.</span></div><button class="button secondary" data-route="nodes">${icon("node")} Manage nodes</button></div>
        <div class="setting-row"><div class="setting-copy"><b>Node certificates</b><span>Per-node fingerprints, expiry and revocation — open a node and use <b>certs</b> / <b>enroll</b>.</span></div><span class="chip">${nodeCount} node${nodeCount === 1 ? "" : "s"}</span></div>
        <div class="setting-row"><div class="setting-copy"><b>Inbound TLS &amp; REALITY</b><span>Data-plane inbounds use ACME, cert_path/key_path or REALITY handshake camouflage. Configure per inbound.</span></div><button class="button secondary" data-route="inbounds">${icon("inbound")} Open inbounds</button></div>
        <div class="setting-row"><div class="setting-copy"><b>ACME automation (sing-box)</b><span>Pick <b>automatic (ACME)</b> when adding a TLS inbound — sing-box obtains &amp; auto-renews via Let's Encrypt (HTTP-01 on :80). xray inbounds still use manual paths.</span></div><span class="chip blue">${acmeCount} active</span></div>
        <div class="setting-row"><div class="setting-copy"><b>Certificate expiry &amp; alerts</b><span>Expiry is monitored through Issues and notification channels.</span></div><span class="chip blue">Active</span></div>
      </div></div>`;
  }

  function renderTlsCertificates() {
    const tls = state.inbounds.filter((inbound) => inbound.tls_enabled || inbound.reality);
    const rows = tls.map((inbound) => `<tr><td class="primary-cell"><button class="cell-link" data-open="inbounds/${inbound.id}">${esc(inbound.tag)}</button></td>
      <td>${inbound.reality ? "REALITY keypair" : inbound.extra?.acme ? "ACME certificate" : "manual certificate"}</td>
      <td class="mono">${esc(inbound.server_name || "—")}</td><td>${esc(inbound.core)}</td>
      <td><span class="status ${inbound.enabled ? "ok" : "warn"}">${inbound.enabled ? "in use" : "disabled"}</span></td></tr>`).join("");
    return `<div class="page">${pageHeader("TLS certificates", "Data-plane certificate and REALITY identities attached to inbounds.")}
      ${tableShell(`<b>${tls.length}</b> secured inbound${tls.length === 1 ? "" : "s"}.`, ["Inbound", "Identity source", "Server name", "Core", "State"], rows, "No secured inbounds are configured.")}</div>`;
  }

  function renderTlsAcme() {
    const acme = state.inbounds.filter((inbound) => inbound.extra?.acme);
    const rows = acme.map((inbound) => `<tr><td class="primary-cell"><button class="cell-link" data-open="inbounds/${inbound.id}">${esc(inbound.tag)}</button></td>
      <td class="mono">${esc(inbound.server_name || "—")}</td><td>${esc(inbound.core)}</td><td class="mono">HTTP-01 · :80</td>
      <td><span class="status ${inbound.enabled ? "ok" : "warn"}">${inbound.enabled ? "managed" : "disabled"}</span></td></tr>`).join("");
    return `<div class="page">${pageHeader("ACME", "Automatic certificate issuance and renewal for sing-box and Honey-managed Xray.")}
      <div class="panel" style="margin-bottom:16px"><div class="panel-body"><p class="form-note">HTTP-01 requires the hostname to resolve to this node and public TCP port 80 to reach Caddy, which forwards the challenge to the local ACME gateway on :9080.</p></div></div>
      ${tableShell(`<b>${acme.length}</b> ACME-managed inbound${acme.length === 1 ? "" : "s"}.`, ["Inbound", "Hostname", "Core", "Challenge", "State"], rows, "No inbounds currently use automatic ACME.")}</div>`;
  }

  function renderTlsReality() {
    const reality = state.inbounds.filter((inbound) => inbound.reality);
    const rows = reality.map((inbound) => `<tr><td class="primary-cell"><button class="cell-link" data-open="inbounds/${inbound.id}">${esc(inbound.tag)}</button></td>
      <td class="mono">${esc(inbound.server_name || "—")}</td><td class="mono">${esc(inbound.extra?.reality_target || inbound.extra?.target || "—")}</td>
      <td>${esc(inbound.flow || "—")}</td><td>${esc(inbound.core)}</td></tr>`).join("");
    return `<div class="page">${pageHeader("REALITY", "SNI and handshake targets for REALITY-enabled Xray listeners.")}
      <div class="panel" style="margin-bottom:16px"><div class="panel-body"><p class="form-note">In filtered networks, use a hostname you control and validate the external ClientHello path separately from local fallback.</p></div></div>
      ${tableShell(`<b>${reality.length}</b> REALITY inbound${reality.length === 1 ? "" : "s"}.`, ["Inbound", "SNI", "Handshake target", "Flow", "Core"], rows, "No REALITY inbounds are configured.")}</div>`;
  }

  function renderRules() {
    const rows = state.profiles.map((p) => {
      const tags = [
        p.block_ads && "ad-block",
        p.block_adult && "block-adult",
        p.block_gambling && "block-gambling",
        p.direct_private && "bypass-LAN",
        (p.direct_geosite || []).includes("cn") && "direct-CN",
        (p.direct_geosite || []).includes("ru") && "direct-RU",
        (p.blocked_domains || []).length && `block ${p.blocked_domains.length}d`,
        (p.direct_domains || []).length && `direct ${p.direct_domains.length}d`,
        (p.proxy_domains || []).length && `proxy ${p.proxy_domains.length}d`,
        (p.app_rules || []).length && `${p.app_rules.length} app-rule${p.app_rules.length === 1 ? "" : "s"}`,
        (p.dns_doh || "").trim() && `DoH${p.dns_fakeip ? "+fakeip" : ""}${p.dns_block_plain ? "+no53" : ""}`,
        p.final_proxy ? "final→proxy" : "final→direct",
      ].filter(Boolean).join(" · ");
      return `<tr data-search="${esc(p.name.toLowerCase())}">
        <td class="primary-cell">${esc(p.name)}${p.is_default ? ' <span class="chip blue">default</span>' : ""}</td>
        <td class="secondary-cell">v${p.version}</td>
        <td class="secondary-cell">${esc(tags || "—")}</td>
        <td><div class="row-actions">${p.is_default ? "" : `<button class="row-button" data-action="default-profile" data-id="${p.id}">set default</button>`}<button class="row-button" data-action="edit-profile" data-id="${p.id}">edit</button><button class="row-button danger" data-action="delete-profile" data-id="${p.id}">delete</button></div></td>
      </tr>`;
    }).join("");
    return `<div class="page">${pageHeader("Rules", "Routing profiles delivered inside every subscription — clients apply them on import.")}
      <div class="panel" style="margin-bottom:16px"><div class="panel-body"><p class="form-note">A profile is a set of routing toggles that sing-box and Clash outputs translate into native rules (geosite/geoip via SagerNet rule-sets / Clash geodata). Pin one per user (in the user page) or mark a default. Edits bump the version and propagate on the next subscription refresh.</p></div></div>
      ${toolbar("Search profiles", "profile", "Add profile")}
      ${tableShell(`You have <b>${state.profiles.length}</b> routing profile${state.profiles.length === 1 ? "" : "s"}.`, ["Name", "Version", "Rules", ""], rows, "Create a routing profile to ship rules with subscriptions.")}
    </div>`;
  }

  function renderRuleCoverage() {
    const profiles = new Map(state.profiles.map((profile) => [profile.id, profile]));
    const fallback = state.profiles.find((profile) => profile.is_default);
    const rows = state.users.map((user) => {
      const explicit = profiles.get(user.routing_profile_id);
      const profile = explicit || fallback;
      return `<tr><td class="primary-cell"><button class="cell-link" data-open="users/${user.id}">${esc(user.username)}</button></td>
        <td>${esc(profile?.name || "No profile")}</td><td>${explicit ? '<span class="chip">pinned</span>' : '<span class="chip blue">default</span>'}</td>
        <td>${profile ? `v${esc(profile.version)}` : "—"}</td></tr>`;
    }).join("");
    return `<div class="page">${pageHeader("Rule coverage", "Effective routing profile resolved for every user.")}
      ${tableShell(`<b>${state.users.filter((user) => user.routing_profile_id).length}</b> explicitly pinned users; the rest inherit the default.`, ["User", "Effective profile", "Source", "Version"], rows, "No users are available.")}</div>`;
  }

  function renderRuleDelivery() {
    const rows = [
      ["sing-box", "full", "Native route rules and remote rule-sets"],
      ["sing-box TUN", "full", "Native rules plus system-wide TUN"],
      ["Clash / Mihomo", "full", "Rules, providers and policy groups"],
      ["V2Ray links", "transport only", "Share links do not carry a routing profile"],
      ["Plain links", "transport only", "One outbound link per line"],
    ].map(([format, support, detail]) => `<tr><td class="primary-cell">${esc(format)}</td><td><span class="status ${support === "full" ? "ok" : "warn"}">${esc(support)}</span></td><td>${esc(detail)}</td></tr>`).join("");
    return `<div class="page">${pageHeader("Rule delivery", "Which subscription outputs can carry honey routing profiles.")}
      ${tableShell("Choose a full configuration format when client-side routing policy must be enforced.", ["Format", "Profile support", "Delivery"], rows, "")}</div>`;
  }

  async function deleteProfile(id) {
    const p = state.profiles.find((x) => x.id === id);
    if (!confirm(`Delete routing profile "${p?.name || id}"? Users on it fall back to the default.`)) return;
    try {
      await api(`/routing-profiles/${id}`, { method: "DELETE" });
      toast("profile deleted");
      await loadData({ quiet: true });
    } catch (error) { toast(error.message, true); }
  }

  async function setDefaultProfile(id) {
    try {
      await api(`/routing-profiles/${id}`, { method: "PATCH", body: JSON.stringify({ is_default: true }) });
      toast("default profile set");
      await loadData({ quiet: true });
    } catch (error) { toast(error.message, true); }
  }

  async function assignUserProfile(userId, value) {
    try {
      await api(`/users/${userId}/routing-profile`, { method: "PUT", body: JSON.stringify({ profile_id: value || null }) });
      const u = state.users.find((x) => x.id === userId);
      if (u) u.routing_profile_id = value || null;
      toast("routing profile updated");
    } catch (error) { toast(error.message, true); }
  }

  function wizDefaults() {
    return {
      core: state.settings?.default_inbound_core || "singbox", kind: "vless", node_id: state.nodes[0]?.id || "",
      tag: "", port: "", flow: "xtls-rprx-vision",
      happ_name: "", happ_description: "", country_code: "",
      security: "reality", network: "tcp", transport_path: "", transport_service_name: "", transport_host: "", transport_mode: "",
      utls_fingerprint: "qq",
      cert_source: "acme", acme_email: "", acme_challenge: "http", acme_http_port: "9080",
      server_name: "www.cloudflare.com", cert_path: "", key_path: "",
      reality_handshake_server: "www.cloudflare.com", reality_handshake_port: "443",
      reality_short_ids: "", reality_private_key: "", reality_public_key: "",
      shadowtls_handshake_server: "", shadowtls_handshake_port: "", hop_ports: "",
      fallback_host: "", sni_pool: "", cdn_pool: "", up_mbps: 0, down_mbps: 0, udp_idle_timeout: "60s", upstream_inbound_id: "",
      ss_method: "", ss_plugin: "", ss_plugin_opts: "",
    };
  }
  // build a wizard state from an existing inbound so edit reuses the same
  // fullscreen wizard as create (prefilled). _orig keeps the row for extra-merge.
  function wizFromInbound(ib) {
    const security = ib.reality ? "reality" : ib.tls_enabled ? "tls" : "none";
    const acme = Boolean(ib.extra && ib.extra.acme);
    return {
      editing_id: ib.id, _orig: ib,
      core: ib.core, kind: ib.kind, node_id: ib.node_id,
      tag: ib.tag || "", port: String(ib.listen_port || ""), flow: ib.flow || "",
      happ_name: ib.extra?.happ?.name || "",
      happ_description: ib.extra?.happ?.description || "",
      country_code: ib.extra?.happ?.country_code || "",
      security, network: ib.network || "tcp",
      transport_path: ib.transport_path || "", transport_service_name: ib.transport_service_name || "", transport_host: ib.transport_host || "",
      transport_mode: ib.transport_mode || "",
      utls_fingerprint: ib.utls_fingerprint || "qq",
      cert_source: acme ? "acme" : "manual", acme_email: (acme && ib.extra.acme.email) || "",
      acme_challenge: acme && ib.extra.acme.disable_http_challenge ? "tls-alpn" : "http",
      acme_http_port: (acme && ib.extra.acme.alternative_http_port) || "9080",
      server_name: ib.server_name || "www.cloudflare.com", cert_path: ib.cert_path || "", key_path: ib.key_path || "",
      reality_handshake_server: ib.reality_handshake_server || "www.cloudflare.com", reality_handshake_port: String(ib.reality_handshake_port || "443"),
      reality_short_ids: (ib.reality_short_ids || []).join(","), reality_private_key: "", reality_public_key: ib.reality_public_key || "",
      shadowtls_handshake_server: ib.shadowtls_handshake_server || "", shadowtls_handshake_port: String(ib.shadowtls_handshake_port || ""),
      hop_ports: (ib.extra && ib.extra.hop_ports) || "",
      fallback_host: ib.fallback_host || "", sni_pool: (ib.sni_pool || []).join(", "), cdn_pool: (ib.cdn_pool || []).join(", "),
      ss_method: (ib.extra && ib.extra.method) || "", ss_plugin: (ib.extra && ib.extra.plugin) || "", ss_plugin_opts: (ib.extra && ib.extra.plugin_opts) || "",
      up_mbps: Number(ib.up_mbps || 0), down_mbps: Number(ib.down_mbps || 0), udp_idle_timeout: ib.udp_idle_timeout || "60s",
      upstream_inbound_id: ib.upstream_inbound_id || "",
    };
  }
  function multihopExitOptions(w) {
    const chainable = new Set(["vless", "vmess", "trojan", "hysteria2", "tuic", "shadowsocks"]);
    const exits = (state.inbounds || []).filter((i) => i.id !== w.editing_id && (i.core || "singbox") === "singbox" && chainable.has(i.kind));
    const nodeName = (id) => (state.nodes.find((n) => n.id === id) || {}).name || "node";
    const opts = exits.map((i) => `<option value="${i.id}" ${w.upstream_inbound_id === i.id ? "selected" : ""}>${esc(nodeName(i.node_id))} · ${esc(i.tag)} (${esc(i.kind)})</option>`).join("");
    return `<option value="">— direct egress (no chain) —</option>${opts}`;
  }
  function inboundProtocols(core) {
    return core === "xray"
      ? ["vless", "vmess", "trojan", "shadowsocks"]
      : ["vless", "vmess", "trojan", "hysteria2", "tuic", "shadowsocks", "anytls", "shadowtls"];
  }
  function securitiesFor(kind) {
    switch (kind) {
      case "vless": return ["reality", "tls", "none"];
      case "vmess": case "trojan": return ["tls", "none"];
      case "hysteria2": case "tuic": case "anytls": return ["tls"];
      case "shadowtls": return ["shadowtls"];
      default: return ["none"]; // shadowsocks
    }
  }
  function wizSupportsTransport(kind) { return ["vless", "vmess", "trojan"].includes(kind); }
  function wizNetworks(core, security) {
    if (core === "singbox") return ["tcp", "ws", "grpc", "http", "h2", "httpupgrade", "quic"];
    return security === "reality"
      ? ["tcp", "xhttp", "grpc"]
      : ["tcp", "ws", "grpc", "http", "h2", "httpupgrade", "xhttp", "quic", "mkcp"];
  }
  function wizNormalize(w) {
    const protos = inboundProtocols(w.core);
    if (!protos.includes(w.kind)) w.kind = protos[0];
    const secs = securitiesFor(w.kind);
    if (!secs.includes(w.security)) w.security = secs[0];
    const nets = wizNetworks(w.core, w.security);
    if (!nets.includes(w.network)) w.network = nets[0];
    if (w.kind !== "vless" || w.network !== "tcp" || w.security === "none") w.flow = "";
    if (w.network === "xhttp" && !w.transport_mode) w.transport_mode = "auto";
    if (w.network !== "xhttp") w.transport_mode = "";
  }
  function flowChoices(w) {
    if (w.kind !== "vless") return [""];
    if (w.network === "tcp" && (w.security === "tls" || w.security === "reality")) {
      return ["", "xtls-rprx-vision"];
    }
    return [""];
  }
  function repaintWiz() {
    const m = document.getElementById("wiz-body");
    if (m) m.innerHTML = wizInboundBody(state.wiz);
  }

  function renderNewInbound() {
    if (!state.nodes.length) {
      return `<div class="page narrow">${backLink("inbounds", "Back to inbounds")}<div class="panel empty"><div><div class="empty-icon">${icon("node")}</div><h3>Add a node first</h3><p>Inbounds live on a node — create one, then come back.</p><button class="button primary" data-action="add-node">${icon("plus")} Add node</button></div></div></div>`;
    }
    if (!state.wiz) state.wiz = wizDefaults();
    const editing = Boolean(state.wiz.editing_id);
    return `<div class="page narrow">${backLink("inbounds", "Back to inbounds")}
      ${detailHead(editing ? "Edit inbound" : "New inbound", "", editing ? "Adjust this inbound — the same guided wizard, prefilled." : "Pick a core first — protocols, security and transport adapt to it.", "")}
      <div id="wiz-body">${wizInboundBody(state.wiz)}</div></div>`;
  }

  function wizInboundBody(w) {
    wizNormalize(w);
    const protos = inboundProtocols(w.core);
    const secs = securitiesFor(w.kind);
    const showT = wizSupportsTransport(w.kind);
    const nets = wizNetworks(w.core, w.security);
    const coreNote = w.core === "xray"
      ? "xray: vless / vmess / trojan / shadowsocks — no hysteria2, tuic, anytls or shadowtls."
      : "sing-box: full protocol set incl. hysteria2, tuic, anytls, shadowtls.";
    const domainList = state.domains.length
      ? `<datalist id="wiz-domains">${state.domains.map((d) => `<option value="${esc(d.host)}"></option>`).join("")}</datalist>`
      : "";
    return `
      ${domainList}
      <div class="panel wiz-panel"><div class="panel-title">CDN presets</div><div class="form-body">
        <div class="preset-row">
          <button type="button" class="preset-btn" data-action="preset-cf-ws">${icon("globe")}<span><b>Cloudflare · VLESS + WS</b><small>ws behind CF · ACME TLS · uses CDN hostname</small></span></button>
          <button type="button" class="preset-btn" data-action="preset-cf-xhttp">${icon("globe")}<span><b>Cloudflare · VLESS + xHTTP</b><small>xray xhttp behind CF · manual TLS certificate</small></span></button>
        </div>
        <p class="form-note">Or build it manually below. Registered domains autocomplete in the SNI/CDN-host fields.</p>
      </div></div>

      <div class="panel wiz-panel"><div class="panel-title">1 · Core</div><div class="form-body">
        <div class="seg">${["singbox", "xray"].map((c) => `<button type="button" class="seg-btn ${w.core === c ? "on" : ""}" data-wiz-set="core" data-val="${c}">${c === "singbox" ? "sing-box" : "xray"}</button>`).join("")}</div>
        <p class="form-note">${coreNote}</p>
      </div></div>

      <div class="panel wiz-panel"><div class="panel-title">2 · Protocol &amp; placement</div><div class="form-body">
        <div class="form-row"><label><span>Protocol</span><select data-wiz="kind" data-struct>${protos.map((p) => option(p, p, w.kind)).join("")}</select></label><label><span>Node</span><select data-wiz="node_id">${state.nodes.map((n) => option(n.id, n.name, w.node_id)).join("")}</select></label></div>
        <div class="form-row"><label><span>Tag</span><input data-wiz="tag" value="${esc(w.tag)}" placeholder="vless-in"></label><label><span>Port</span><input data-wiz="port" type="number" min="1" max="65535" value="${esc(w.port)}" placeholder="443"></label></div>
        <div class="section-label">CLIENT DISPLAY (HAPP AND OTHER APPS)</div>
        <div class="form-row"><label>${helpLabel("Config name", "The server name shown in Happ instead of the technical user @ node / tag label. Empty keeps the legacy generated name.")}<input data-wiz="happ_name" maxlength="80" value="${esc(w.happ_name)}" placeholder="Poland · Premium"></label><label>${helpLabel("Country code", "Two-letter ISO country code. Honey turns PL into the 🇵🇱 flag prefix in every generated client config.")}<input data-wiz="country_code" maxlength="2" value="${esc(w.country_code)}" placeholder="PL" autocomplete="off"></label></div>
        <label>${helpLabel("Config description", "Optional Happ subtitle shown below this config name. Happ limits it to 30 characters.")}<input data-wiz="happ_description" maxlength="30" value="${esc(w.happ_description)}" placeholder="VLESS · low latency"></label>
        ${w.kind === "vless" ? `<label><span>Flow</span><select data-wiz="flow">${flowChoices(w).map((v) => option(v, v || "default", w.flow)).join("")}</select><p class="form-note">Vision is available only for VLESS over TCP with TLS or REALITY. Leave default for XHTTP.</p></label>` : ""}
        ${w.kind === "hysteria2" ? `<label>${helpLabel("UDP port hopping (optional)", "A comma-separated list or range of UDP ports advertised to compatible clients. Every selected port must reach this node and be forwarded to the inbound listener.")}<input data-wiz="hop_ports" value="${esc(w.hop_ports)}" placeholder="20000-30000"></label>` : ""}
        ${w.kind === "hysteria2" ? `<div class="form-row"><label>${helpLabel("Down speed cap, Mbps", "Traffic shaping: caps download bandwidth for this hysteria2 inbound (drives its congestion control). 0 = unlimited. Use tiered inbounds as speed plans — vless/vmess/trojan cores have no per-inbound speed limiter.")}<input data-wiz="down_mbps" type="number" min="0" max="100000" value="${esc(w.down_mbps)}" placeholder="0"></label><label>${helpLabel("Up speed cap, Mbps", "Caps upload bandwidth for this hysteria2 inbound. 0 = unlimited.")}<input data-wiz="up_mbps" type="number" min="0" max="100000" value="${esc(w.up_mbps)}" placeholder="0"></label></div><label>${helpLabel("UDP idle timeout", "Native Hysteria2 timeout for an idle UDP session. 60s is the official default; Roblox can benefit from a longer value such as 5m.")}<input data-wiz="udp_idle_timeout" value="${esc(w.udp_idle_timeout || "60s")}" placeholder="60s"></label>` : ""}
        ${w.core !== "xray" ? `<label>${helpLabel("Multihop exit (optional)", "Chain this inbound's traffic through another sing-box inbound (the exit), so users enter here and egress from the exit node — e.g. enter in RU, exit abroad. The entry node dials the exit with a dedicated credential; both must be sing-box.")}<select data-wiz="upstream_inbound_id">${multihopExitOptions(w)}</select></label>` : ""}
        ${w.kind === "shadowsocks" ? `<div class="form-row"><label>${helpLabel("Method (cipher)", "Shadowsocks cipher, e.g. 2022-blake3-aes-256-gcm (SS2022) or aes-256-gcm. Required for the client link.")}<input data-wiz="ss_method" value="${esc(w.ss_method)}" placeholder="2022-blake3-aes-256-gcm"></label><label>${helpLabel("SIP003 plugin (optional)", "sing-box native plugins: obfs-local or v2ray-plugin. External plugins (cloak, kcptun) need the plugin binary on the node and a core that execs SIP003 — not managed by honey.")}<input data-wiz="ss_plugin" value="${esc(w.ss_plugin)}" placeholder="v2ray-plugin"></label></div>
        <label>${helpLabel("Plugin options (optional)", "SIP003 plugin_opts string, e.g. 'tls;host=example.com' for v2ray-plugin or 'obfs=http;obfs-host=example.com' for obfs.")}<input data-wiz="ss_plugin_opts" value="${esc(w.ss_plugin_opts)}" placeholder="tls;host=example.com"></label>` : ""}
      </div></div>

      <div class="panel wiz-panel"><div class="panel-title">3 · Security</div><div class="form-body">
        <div class="seg">${secs.map((s) => `<button type="button" class="seg-btn ${w.security === s ? "on" : ""}" data-wiz-set="security" data-val="${s}">${esc(s)}</button>`).join("")}</div>
        ${wizSecurityFields(w)}
        ${["tls", "reality"].includes(w.security) ? `<label>${helpLabel("uTLS fingerprint", "Client TLS fingerprint advertised in generated subscriptions. qq is the compatibility default; choose another only for a client or network that needs it.")}<select data-wiz="utls_fingerprint">${["chrome", "firefox", "safari", "ios", "android", "edge", "360", "qq", "random", "randomized"].map((v) => option(v, v, w.utls_fingerprint || "qq")).join("")}</select></label>` : ""}
      </div></div>

      ${showT ? `<div class="panel wiz-panel"><div class="panel-title">4 · Transport</div><div class="form-body">
        <div class="form-row"><label>${helpLabel("Network", "The transport framing used above the protocol. Server and client must use the same transport; paths, hosts or service names appear only when that transport needs them.")}<select data-wiz="network" data-struct>${nets.map((n) => option(n, n, w.network)).join("")}</select></label>
        ${["ws", "httpupgrade", "xhttp"].includes(w.network) ? `<label><span>Path</span><input data-wiz="transport_path" value="${esc(w.transport_path)}" placeholder="/honey"></label>` : w.network === "grpc" ? `<label><span>gRPC service</span><input data-wiz="transport_service_name" value="${esc(w.transport_service_name)}" placeholder="honey"></label>` : `<label><span>&nbsp;</span><span class="form-note" style="padding-top:9px">${w.network === "tcp" ? "raw tcp — no extra options" : esc(w.network) + " — defaults used"}</span></label>`}</div>
        ${["ws", "http", "h2", "httpupgrade", "xhttp"].includes(w.network) ? `<div class="form-row"><label>${helpLabel("CDN host (Host header)", "The HTTP Host value used by this transport. When a public CDN hostname is configured, generated subscriptions connect to it instead of the node address.")}<input data-wiz="transport_host" list="wiz-domains" value="${esc(w.transport_host)}" placeholder="cdn.example.com"></label><label><span>&nbsp;</span><span class="form-note" style="padding-top:9px">set to a registered CDN domain — the subscription connects here, not the origin IP</span></label></div>` : ""}
        ${w.network === "xhttp" ? `<label>${helpLabel("xHTTP mode", "Xray's auto mode selects the transport behavior for the client. Change it only when a particular network or client benefits from an explicit mode.")}<select data-wiz="transport_mode" data-struct>${["auto", "packet-up", "stream-up", "stream-one"].map((v) => option(v, v, w.transport_mode || "auto")).join("")}</select><p class="form-note">auto is the default; packet-up and stream modes remain available for compatibility or latency tuning.</p></label>` : ""}
        ${w.core === "xray" && w.security === "reality" ? `<p class="form-note">xray REALITY allows only tcp / xhttp / grpc.</p>` : ""}
      </div></div>` : ""}

      <div class="panel wiz-panel"><div class="panel-title">${showT ? "5" : "4"} · RF-resilience <span class="chip">optional</span></div><div class="form-body">
        <div class="form-row"><label>${helpLabel("Fallback CDN host", "When a vantage checker confirms this endpoint blocked, the subscription fronts it through this CDN host instead of dropping it. Only meaningful for ws/http/xhttp inbounds that also work behind a CDN.")}<input data-wiz="fallback_host" list="wiz-domains" value="${esc(w.fallback_host)}" placeholder="cdn.example.com"></label><label>${helpLabel("SNI pool (comma-separated)", "Owned SNIs to rotate this inbound's server_name through when one is blocked. A vantage-detected block auto-rotates to the next value and re-pushes; you can also rotate manually from the inbound page.")}<input data-wiz="sni_pool" value="${esc(w.sni_pool)}" placeholder="a.example.com, b.example.com"></label></div>
        <div class="form-row"><label>${helpLabel("CDN pool (comma-separated)", "Candidate CDN fronting hosts. When proactive CDN rotation is enabled (Runtime settings), the master measures TCP latency to each and points this inbound's CDN host at the fastest reachable one — rotation 'by ping'. Only meaningful for ws/http/xhttp inbounds behind a CDN.")}<input data-wiz="cdn_pool" value="${esc(w.cdn_pool)}" placeholder="cf.example.com, edge2.example.com"></label><label><span>&nbsp;</span><span class="form-note" style="padding-top:9px">picks the lowest-latency edge automatically</span></label></div>
        <p class="form-note">Auto-failover uses the reachability fleet (vantage reports via <code>PUT /inbounds/:id/reachability</code>).</p>
      </div></div>

      <div class="wiz-actions">
        <button class="button" data-open="inbounds">Cancel</button>
        <button class="button primary" data-action="create-inbound">${icon(w.editing_id ? "check" : "plus")} ${w.editing_id ? "Save inbound" : "Create inbound"}</button>
      </div>
      <p class="field-error" id="wiz-error"></p>`;
  }

  function wizSecurityFields(w) {
    if (w.security === "tls") {
      const src = w.cert_source || "acme";
      const srcSeg = `<div class="seg" style="margin-top:2px">${["acme", "manual"].map((s) => `<button type="button" class="seg-btn ${src === s ? "on" : ""}" data-wiz-set="cert_source" data-val="${s}">${s === "acme" ? "automatic (ACME)" : "manual paths"}</button>`).join("")}</div>`;
      const body = src === "acme"
        ? `<div class="form-row"><label><span>ACME email</span><input data-wiz="acme_email" value="${esc(w.acme_email)}" placeholder="you@example.com"></label><label><span>Challenge</span><select data-wiz="acme_challenge" data-struct>${option("http", "HTTP-01", w.acme_challenge || "http")}${w.core === "xray" ? "" : option("tls-alpn", "TLS-ALPN-01", w.acme_challenge || "http")}</select></label></div>
          ${(w.acme_challenge || "http") === "http"
            ? `<label>${helpLabel("Local HTTP-01 port", "Honey forwards public /.well-known/acme-challenge/ requests from Caddy to its ACME gateway. Xray uses the fixed local gateway port :9080.")}<input data-wiz="acme_http_port" type="number" min="1" max="65535" value="${esc(w.core === "xray" ? "9080" : w.acme_http_port)}" ${w.core === "xray" ? "readonly" : ""}></label><p class="form-note">${w.core === "xray" ? "Honey manages the ACME client and certificate files for Xray; HTTP-01 is the supported challenge." : "HTTP-01 needs public TCP :80 to reach this node."}</p>`
            : `<p class="form-note">TLS-ALPN-01 is available only for sing-box. Xray uses Honey-managed HTTP-01.</p>`}`
        : `<div class="form-row"><label><span>Certificate path</span><input data-wiz="cert_path" value="${esc(w.cert_path)}" placeholder="/etc/letsencrypt/live/…/fullchain.pem"></label><label><span>Key path</span><input data-wiz="key_path" value="${esc(w.key_path)}" placeholder="/…/privkey.pem"></label></div>`;
      return `<div class="form-row"><label>${helpLabel("Server name (SNI / domain)", "The TLS name sent by clients. It must be covered by the configured certificate and resolve through the public path you intend clients to use.")}<input data-wiz="server_name" list="wiz-domains" value="${esc(w.server_name)}" placeholder="vpn.example.com"></label><label><span>Certificate source</span>${srcSeg}</label></div>
        ${body}`;
    }
    if (w.security === "reality") {
      const ready = w.reality_public_key
        ? `<div class="rk-out"><div><span>public key</span><code>${esc(w.reality_public_key)}</code></div><div><span>short id</span><code>${esc(w.reality_short_ids)}</code></div><span class="chip blue">keys ready</span></div>`
        : `<p class="form-note">No keys yet — generate an x25519 keypair (private stays on the master, public + short id ship in client links).</p>`;
      return `<div class="form-row"><label>${helpLabel("Server name (SNI)", "A name accepted by the REALITY target. Generated client profiles send this exact value, so it must match one of the target's usable TLS names.")}<input data-wiz="server_name" value="${esc(w.server_name)}" placeholder="www.cloudflare.com"></label><label>${helpLabel("Handshake / target", "The TLS 1.3 endpoint the node uses for the REALITY handshake. Verify that the node can reach host:port and that the selected SNI is valid for it.")}<input data-wiz="reality_handshake_server" value="${esc(w.reality_handshake_server)}" placeholder="www.cloudflare.com"></label></div>
        <div class="form-row"><label><span>Handshake port</span><input data-wiz="reality_handshake_port" type="number" min="1" max="65535" value="${esc(w.reality_handshake_port)}" placeholder="443"></label><label><span>&nbsp;</span><button type="button" class="button" data-action="gen-reality">${icon("key")} Generate keys</button></label></div>
        ${ready}`;
    }
    if (w.security === "shadowtls") {
      return `<div class="form-row"><label><span>Handshake server</span><input data-wiz="shadowtls_handshake_server" value="${esc(w.shadowtls_handshake_server)}" placeholder="www.apple.com"></label><label><span>Handshake port</span><input data-wiz="shadowtls_handshake_port" type="number" min="1" max="65535" value="${esc(w.shadowtls_handshake_port)}" placeholder="443"></label></div>
        <p class="form-note">ShadowTLS masquerades the handshake as the chosen site; the wrapped Shadowsocks inbound detour still uses extra JSON.</p>`;
    }
    return `<p class="form-note">${w.kind === "shadowsocks" ? "Shadowsocks carries its own encryption — no TLS/REALITY. Set method/password via node defaults or extra JSON." : "Plain — no TLS. Only sensible behind a CDN/tunnel that terminates TLS for you."}</p>`;
  }

  function applyCdnPreset(net) {
    if (!state.wiz) state.wiz = wizDefaults();
    const w = state.wiz;
    w.core = net === "xhttp" ? "xray" : "singbox"; w.kind = "vless"; w.flow = "";
    w.security = "tls"; w.cert_source = "acme";
    w.network = net; w.transport_path = net === "xhttp" ? "/" : "/ws";
    if (!w.port) w.port = "443";
    wizNormalize(w);
    repaintWiz();
    toast(net === "xhttp"
      ? "Cloudflare xHTTP preset applied — set your domain + certificate paths"
      : "Cloudflare WS preset applied — set your domain + ACME email");
  }

  async function genReality() {
    try {
      const k = await api("/reality/keygen", { method: "POST" });
      state.wiz.reality_private_key = k.private_key;
      state.wiz.reality_public_key = k.public_key;
      state.wiz.reality_short_ids = k.short_id;
      repaintWiz();
      toast("reality keys generated");
    } catch (error) {
      toast(error.message, true);
    }
  }

  async function createInboundFromWiz() {
    const w = state.wiz;
    wizNormalize(w);
    const err = document.getElementById("wiz-error");
    if (err) err.textContent = "";
    const tls_enabled = w.security === "tls" || w.security === "reality";
    const reality = w.security === "reality";
    const acme = w.security === "tls" && (w.cert_source || "acme") === "acme";
    const acmeChallenge = w.core === "xray" ? "http" : (w.acme_challenge || "http");
    const showT = wizSupportsTransport(w.kind);
    const body = {
      node_id: w.node_id,
      tag: (w.tag || "").trim(),
      kind: w.kind, core: w.core,
      listen_port: Number(w.port),
      flow: w.kind === "vless" ? (w.flow || "").trim() : "",
      tls_enabled, reality,
      server_name: (tls_enabled ? (w.server_name || "").trim() : "") || null,
      cert_path: (w.security === "tls" && !acme ? (w.cert_path || "").trim() : "") || null,
      key_path: (w.security === "tls" && !acme ? (w.key_path || "").trim() : "") || null,
      reality_public_key: reality ? (w.reality_public_key || "").trim() || null : null,
      reality_short_ids: reality ? (w.reality_short_ids || "").split(",").map((s) => s.trim()).filter(Boolean) : [],
      reality_handshake_server: reality ? (w.reality_handshake_server || "").trim() || null : null,
      reality_handshake_port: reality && w.reality_handshake_port ? Number(w.reality_handshake_port) : null,
      network: showT ? w.network : "tcp",
      transport_path: showT ? (w.transport_path || "").trim() || null : null,
      transport_host: showT ? (w.transport_host || "").trim() || null : null,
      transport_service_name: showT ? (w.transport_service_name || "").trim() || null : null,
      transport_mode: showT && w.network === "xhttp" ? (w.transport_mode || "auto") : null,
      utls_fingerprint: tls_enabled ? (w.utls_fingerprint || "qq") : null,
      shadowtls_handshake_server: w.kind === "shadowtls" ? (w.shadowtls_handshake_server || "").trim() || null : null,
      shadowtls_handshake_port: w.kind === "shadowtls" && w.shadowtls_handshake_port ? Number(w.shadowtls_handshake_port) : null,
      extra: (() => {
        // on edit preserve unmanaged extra keys (custom JSON, utls, etc.); the
        // wizard only owns acme and hop_ports.
        const ex = w.editing_id && w._orig && w._orig.extra ? { ...w._orig.extra } : {};
        if (acme) {
          ex.acme = { email: (w.acme_email || "").trim() };
          if (acmeChallenge === "http") {
            ex.acme.disable_tls_alpn_challenge = true;
            if (w.core === "xray") ex.acme.alternative_http_port = 9080;
            else if (Number(w.acme_http_port || 0) > 0) ex.acme.alternative_http_port = Number(w.acme_http_port);
          } else {
            ex.acme.disable_http_challenge = true;
          }
        } else delete ex.acme;
        const happ = {
          name: (w.happ_name || "").trim(),
          description: (w.happ_description || "").trim(),
          country_code: (w.country_code || "").trim().toUpperCase(),
        };
        if (happ.name || happ.description || happ.country_code) ex.happ = happ; else delete ex.happ;
        if (w.kind === "hysteria2" && (w.hop_ports || "").trim()) ex.hop_ports = (w.hop_ports || "").trim(); else delete ex.hop_ports;
        if (w.kind === "shadowsocks") {
          if ((w.ss_method || "").trim()) ex.method = w.ss_method.trim(); else delete ex.method;
          if ((w.ss_plugin || "").trim()) { ex.plugin = w.ss_plugin.trim(); ex.plugin_opts = (w.ss_plugin_opts || "").trim(); } else { delete ex.plugin; delete ex.plugin_opts; }
        }
        return ex;
      })(),
      fallback_host: (w.fallback_host || "").trim() || null,
      sni_pool: (w.sni_pool || "").split(",").map((s) => s.trim()).filter(Boolean),
      cdn_pool: (w.cdn_pool || "").split(",").map((s) => s.trim()).filter(Boolean),
      up_mbps: w.kind === "hysteria2" ? Math.max(0, Math.round(Number(w.up_mbps || 0))) : 0,
      down_mbps: w.kind === "hysteria2" ? Math.max(0, Math.round(Number(w.down_mbps || 0))) : 0,
      udp_idle_timeout: w.kind === "hysteria2" ? (w.udp_idle_timeout || "60s").trim() : "60s",
      upstream_inbound_id: (w.core !== "xray" && w.upstream_inbound_id) ? w.upstream_inbound_id : null,
    };
    if (reality && w.reality_private_key) body.reality_private_key = w.reality_private_key;
    if (!body.tag) { if (err) err.textContent = "Tag is required."; return; }
    if (!body.listen_port) { if (err) err.textContent = "Port is required."; return; }
    if (reality && !body.reality_public_key) { if (err) err.textContent = "Generate REALITY keys first."; return; }
    if (acme && !body.server_name) { if (err) err.textContent = "A domain (server name) is required for ACME."; return; }
      if (acme && !body.extra.acme.email) { if (err) err.textContent = "An ACME email is required."; return; }
      if (body.extra.happ?.country_code && !/^[A-Z]{2}$/.test(body.extra.happ.country_code)) { if (err) err.textContent = "Country code must be two letters (for example PL)."; return; }
    const editingId = w.editing_id;
    try {
      if (editingId) {
        await api(`/inbounds/${editingId}`, { method: "PATCH", body: JSON.stringify(body) });
        toast("inbound updated");
      } else {
        await api("/inbounds", { method: "POST", body: JSON.stringify(body) });
        toast("inbound added");
      }
      state.wiz = null;
      await loadData({ quiet: true });
      go("inbounds");
    } catch (error) {
      if (err) err.textContent = error.message;
    }
  }

  function userWizDefaults(user = null) {
    return {
      editing_id: user?.id || "",
      username: user?.username || "",
      password: "",
      subscription_title: user?.subscription_title || "",
      subscription_description: user?.subscription_description || "",
      subscription_group: user?.subscription_group || "",
      subscription_traffic_policy: user?.subscription_traffic_policy || "inherit",
      traffic_gb: user ? Number(user.traffic_limit_bytes || 0) / 1024 ** 3 : 0,
      expires_days: user ? expiryDays(user.expires_at) : 0,
      device_limit: user?.device_limit || 0,
      enabled: user?.enabled !== false,
      bulk_prefix: "",
      bulk_count: 1,
    };
  }

  function renderNewUser() {
    const w = state.userWiz || (state.userWiz = userWizDefaults());
    const editing = Boolean(w.editing_id);
    return `<div class="page narrow">${backLink("users", "Back to users")}
      ${detailHead(editing ? "Edit user" : "Create user", "", editing ? "Update credentials and Happ metadata." : "Issue one user or a batch of generated credentials.", "")}
      <div class="panel wiz-panel"><div class="form-body">
        <label><span>Username</span><input id="uw-username" value="${esc(w.username)}" placeholder="alice" ${editing ? "" : "required"}></label>
        ${editing ? "" : `<label><span>Password</span><div class="form-row"><input id="uw-password" type="text" value="${esc(w.password)}" placeholder="click Generate or type your own"><button class="button secondary" data-action="generate-wiz-password">Generate</button></div></label>`}
        <div class="form-row"><label><span>Subscription title</span><input id="uw-title" maxlength="25" value="${esc(w.subscription_title)}" placeholder="global default"></label><label><span>Client group</span><input id="uw-group" maxlength="40" value="${esc(w.subscription_group)}" placeholder="global default"></label></div>
        <label><span>Subscription description</span><textarea id="uw-description" maxlength="200" rows="3" placeholder="Traffic: {TRAFFIC_SPENT} · left: {DAYS_LEFT}">${esc(w.subscription_description)}</textarea></label>
        <div class="form-row"><label><span>Traffic row in clients</span><select id="uw-traffic-policy">${["inherit","auto","always","never"].map((v) => option(v, v, w.subscription_traffic_policy)).join("")}</select></label><label><span>Device limit</span><input id="uw-devices" type="number" min="0" value="${w.device_limit}"></label></div>
        <div class="form-row"><label><span>Traffic limit, GB</span><input id="uw-traffic" type="number" min="0" step=".01" value="${w.traffic_gb}"></label><label><span>Expires in, days</span><input id="uw-expires" type="number" min="0" step="1" value="${w.expires_days}"></label></div>
        ${editing ? `<label><span>Enabled</span><select id="uw-enabled">${option("true","on",String(w.enabled))}${option("false","off",String(w.enabled))}</select></label>` : `
        <div class="section-label">BULK CREATION</div>
        <div class="form-row"><label><span>Prefix</span><input id="uw-prefix" value="${esc(w.bulk_prefix)}" placeholder="poland"></label><label><span>Count</span><input id="uw-count" type="number" min="1" max="100" value="${w.bulk_count}"></label></div>
        <p class="form-note">When count is greater than one, users are generated as prefix-random suffix. Passwords are generated server-side and shown once after creation.</p>`}
        <p class="field-error" id="uw-error"></p>
      </div></div>
      <div class="wiz-actions"><button class="button" data-route="users">Cancel</button><button class="button primary" data-action="save-user-wiz">${editing ? "Save user" : "Create user(s)"}</button></div>
    </div>`;
  }

  function generateWizardPassword() {
    const input = document.getElementById("uw-password");
    if (!input) return;
    const alphabet = "ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789!@#$%";
    const bytes = new Uint8Array(20);
    crypto.getRandomValues(bytes);
    input.value = Array.from(bytes, (b) => alphabet[b % alphabet.length]).join("");
  }

  async function saveUserWizard() {
    const w = state.userWiz || {};
    const err = document.getElementById("uw-error");
    const username = document.getElementById("uw-username")?.value.trim() || "";
    const password = document.getElementById("uw-password")?.value || "";
    const title = document.getElementById("uw-title")?.value.trim() || null;
    const description = document.getElementById("uw-description")?.value.trim() || null;
    const group = document.getElementById("uw-group")?.value.trim() || null;
    const trafficPolicy = document.getElementById("uw-traffic-policy")?.value || "inherit";
    const days = Number(document.getElementById("uw-expires")?.value || 0);
    const traffic = Math.round(Number(document.getElementById("uw-traffic")?.value || 0) * 1024 ** 3);
    const device = Math.max(0, Math.round(Number(document.getElementById("uw-devices")?.value || 0)));
    if (!w.editing_id && !username && !(document.getElementById("uw-prefix")?.value.trim())) { err.textContent = "username or bulk prefix is required"; return; }
    if (!Number.isInteger(days) || days < 0) { err.textContent = "expiry must be a whole number"; return; }
    try {
      if (w.editing_id) {
        await api(`/users/${w.editing_id}`, { method: "PATCH", body: JSON.stringify({ username, subscription_title: title, subscription_description: description, subscription_group: group, subscription_traffic_policy: trafficPolicy, traffic_limit_bytes: traffic, expires_at: days ? new Date(Date.now() + days * 86400000).toISOString() : null, device_limit: device, enabled: document.getElementById("uw-enabled").value === "true" }) });
        toast("user saved");
      } else {
        const count = Math.max(1, Math.min(100, Number(document.getElementById("uw-count")?.value || 1)));
        const prefix = document.getElementById("uw-prefix")?.value.trim() || username;
        const rows = [];
        for (let i = 0; i < count; i++) {
          const suffix = count > 1 ? `-${Math.random().toString(36).slice(2, 8)}` : "";
          const pass = count > 1 || !password ? (() => { generateWizardPassword(); return document.getElementById("uw-password").value; })() : password;
          const result = await api("/users", { method: "POST", body: JSON.stringify({ username: `${prefix}${suffix}`, password: pass, subscription_title: title, subscription_description: description, subscription_group: group, subscription_traffic_policy: trafficPolicy, traffic_limit_bytes: traffic, expires_at: days ? new Date(Date.now() + days * 86400000).toISOString() : null, device_limit: device }) });
          const createdUsername = result.user?.username || `${prefix}${suffix}`;
          rows.push([createdUsername, pass, `${createdUsername}:${pass}`, location.origin + result.subscription_path]);
        }
        showResult("Users created", rows.flatMap((r) => [
          [`${r[0]} · password`, r[1]],
          [`${r[0]} · Hysteria2 auth`, r[2]],
          [`${r[0]} · subscription`, r[3]],
        ]), "users");
      }
      state.userWiz = null;
      await loadData({ quiet: true });
      if (w.editing_id) go("users");
    } catch (error) { if (err) err.textContent = error.message; }
  }

  function renderNewNode() {
    const n = state.nodeEdit || null;
    const editing = Boolean(n);
    return `<div class="page narrow">${backLink("nodes", "Back to nodes")}
      ${detailHead(editing ? "Edit node" : "New node", "", editing ? "Update this node's control-plane settings." : "Register a server, then enroll its agent with honey-enroll.", "")}
      <div class="panel wiz-panel"><div class="form-body">
        <label><span>Name</span><input id="nn-name" placeholder="de-fra-01" autocomplete="off" value="${esc(n?.name || "")}"></label>
        <label><span>Address</span><input id="nn-address" placeholder="203.0.113.10" autocomplete="off" value="${esc(n?.address || "")}"></label>
        <div class="form-row"><label><span>Transport</span><select id="nn-transport">${["serve", "dial", "both"].map((v) => option(v, v, n?.transport || "serve")).join("")}</select></label><label><span>gRPC port</span><input id="nn-port" type="number" min="1" max="65535" value="${esc(n?.grpc_port || 8443)}"></label></div>
        <label><span>TLS server name</span><input id="nn-tls" value="${esc(n?.tls_server_name || "honey-agent")}" autocomplete="off"></label>
        <label><span>Monthly cost (for P&amp;L, 0 = untracked)</span><input id="nn-cost" type="number" min="0" step=".01" value="${n?.monthly_cost_cents ? Number(n.monthly_cost_cents) / 100 : 0}" placeholder="5.00"></label>
        ${editing ? `<label><span>Extra addresses (comma-separated)</span><input id="nn-extra" value="${esc((n.extra_addresses || []).join(", "))}" placeholder="2001:db8::1, cdn.example.com" autocomplete="off"></label>
        <label><span>Enabled</span><select id="nn-enabled">${option("true", "on", String(n.enabled))}${option("false", "off", String(n.enabled))}</select></label>` : ""}
        <p class="form-note">serve = master dials the node; dial = node dials the master (NAT). ${editing ? "Changing the address re-points the control plane." : "After creating, open the node to issue an enrollment token."}</p>
      </div></div>
      <div class="wiz-actions"><button class="button" data-open="nodes">Cancel</button><button class="button primary" data-action="create-node">${icon(editing ? "check" : "plus")} ${editing ? "Save node" : "Create node"}</button></div>
      <p class="field-error" id="wiz-error"></p>`;
  }

  async function createNodeFromWiz() {
    const err = document.getElementById("wiz-error");
    if (err) err.textContent = "";
    const editing = state.nodeEdit || null;
    const body = {
      name: (document.getElementById("nn-name").value || "").trim(),
      address: (document.getElementById("nn-address").value || "").trim(),
      transport: document.getElementById("nn-transport").value,
      grpc_port: Number(document.getElementById("nn-port").value),
      tls_server_name: (document.getElementById("nn-tls").value || "").trim(),
      monthly_cost_cents: Math.round(Number(document.getElementById("nn-cost")?.value || 0) * 100),
    };
    if (!body.name || !body.address) { if (err) err.textContent = "Name and address are required."; return; }
    if (editing) {
      body.enabled = document.getElementById("nn-enabled").value === "true";
      body.extra_addresses = (document.getElementById("nn-extra").value || "").split(",").map((s) => s.trim()).filter(Boolean);
    }
    try {
      if (editing) {
        await api(`/nodes/${editing.id}`, { method: "PATCH", body: JSON.stringify(body) });
        toast("node updated");
      } else {
        await api("/nodes", { method: "POST", body: JSON.stringify(body) });
        toast("node added");
      }
      state.nodeEdit = null;
      await loadData({ quiet: true });
      go("nodes");
    } catch (error) {
      if (err) err.textContent = error.message;
    }
  }

  async function manage2fa() {
    try {
      const { enabled } = await api("/auth/totp");
      if (enabled) {
        const recovery = await api("/auth/totp/recovery");
        showList("Two-factor", `<div class="form-body"><p class="form-note">Two-factor is <b>on</b>. Enter a current code to turn it off or rotate your recovery codes.</p><label><span>Current code</span><input id="totp-code" inputmode="numeric" placeholder="123456" maxlength="8"></label><div class="recovery-box"><div><b>Recovery codes</b><small>${recovery.remaining} unused · each code works once</small></div><button class="button secondary" data-action="generate-recovery">Generate new</button></div><button class="button danger" data-action="totp-disable">Disable 2FA</button><p class="field-error" id="totp-err"></p></div>`);
      } else {
        const s = await api("/auth/totp/setup", { method: "POST" });
        showList("Enable two-factor", `<div class="form-body"><p class="form-note">Scan with an authenticator app (or type the secret), then confirm a code.</p><div class="totp-qr">${s.qr_svg}</div><code class="code-value" style="word-break:break-all">${esc(s.secret)}</code><label><span>Code from app</span><input id="totp-code" inputmode="numeric" placeholder="123456" maxlength="8"></label><button class="button primary" data-action="totp-enable">Enable 2FA</button><p class="field-error" id="totp-err"></p></div>`);
      }
    } catch (error) { toast(error.message, true); }
  }
  async function totpEnable() {
    const code = document.getElementById("totp-code")?.value.trim() || "";
    try { await api("/auth/totp/enable", { method: "POST", body: JSON.stringify({ code }) }); formDialog.close(); toast("two-factor enabled"); }
    catch (error) { const el = document.getElementById("totp-err"); if (el) el.textContent = error.message; }
  }
  async function totpDisable() {
    const code = document.getElementById("totp-code")?.value.trim() || "";
    try { await api("/auth/totp/disable", { method: "POST", body: JSON.stringify({ code }) }); formDialog.close(); toast("two-factor disabled"); }
    catch (error) { const el = document.getElementById("totp-err"); if (el) el.textContent = error.message; }
  }
  async function generateRecoveryCodes() {
    const code = document.getElementById("totp-code")?.value.trim() || "";
    try {
      const result = await api("/auth/totp/recovery/generate", { method: "POST", body: JSON.stringify({ code }) });
      const joined = result.codes.join("\n");
      showList("Save recovery codes", `<div class="form-body"><p class="form-note"><b>Save these now.</b> They are shown once and the previous set is invalidated.</p><textarea class="recovery-codes" readonly>${esc(joined)}</textarea><button type="button" class="button secondary" data-copy="${esc(joined)}">${icon("copy")} Copy all</button><p class="form-note">Use one code on the sign-in screen if your authenticator is unavailable.</p></div>`);
    } catch (error) { const el = document.getElementById("totp-err"); if (el) el.textContent = error.message; }
  }
  async function manageIps() {
    try {
      const ips = await api("/admin-ips");
      const rows = ips.length
        ? ips.map((i) => `<div class="check-row"><span><b>${esc(i.cidr)}</b><small>${esc(i.note || "")}</small></span><button class="row-button danger" data-action="del-ip" data-id="${i.id}">remove</button></div>`).join("")
        : '<div class="check-row"><span><small>no restrictions — open from anywhere</small></span></div>';
      showList("Admin IP allowlist", `<div class="form-body"><p class="form-note">Empty = allow from anywhere. Add your own address before relying on it, or you can lock yourself out.</p><div class="check-list">${rows}</div><label><span>Add IP or CIDR</span><input id="new-ip" placeholder="203.0.113.4 or 10.0.0.0/24"></label><input id="new-ip-note" placeholder="note (optional)" style="margin-top:8px"><button class="button" data-action="add-ip" style="margin-top:10px">Add</button><p class="field-error" id="ip-err"></p></div>`);
    } catch (error) { toast(error.message, true); }
  }
  async function addIp() {
    const cidr = document.getElementById("new-ip")?.value.trim();
    if (!cidr) return;
    const note = document.getElementById("new-ip-note")?.value.trim() || "";
    try { await api("/admin-ips", { method: "POST", body: JSON.stringify({ cidr, note }) }); manageIps(); }
    catch (error) { const el = document.getElementById("ip-err"); if (el) el.textContent = error.message; }
  }
  async function delIp(id) {
    try { await api(`/admin-ips/${id}`, { method: "DELETE" }); manageIps(); }
    catch (error) { toast(error.message, true); }
  }
  async function assignUserQuota(userId, interval) {
    try {
      await api(`/users/${userId}/quota-interval`, { method: "PUT", body: JSON.stringify({ interval }) });
      const u = state.users.find((x) => x.id === userId);
      if (u) u.quota_interval = interval;
      toast("quota window updated");
    } catch (error) { toast(error.message, true); }
  }

  async function manageNotifications() {
    try {
      const chans = await api("/notify-channels");
      const rows = chans.length
        ? chans.map((c) => `<div class="check-row"><span><b>${esc(c.name)} · ${esc(c.kind)}</b><small>${(c.events || []).join(", ") || "all events"} · ${c.enabled ? "on" : "off"}</small></span><span style="display:flex;gap:6px"><button class="row-button" data-action="test-channel" data-id="${c.id}">test</button><button class="row-button danger" data-action="del-channel" data-id="${c.id}">remove</button></span></div>`).join("")
        : '<div class="check-row"><span><small>no channels — alerts go nowhere</small></span></div>';
      showList("Notification channels", `<div class="form-body"><p class="form-note">Alerts (node down, cert expiry, quota reset, push fail) are fanned out here. Target formats: Telegram <code>bot_token@chat_id</code> · Email <code>resend|key|from|to</code> or <code>mailgun|domain|key|from|to</code> · SMS <code>twilio|sid|token|from|to</code> · Alertmanager base URL.</p><div class="check-list">${rows}</div>
        <div class="form-row"><label><span>Name</span><input id="nc-name" placeholder="ops discord"></label><label><span>Kind</span><select id="nc-kind">${["discord", "slack", "telegram", "webhook", "email", "sms", "alertmanager"].map((k) => `<option value="${k}">${k}</option>`).join("")}</select></label></div>
        <label><span>Target</span><input id="nc-target" placeholder="see formats above"></label>
        <button class="button" data-action="add-channel" style="margin-top:10px">Add channel</button><p class="field-error" id="nc-err"></p></div>`);
    } catch (error) { toast(error.message, true); }
  }
  async function addChannel() {
    const name = document.getElementById("nc-name")?.value.trim();
    const kind = document.getElementById("nc-kind")?.value;
    const target = document.getElementById("nc-target")?.value.trim();
    if (!name || !target) return;
    try { await api("/notify-channels", { method: "POST", body: JSON.stringify({ name, kind, target, events: [] }) }); manageNotifications(); }
    catch (error) { const el = document.getElementById("nc-err"); if (el) el.textContent = error.message; }
  }
  async function testChannel(id) { try { await api(`/notify-channels/${id}/test`, { method: "POST" }); toast("test sent"); } catch (error) { toast(error.message, true); } }
  async function delChannel(id) { try { await api(`/notify-channels/${id}`, { method: "DELETE" }); manageNotifications(); } catch (error) { toast(error.message, true); } }

  async function manageTelegram() {
    try {
      const chats = await api("/telegram-chats");
      const rows = chats.length
        ? chats.map((c) => `<div class="check-row"><span><b>${c.chat_id} · ${esc(c.role)}</b><small>${esc(c.note || "")}</small></span><button class="row-button danger" data-action="del-tgchat" data-id="${c.chat_id}">remove</button></div>`).join("")
        : '<div class="check-row"><span><small>no allowlisted chats</small></span></div>';
      showList("Telegram bot", `<div class="form-body"><p class="form-note">Set <code>HONEY_TELEGRAM_TOKEN</code> (+ <code>HONEY_PUBLIC_URL</code>) on the master to run the bot. Admin chats can run <code>/status /nodes /find</code>; anyone can <code>/sub &lt;token&gt;</code>.</p><div class="check-list">${rows}</div>
        <div class="form-row"><label><span>Chat ID</span><input id="tg-id" inputmode="numeric" placeholder="123456789"></label><label><span>Role</span><select id="tg-role"><option value="admin">admin</option><option value="user">user</option></select></label></div>
        <input id="tg-note" placeholder="note (optional)"><button class="button" data-action="add-tgchat" style="margin-top:10px">Add chat</button><p class="field-error" id="tg-err"></p></div>`);
    } catch (error) { toast(error.message, true); }
  }
  async function addTgChat() {
    const id = Number(document.getElementById("tg-id")?.value.trim());
    const role = document.getElementById("tg-role")?.value;
    const note = document.getElementById("tg-note")?.value.trim() || "";
    if (!id) return;
    try { await api("/telegram-chats", { method: "POST", body: JSON.stringify({ chat_id: id, role, note }) }); manageTelegram(); }
    catch (error) { const el = document.getElementById("tg-err"); if (el) el.textContent = error.message; }
  }
  async function delTgChat(id) { try { await api(`/telegram-chats/${id}`, { method: "DELETE" }); manageTelegram(); } catch (error) { toast(error.message, true); } }

  function renderAuditPage() {
    return `<div class="page narrow">${pageHeader("Audit log", "Append-only operator mutations, separate from runtime and agent logs.", '<button class="button primary" data-action="show-audit">Open audit records</button>')}
      <div class="panel settings-grid">
        <div class="setting-row"><div class="setting-copy"><b>Actors</b><span>Records include the administrator or API identity responsible for a mutation.</span></div><span class="chip">attributed</span></div>
        <div class="setting-row"><div class="setting-copy"><b>Resources</b><span>Node, inbound, user, domain, rule and settings changes retain their resource identity.</span></div><span class="chip">append only</span></div>
        <div class="setting-row"><div class="setting-copy"><b>Runtime diagnostics</b><span>Crashes, core output and reconcile messages belong in Runtime, not Audit.</span></div><button class="button secondary" data-route="logs" data-section="runtime">Open runtime</button></div>
      </div></div>`;
  }

  function renderScheduledPage() {
    return `<div class="page narrow">${pageHeader("Scheduled operations", "Deferred node, inbound and user mutations waiting for their execution time.", '<button class="button primary" data-action="show-scheduled">View operation queue</button>')}
      <div class="panel settings-grid">
        <div class="setting-row"><div class="setting-copy"><b>Supported operations</b><span>Enable, disable, push and rotate actions can be deferred from their resource page.</span></div><span class="chip">resource scoped</span></div>
        <div class="setting-row"><div class="setting-copy"><b>Execution</b><span>The elected master leader executes due operations and records their result.</span></div><span class="chip">HA safe</span></div>
        <div class="setting-row"><div class="setting-copy"><b>Audit trail</b><span>Completed mutations appear in the append-only audit log.</span></div><button class="button secondary" data-route="logs" data-section="audit">Open audit</button></div>
      </div></div>`;
  }

  function renderSettings() {
    const section = categoryFor("settings").some((item) => item.key === state.categorySection) ? state.categorySection : "general";
    const basePath = new URL(".", location.href).pathname.replace(/\/$/, "");
    const owner = state.admin?.role === "owner";
    const pages = {
      general: {
        title: "General settings",
        description: "Master identity, runtime behaviour and public control-plane facts.",
        rows: `
          <div class="setting-row"><div class="setting-copy"><b>Panel origin</b><span>Allowed by the panel_domains Host + path allowlist.</span></div><code class="code-value">${esc(location.origin + basePath)}</code></div>
          <div class="setting-row"><div class="setting-copy"><b>Signed in</b><span>HttpOnly session · ${esc(state.admin?.role || "unknown")} role.</span></div><button class="button secondary" data-action="logout">${icon("key")} Sign out ${esc(state.admin?.username || "")}</button></div>
          ${owner ? `<div class="setting-row"><div class="setting-copy"><b>Runtime settings</b><span>Reconcile cadence, retention, inbound defaults and public subscription protection — edited live.</span></div><button class="button secondary" data-action="manage-settings">${icon("settings")} Configure</button></div>
          <div class="setting-row"><div class="setting-copy"><b>Subscription appearance</b><span>Global title, description, client group, traffic row and compatibility profiles.</span></div><button class="button secondary" data-action="manage-settings">${icon("link")} Edit subscription</button></div>` : ""}
          <div class="setting-row"><div class="setting-copy"><b>Subscription guard</b><span>${state.settings?.subscription_guard_enabled ? "On" : "Off"} · ${Number(state.settings?.subscription_guard_recent_blocks || 0)} recently blocked.</span></div><span class="status ${state.settings?.subscription_guard_enabled ? "ok" : "warn"}">${state.settings?.subscription_guard_enabled ? "protected" : "disabled"}</span></div>
          <div class="setting-row"><div class="setting-copy"><b>Announcements</b><span>Broadcast a banner on subscription pages and the public status page.</span></div><button class="button secondary" data-action="manage-announcements">${icon("globe")} Manage</button></div>`,
      },
      automation: {
        title: "Automation",
        description: "Control when desired configuration is applied to connected nodes.",
        rows: `
          <div class="setting-row"><div class="setting-copy"><b>Auto-push</b><span>${state.settings?.auto_push_enabled !== false ? "Changes are applied automatically and reconcile repairs drift." : "Changes remain pending until an operator pushes them manually."}</span></div><span class="status ${state.settings?.auto_push_enabled !== false ? "ok" : "warn"}">${state.settings?.auto_push_enabled !== false ? "enabled" : "manual"}</span></div>
          ${owner ? `<div class="setting-row"><div class="setting-copy"><b>Push policy</b><span>Controls mutation pushes, quota updates, scheduled changes, CDN rotation and reconcile convergence. Manual node push always remains available.</span></div><button class="button secondary" data-action="manage-settings">${icon("settings")} Configure auto-push</button></div>` : ""}
          <div class="setting-row"><div class="setting-copy"><b>Reconcile interval</b><span>When Auto-push is enabled, drift is checked every ${Number(state.settings?.reconcile_secs || 30)} seconds.</span></div><span class="chip">${Number(state.settings?.reconcile_secs || 30)}s</span></div>
          <div class="setting-row"><div class="setting-copy"><b>Manual fallback</b><span>Use Push on a node at any time, including while Auto-push is disabled.</span></div><button class="button secondary" data-route="nodes" data-section="management">${icon("node")} Open nodes</button></div>`,
      },
      security: {
        title: "Security",
        description: "Administrator identity, session boundaries and secret protection.",
        rows: `
          <div class="setting-row"><div class="setting-copy"><b>Sessions & login history</b><span>Review devices and addresses, revoke access, and inspect recent sign-in outcomes.</span></div><button class="button secondary" data-action="manage-sessions">Manage sessions</button></div>
          <div class="setting-row"><div class="setting-copy"><b>Administrators</b><span>Accounts, roles and active state.</span></div><button class="button secondary" data-action="show-admins">Manage admins</button></div>
          ${owner ? `<div class="setting-row"><div class="setting-copy"><b>API keys</b><span>Named, scoped bearer keys for automation and integrations.</span></div><button class="button secondary" data-action="manage-api-keys">${icon("key")} Manage keys</button></div>
          <div class="setting-row"><div class="setting-copy"><b>Secrets & encryption</b><span>XChaCha20-Poly1305 at rest. Key source: <b>${esc(secretBackendLabel(state.settings?.secret_backend))}</b>.</span></div><span class="status ${state.settings?.secret_encryption_enabled ? "ok" : "bad"}">${state.settings?.secret_encryption_enabled ? "encrypted" : "plaintext (dev)"}</span></div>` : ""}
          <div class="setting-row"><div class="setting-copy"><b>Two-factor auth</b><span>TOTP second factor for your administrator login.</span></div><button class="button secondary" data-action="manage-2fa">${icon("lock")} Manage 2FA</button></div>
          <div class="setting-row"><div class="setting-copy"><b>IP allowlist</b><span>Restrict administrator access to explicit addresses; empty means open.</span></div><button class="button secondary" data-action="manage-ips">Manage IPs</button></div>
          ${owner ? `<div class="setting-row"><div class="setting-copy"><b>Custom roles (RBAC)</b><span>Granular read/write permissions that override fixed rank roles.</span></div><button class="button secondary" data-action="manage-custom-roles">${icon("lock")} Roles</button></div>` : ""}`,
      },
      integrations: {
        title: "Integrations",
        description: "Outbound notifications, automation and migration surfaces.",
        rows: `
          <div class="setting-row"><div class="setting-copy"><b>Notifications</b><span>Fan panel alerts out to Webhook, Discord, Slack or Telegram.</span></div><button class="button secondary" data-action="manage-notifications">Channels</button></div>
          <div class="setting-row"><div class="setting-copy"><b>Telegram bot</b><span>Allowlist chats for administrator operations and user self-service.</span></div><button class="button secondary" data-action="manage-telegram">Telegram</button></div>
          ${state.admin?.role !== "reseller" ? `<div class="setting-row"><div class="setting-copy"><b>Import users</b><span>Bulk-import users from Marzban, x-ui or a generic JSON export.</span></div><button class="button secondary" data-action="manage-import">${icon("users")} Import</button></div>` : ""}
          ${owner ? `<div class="setting-row"><div class="setting-copy"><b>Config as code (GitOps)</b><span>Export declarative fleet config and apply it to converge.</span></div><button class="button secondary" data-action="manage-gitops">${icon("rules")} Config</button></div>` : ""}
          <div class="setting-row"><div class="setting-copy"><b>OpenAPI</b><span>Machine-readable API schema for custom integrations.</span></div><a class="button secondary" href="/openapi.json" target="_blank" rel="noopener">Open schema</a></div>`,
      },
      software: {
        title: "Software & operations",
        description: "Master lifecycle, leader state and deferred operations.",
        rows: `
          <div class="setting-row"><div class="setting-copy"><b>High availability</b><span>Instances sharing this database and the current background-loop leader.</span></div><button class="button secondary" data-action="show-ha">${icon("groups")} View instances</button></div>
          ${owner ? `<div class="setting-row"><div class="setting-copy"><b>Software update</b><span>Check GitHub and install a SHA-256 verified master build when enabled.</span></div><button class="button secondary" data-action="show-update">${icon("refresh")} Check for updates</button></div>` : ""}
          <div class="setting-row"><div class="setting-copy"><b>Audit log</b><span>Append-only record of operator mutations.</span></div><button class="button secondary" data-route="logs" data-section="audit">Open audit</button></div>
          <div class="setting-row"><div class="setting-copy"><b>Scheduled operations</b><span>Deferred enable, disable, push and rotate work.</span></div><button class="button secondary" data-route="logs" data-section="scheduled">View queue</button></div>
          <div class="setting-row"><div class="setting-copy"><b>Domain CLI</b><span>Add another host and path without rebuilding honey.</span></div><code class="code-value">honey-master domain add panel.example.com/honey</code></div>`,
      },
      appearance: {
        title: "Appearance",
        description: "Language, colour mode, branding and the self-hosted typeface.",
        rows: `
          <div class="setting-row"><div class="setting-copy"><b>Interface language</b><span>English is used by default until you explicitly choose another language.</span></div><select class="settings-lang-select" data-language-select aria-label="Interface language"><option value="en" ${lang === "en" ? "selected" : ""}>English</option><option value="ru" ${lang === "ru" ? "selected" : ""}>Русский</option></select></div>
          <div class="setting-row"><div class="setting-copy"><b>Theme</b><span>High-contrast monochrome dark or light mode.</span></div><button class="button secondary" data-action="theme">${icon("sun")} Switch theme</button></div>
          ${owner ? `<div class="setting-row"><div class="setting-copy"><b>White-label branding</b><span>Brand name, accent colour, logo and subscription-page presentation.</span></div><button class="button secondary" data-action="manage-branding">${icon("settings")} Customize</button></div>` : ""}
          <div class="setting-row"><div class="setting-copy"><b>Pretendard</b><span>Self-hosted variable font under SIL Open Font License 1.1.</span></div><a class="button secondary" href="./assets/PRETENDARD-LICENSE.txt" target="_blank" rel="noopener">View license</a></div>`,
      },
    };
    const page = pages[section] || pages.general;
    return `<div class="page narrow">${pageHeader(page.title, page.description)}<div class="panel settings-grid">${page.rows}</div></div>`;
  }


  function renderLocked() {
    document.body.classList.add("auth-locked");
    view.innerHTML = `<section class="auth-screen">
      <aside class="auth-intro"><div class="auth-brand"><span class="brand-mark" aria-hidden="true"><i></i><i></i><i></i></span><b>honey</b></div><span class="eyebrow">most universal panel for business/local projects</span><h1>one control plane<br>for every node.</h1><p>Run business and local projects from a focused control surface for sing-box, Xray and subscriptions.</p><ul class="auth-features"><li>multi-node operations</li><li>users, quotas and subscriptions</li><li>safe config pushes with history</li></ul><small class="auth-build">honey master · secure by default</small></aside>
      <form class="auth-card" id="token-form">
        <header><span class="eyebrow">who r u?</span><h2>Admin access</h2><p>Sign in with an administrator created through <code>honey-master admin add</code>.</p></header>
        <div class="form-body">
          <label><span>Username</span><input name="username" autocomplete="username" placeholder="admin" required spellcheck="false"></label>
          <label><span>Password</span><input type="password" name="password" autocomplete="current-password" placeholder="your password" required></label>
          <label id="totp-row" hidden><span>Two-factor code</span><input name="totp_code" inputmode="numeric" autocomplete="one-time-code" placeholder="123456" maxlength="8"></label>
          <label id="recovery-row" hidden><span>Recovery code</span><input name="recovery_code" autocomplete="off" placeholder="20-character code"></label>
          <button type="button" class="auth-link" id="recovery-toggle" hidden>Use a recovery code instead</button>
          <p class="field-error" id="token-error"></p>
          <button class="button primary auth-submit" type="submit">Sign in</button>
        </div>
        <footer>Sessions expire after 12 hours.</footer>
      </form>
    </section>`;
    requestAnimationFrame(() => $("#token-form")?.elements.username.focus());
  }

  function renderError(message) {
    view.innerHTML = `<div class="page narrow">${pageHeader("Could not load honey", "Master returned an error.")}<div class="panel empty"><div><div class="empty-icon">${icon("refresh")}</div><h3>Something is off</h3><p>${esc(message)}</p><button class="button primary" data-action="refresh">Try again</button></div></div></div>`;
  }

  function buildCommandEntries() {
    const reseller = state.admin?.role === "reseller";
    const canAdmin = ["owner", "admin"].includes(state.admin?.role);
    const canOwner = state.admin?.role === "owner";
    const resellerPages = new Set(["overview", "users", "traffic", "live", "subscriptions"]);
    const pages = [
      ["Issues", "Fleet health and actionable warnings", "issues", "issues", "health warnings alerts problems"],
      ["Overview", "Live snapshot and quick actions", "overview", "home", "dashboard главная"],
      ["Nodes", "Agents and server health", "nodes", "node", "servers agents ноды сервера"],
      ["Inbounds", "Protocols and listening ports", "inbounds", "inbound", "protocols sing-box xray входящие"],
      ["Users", "Credentials, quotas and expiry", "users", "users", "clients subscribers пользователи"],
      ["Traffic", "Usage and quota state", "traffic", "chart", "usage analytics трафик"],
      ["Live connections", "Who is online right now", "live", "chart", "live connections online devices онлайн соединения"],
      ["Subscriptions", "Client links and configs", "subscriptions", "link", "links configs подписки"],
      ["Domains", "Owned-domain registry & verify", "domains", "globe", "domains cdn cloudflare домены"],
      ["SSL/TLS", "Certificates and transport security", "ssltls", "lock", "ssl tls certs certificates reality сертификаты"],
      ["Rules", "Routing rule sets for clients", "rules", "rules", "routing rules geosite маршрутизация правила"],
      ["Logs", "Audit of operator actions", "logs", "chart", "audit logs логи аудит"],
      ["Settings", "Panel domain, token and theme", "settings", "settings", "domain auth настройки"],
    ].map(([label, detail, route, iconName, keywords]) => ({ type: "Pages", label, detail, route, icon: iconName, keywords }))
      .filter((entry) => !reseller || resellerPages.has(entry.route));
    const actions = [
      { type: "Actions", label: "Continue setup", detail: "Open the first-run checklist", route: "overview", icon: "check", keywords: "onboarding first run setup начать настройка" },
      { type: "Actions", label: "Add node", detail: "Connect another agent", action: "add-node", icon: "plus", keywords: "server нода добавить", visible: canAdmin },
      { type: "Actions", label: "Add user", detail: "Create credentials", action: "add-user", icon: "plus", keywords: "client subscriber пользователь добавить", visible: canAdmin || reseller },
      { type: "Actions", label: "Add inbound", detail: "Open a protocol listener", action: "add-inbound", icon: "plus", keywords: "protocol port входящий добавить", visible: canAdmin },
      { type: "Actions", label: "Add domain", detail: "Register an owned hostname", action: "add-domain", icon: "globe", keywords: "domain cdn hostname домен добавить", visible: canAdmin },
      { type: "Actions", label: "Add group", detail: "Create an access group", action: "add-group", icon: "groups", keywords: "access node group группа добавить", visible: canAdmin },
      { type: "Actions", label: "Add routing profile", detail: "Create subscription routing rules", action: "add-profile", icon: "rules", keywords: "routing profile rule маршрутизация", visible: canAdmin },
      { type: "Actions", label: "Refresh everything", detail: "Read master state again", action: "refresh", icon: "refresh", keywords: "reload обновить" },
      { type: "System", label: "Switch theme", detail: "Dark or light", action: "theme", icon: "sun", keywords: "dark light тема" },
      { type: "System", label: "Manage admins", detail: "Accounts and roles", action: "show-admins", icon: "key", keywords: "admins админы roles", visible: canOwner },
      { type: "System", label: "Runtime settings", detail: "Live master defaults and intervals", action: "manage-settings", icon: "settings", keywords: "runtime settings defaults интервалы", visible: canOwner },
      { type: "System", label: "Manage API keys", detail: "Scoped bearer keys and OpenAPI", action: "manage-api-keys", icon: "key", keywords: "api keys bearer openapi integration ci ключи", visible: canOwner },
      { type: "System", label: "Search runtime logs", detail: "Filter by level, code, message or request ID", action: "open-log-search", icon: "chart", keywords: "logs search filter request id M0406 журнал поиск код запрос" },
      { type: "System", label: "Sessions & login history", detail: "Devices, IPs and revocation", action: "manage-sessions", icon: "lock", keywords: "sessions login history devices ip revoke сессии входы" },
      { type: "System", label: "Open notifications", detail: "Recent operational alerts", action: "toggle-notifications", icon: "bell", keywords: "alerts unread bell уведомления алерты", visible: !reseller },
      { type: "System", label: "Sign out", detail: "End this session", action: "logout", icon: "key", keywords: "logout выход" },
    ].filter((entry) => entry.visible !== false);
    const entities = [
      ...state.nodes.map((node) => ({ type: "Nodes", label: node.name, detail: node.address, open: `nodes/${node.id}`, icon: "node", keywords: node.transport })),
      ...state.users.map((user) => ({ type: "Users", label: user.username, detail: user.active ? "active" : user.suppressed_reason, open: `users/${user.id}`, icon: "users", keywords: user.uuid })),
      ...state.inbounds.map((inbound) => ({ type: "Inbounds", label: inbound.tag, detail: `${inbound.kind} · ${inbound.listen_port}`, open: `inbounds/${inbound.id}`, icon: "inbound", keywords: inbound.core })),
    ];
    state.commandEntries = [...pages, ...actions, ...entities];
  }

  function openCommand(query = "") {
    if (!state.admin) return openLogin();
    if (!commandDialog.open) commandDialog.showModal();
    commandInput.value = query;
    state.commandIndex = 0;
    renderCommandResults();
    requestAnimationFrame(() => commandInput.focus());
  }

  function renderCommandResults() {
    const query = commandInput.value.trim().toLowerCase();
    const matches = state.commandEntries.filter((entry) =>
      !query || [entry.label, entry.detail, entry.keywords, entry.type].join(" ").toLowerCase().includes(query)
    ).slice(0, 18);
    state.commandMatches = matches;
    state.commandIndex = Math.min(state.commandIndex, Math.max(matches.length - 1, 0));
    if (!matches.length) {
      commandResults.innerHTML = '<div class="empty" style="min-height:120px"><div><h3>No matches</h3><p>Try a page, node name, user or action.</p></div></div>';
      return;
    }
    let group = "";
    commandResults.innerHTML = matches.map((entry, index) => {
      const heading = entry.type !== group ? `<div class="command-group-label">${esc(t(entry.type))}</div>` : "";
      group = entry.type;
      return `${heading}<button class="command-result ${index === state.commandIndex ? "selected" : ""}" data-command-index="${index}">${icon(entry.icon)}<span><b>${esc(entry.label)}</b><small>${esc(entry.detail)}</small></span>${entry.route || entry.open ? icon("chevron") : "<kbd>run</kbd>"}</button>`;
    }).join("");
    $(".command-result.selected", commandResults)?.scrollIntoView({ block: "nearest" });
  }

  function runCommand(index) {
    const entry = state.commandMatches[index];
    if (!entry) return;
    commandDialog.close();
    if (entry.open) { const [route, id] = entry.open.split("/"); go(route, id || undefined); }
    else if (entry.route) go(entry.route);
    else runAction(entry.action);
  }

  function openEntity(kind, entity = null) {
    if (kind === "inbound" && !state.nodes.length) {
      toast("add a node before creating an inbound", true);
      return;
    }
    const editing = Boolean(entity);
    $("#form-eyebrow").textContent = editing ? "edit" : "add";
    $("#form-title").textContent = `${editing ? "Edit" : "Add"} ${kind}`;
    $("#form-submit").textContent = editing ? "Save" : "Add";
    entityForm.dataset.kind = kind;
    entityForm.dataset.id = entity?.id || "";
    $("#form-body").innerHTML = formFields(kind, entity);
    formDialog.showModal();
    requestAnimationFrame(() => $("input, select", $("#form-body"))?.focus());
  }

  function option(value, label, current) {
    return `<option value="${esc(value)}" ${String(value) === String(current) ? "selected" : ""}>${esc(label)}</option>`;
  }

  function formFields(kind, entity = null) {
    const editing = Boolean(entity);
    if (kind === "node") return `
      <label><span>Name</span><input name="name" value="${esc(entity?.name || "")}" required placeholder="de-fra-01" autocomplete="off"></label>
      <label><span>Address</span><input name="address" value="${esc(entity?.address || "")}" required placeholder="203.0.113.10" autocomplete="off"></label>
      <div class="form-row"><label><span>Transport</span><select name="transport">${["serve","dial","both"].map((v) => option(v, v, entity?.transport || "serve")).join("")}</select></label><label><span>gRPC port</span><input name="grpc_port" type="number" min="1" max="65535" value="${esc(entity?.grpc_port || 8443)}" required></label></div>
      <label><span>TLS server name</span><input name="tls_server_name" value="${esc(entity?.tls_server_name || "honey-agent")}" required autocomplete="off"></label>
      <label><span>Extra addresses</span><input name="extra_addresses" value="${esc((entity?.extra_addresses || []).join(", "))}" placeholder="198.51.100.7, 2001:db8::1" autocomplete="off"></label>
      ${editing ? `<label><span>Enabled</span><select name="enabled">${option("true","on",String(entity.enabled))}${option("false","off",String(entity.enabled))}</select></label>` : ""}
      <p class="form-note">Extra public addresses become additional failover targets per inbound in subscriptions.</p><p class="field-error"></p>`;
    if (kind === "user") return `
      <label><span>Username</span><input name="username" value="${esc(entity?.username || "")}" required placeholder="alice" autocomplete="off"></label>
      <label>${helpLabel("Subscription title", "The profile name shown by Happ and other subscription clients. Empty uses the global default; maximum 25 characters.")}<input name="subscription_title" maxlength="25" value="${esc(entity?.subscription_title || "")}" placeholder="custom title (optional)" autocomplete="off"></label>
      <label>${helpLabel("Subscription description", "Happ announcement text. Supports {DAYS_ELAPSED}, {TRAFFIC_SPENT}, {DAYS_LEFT} and {USERNAME}; maximum 200 characters.")}<textarea name="subscription_description" maxlength="200" rows="3" placeholder="Traffic: {TRAFFIC_SPENT} · left: {DAYS_LEFT}">${esc(entity?.subscription_description || "")}</textarea></label>
      <div class="form-row"><label><span>Client group</span><input name="subscription_group" maxlength="40" value="${esc(entity?.subscription_group || "")}" placeholder="global default"></label><label><span>Traffic row</span><select name="subscription_traffic_policy">${["inherit","auto","always","never"].map((v) => option(v, v, entity?.subscription_traffic_policy || "inherit")).join("")}</select></label></div>
      ${editing ? "" : '<label><span>Password</span><div class="form-row"><input name="password" type="password" required placeholder="generated or custom" autocomplete="new-password"><button type="button" class="button secondary" data-action="generate-user-password">Generate</button></div></label>'}
      <div class="form-row"><label><span>Traffic limit, GB</span><input name="traffic_limit_gb" type="number" min="0" step=".01" value="${entity ? Number(entity.traffic_limit_bytes || 0) / 1024 ** 3 : 0}"></label><label><span>Expires in, days</span><input name="expires_days" type="number" min="0" step="1" inputmode="numeric" value="${expiryDays(entity?.expires_at)}" required></label></div>
      <div class="form-row"><label>${helpLabel("Device limit", "Max distinct concurrent source IPs (a 'device' = a source address; no first-party client for real HWID). 0 = unlimited. Over the limit raises a Traffic/Device alert; enforcement (closing the newest connections) is a runtime setting.")}<input name="device_limit" type="number" min="0" step="1" inputmode="numeric" value="${Number(entity?.device_limit || 0)}"></label>${editing ? `<label><span>Enabled</span><select name="enabled">${option("true","on",String(entity.enabled))}${option("false","off",String(entity.enabled))}</select></label>` : "<label><span>&nbsp;</span><span class=\"form-note\" style=\"padding-top:9px\">anti-sharing cap</span></label>"}</div>
      <p class="form-note">0 GB and 0 days mean unlimited. Device limit 0 = unlimited.</p><p class="field-error"></p>`;
    if (kind === "domain") return `
      <label><span>Host</span><input name="host" value="${esc(entity?.host || "")}" required placeholder="cdn.example.com" autocomplete="off" ${editing ? "readonly" : ""}></label>
      <div class="form-row"><label><span>Node</span><select name="node_id"><option value="">— none —</option>${state.nodes.map((n) => option(n.id, n.name, entity?.node_id || "")).join("")}</select></label><label><span>Fronting</span><select name="proxied">${option("false","direct (points at node)",String(entity?.proxied ?? false))}${option("true","CDN / proxied",String(entity?.proxied ?? false))}</select></label></div>
      <label><span>Notes</span><input name="notes" value="${esc(entity?.notes || "")}" placeholder="cloudflare · ws front" autocomplete="off"></label>
      <p class="form-note">A domain you own. Used to pick node addresses, TLS server names and CDN hosts. REALITY SNI/target remain independent fields.</p><p class="field-error"></p>`;
    if (kind === "group") return `
      <label><span>Name</span><input name="name" value="${esc(entity?.name || "")}" required placeholder="eu-premium" autocomplete="off"></label>
      <label><span>Note</span><input name="note" value="${esc(entity?.note || "")}" placeholder="paid tier, low latency" autocomplete="off"></label>
      <p class="form-note">Assign nodes and grant users this group afterwards. A node with no group is universal.</p><p class="field-error"></p>`;
    if (kind === "profile") {
      const has = (arr, v) => (arr || []).includes(v);
      const bs = (name, cur) => `<select name="${name}">${option("false", "off", String(cur))}${option("true", "on", String(cur))}</select>`;
      return `
      <label><span>Name</span><input name="name" value="${esc(entity?.name || "")}" required placeholder="ru-split / default" autocomplete="off"></label>
      <div class="form-row"><label><span>Ad-block</span>${bs("block_ads", entity?.block_ads ?? false)}</label><label><span>Bypass LAN</span>${bs("direct_private", entity?.direct_private ?? true)}</label></div>
      <div class="form-row"><label><span>Direct China</span>${bs("direct_cn", has(entity?.direct_geosite, "cn"))}</label><label><span>Direct Russia</span>${bs("direct_ru", has(entity?.direct_geosite, "ru"))}</label></div>
      <div class="form-row"><label><span>Final → proxy</span>${bs("final_proxy", entity?.final_proxy ?? true)}</label><label><span>Default profile</span>${bs("is_default", entity?.is_default ?? false)}</label></div>
      <div class="form-row"><label><span>Block adult</span>${bs("block_adult", entity?.block_adult ?? false)}</label><label><span>Block gambling</span>${bs("block_gambling", entity?.block_gambling ?? false)}</label></div>
      <label><span>Blocked domains (comma-separated)</span><input name="blocked_domains" value="${esc((entity?.blocked_domains || []).join(", "))}" placeholder="ads.example.com, tracker.net" autocomplete="off"></label>
      <label><span>Direct (bypass) domains</span><input name="direct_domains" value="${esc((entity?.direct_domains || []).join(", "))}" placeholder="bank.local, intranet.corp" autocomplete="off"></label>
      <label><span>Force-proxy domains</span><input name="proxy_domains" value="${esc((entity?.proxy_domains || []).join(", "))}" placeholder="site.blocked.example" autocomplete="off"></label>
      <label><span>Per-app rules (one per line: <code>geosite action</code>)</span><textarea name="app_rules" rows="3" placeholder="telegram direct&#10;netflix proxy&#10;category-porn block" autocomplete="off" spellcheck="false">${esc((entity?.app_rules || []).map((r) => `${r.geosite} ${r.action}`).join("\n"))}</textarea></label>
      <div class="section-label">CLIENT DNS HARDENING (sing-box)</div>
      <label>${helpLabel("DoH resolver (optional)", "A DNS-over-HTTPS URL the client resolves through, tunnelled via the proxy. Empty = no DNS section (current behaviour). Example: https://dns.quad9.net/dns-query")}<input name="dns_doh" value="${esc(entity?.dns_doh || "")}" placeholder="https://dns.quad9.net/dns-query" autocomplete="off" spellcheck="false"></label>
      <div class="form-row"><label><span>FakeIP</span><select name="dns_fakeip">${option("false","off",String(entity?.dns_fakeip ?? false))}${option("true","on",String(entity?.dns_fakeip ?? false))}</select></label><label><span>Block plaintext :53</span><select name="dns_block_plain">${option("false","off",String(entity?.dns_block_plain ?? false))}${option("true","on",String(entity?.dns_block_plain ?? false))}</select></label></div>
      <label><span>Notes</span><input name="notes" value="${esc(entity?.notes || "")}" autocomplete="off"></label>
      <p class="form-note">Per-app rules route a whole SagerNet geosite category (app/service) to <code>direct</code>, <code>proxy</code> or <code>block</code> — e.g. keep <code>telegram direct</code> while proxying <code>netflix</code>. Content-filter blocks (adult/gambling) use geosite categories; custom domains match by suffix. All rules emit into both sing-box and Clash. <b>DNS hardening</b> (sing-box only): a DoH resolver reached through the proxy, optional FakeIP, and dropping outgoing :53 to stop DNS leaks. Editing bumps the version — clients pick it up on refresh.</p><p class="field-error"></p>`;
    }
    return `
      <label><span>Node</span><select name="node_id" ${editing ? "disabled" : ""}>${state.nodes.map((node) => option(node.id, node.name, entity?.node_id || state.nodes[0]?.id)).join("")}</select></label>
      <div class="form-row"><label><span>Tag</span><input name="tag" value="${esc(entity?.tag || "")}" required placeholder="vless-in"></label><label><span>Port</span><input name="listen_port" type="number" min="1" max="65535" value="${esc(entity?.listen_port || "")}" required placeholder="443"></label></div>
      <div class="form-row"><label><span>Protocol</span><select name="kind">${["vless","hysteria2","vmess","trojan","shadowsocks","tuic","anytls","shadowtls"].map((v) => option(v,v,entity?.kind || "vless")).join("")}</select></label><label><span>Core</span><select name="core">${option("singbox","sing-box",entity?.core || "singbox")}${option("xray","xray",entity?.core || "singbox")}</select></label></div>
      <div class="form-row"><label><span>Flow</span><select name="flow">${["", "xtls-rprx-vision"].map((v) => option(v, v || "default", entity?.flow || "")).join("")}</select></label><label><span>Enabled</span><select name="enabled">${option("true","on",String(entity?.enabled ?? true))}${option("false","off",String(entity?.enabled ?? true))}</select></label></div>
      <div class="form-row"><label><span>TLS</span><select name="tls_enabled">${option("false","off",String(entity?.tls_enabled ?? false))}${option("true","on",String(entity?.tls_enabled ?? false))}</select></label><label><span>Server name</span><input name="server_name" value="${esc(entity?.server_name || "")}" placeholder="vpn.example.com"></label></div>
      <div class="form-row"><label><span>Certificate path</span><input name="cert_path" value="${esc(entity?.cert_path || "")}" placeholder="/etc/letsencrypt/fullchain.pem"></label><label><span>Key path</span><input name="key_path" value="${esc(entity?.key_path || "")}" placeholder="/etc/letsencrypt/privkey.pem"></label></div>
      <div class="form-row"><label><span>REALITY</span><select name="reality">${option("false","off",String(entity?.reality ?? false))}${option("true","on",String(entity?.reality ?? false))}</select></label><label><span>Private key</span><input name="reality_private_key" type="password" placeholder="${editing ? "leave unchanged" : "required for reality"}"></label></div>
      <label><span>REALITY public key</span><input name="reality_public_key" value="${esc(entity?.reality_public_key || "")}"></label>
      <div class="form-row"><label><span>Short IDs</span><input name="reality_short_ids" value="${esc((entity?.reality_short_ids || []).join(","))}" placeholder="deadbeef,01234567"></label><label><span>Handshake</span><input name="reality_handshake_server" value="${esc(entity?.reality_handshake_server || "")}" placeholder="www.cloudflare.com"></label></div>
      <div class="form-row"><label><span>Handshake port</span><input name="reality_handshake_port" type="number" min="1" max="65535" value="${esc(entity?.reality_handshake_port || "")}" placeholder="443"></label><label><span>Network</span><select name="network">${wizNetworks(entity?.core || "singbox", entity?.reality ? "reality" : "tls").map((v) => option(v,v,entity?.network || "tcp")).join("")}</select></label></div>
      <div class="form-row"><label><span>Transport path</span><input name="transport_path" value="${esc(entity?.transport_path || "")}" placeholder="/honey"></label><label><span>Transport host</span><input name="transport_host" value="${esc(entity?.transport_host || "")}" placeholder="cdn.example.com"></label></div>
      <div class="form-row"><label><span>gRPC service</span><input name="transport_service_name" value="${esc(entity?.transport_service_name || "")}" placeholder="honey"></label><label><span>xHTTP mode</span><select name="transport_mode">${["auto","packet-up","stream-up","stream-one"].map((v) => option(v,v,entity?.transport_mode || "auto")).join("")}</select></label></div>
      <div class="form-row"><label><span>uTLS fingerprint</span><select name="utls_fingerprint">${["chrome","firefox","safari","ios","android","edge","360","qq","random","randomized"].map((v) => option(v,v,entity?.utls_fingerprint || "qq")).join("")}</select></label><label><span>ECH</span><select name="ech">${option("false","off",String(entity?.ech ?? false))}${option("true","on",String(entity?.ech ?? false))}</select></label></div>
      <label><span>UDP idle timeout (Hysteria2)</span><input name="udp_idle_timeout" value="${esc(entity?.udp_idle_timeout || "60s")}" placeholder="60s"></label>
      <div class="form-row"><label><span>ShadowTLS handshake</span><input name="shadowtls_handshake_server" value="${esc(entity?.shadowtls_handshake_server || "")}" placeholder="www.cloudflare.com"></label><label><span>ShadowTLS port</span><input name="shadowtls_handshake_port" type="number" min="1" max="65535" value="${esc(entity?.shadowtls_handshake_port || "")}" placeholder="443"></label></div>
      <label><span>Extra JSON</span><textarea name="extra" rows="4">${esc(JSON.stringify(entity?.extra || {}, null, 2))}</textarea></label>
      <p class="field-error"></p>`;
  }

  async function submitEntity() {
    const kind = entityForm.dataset.kind;
    const id = entityForm.dataset.id;
    const editing = Boolean(id);
    const data = Object.fromEntries(new FormData(entityForm));
    let extra = {};
    if (kind === "inbound") {
      try {
        extra = JSON.parse(data.extra || "{}");
      } catch {
        $(".field-error", $("#form-body")).textContent = "Extra JSON is not valid JSON.";
        return;
      }
    }
    let path, body;
    if (kind === "node") {
      path = editing ? `/nodes/${id}` : "/nodes";
      body = { name: data.name.trim(), address: data.address.trim(), tls_server_name: data.tls_server_name.trim(), grpc_port: Number(data.grpc_port), transport: data.transport };
      if (editing) body.enabled = data.enabled === "true";
      if (editing) body.extra_addresses = (data.extra_addresses || "").split(",").map((s) => s.trim()).filter(Boolean);
    } else if (kind === "domain") {
      path = editing ? `/domains/${id}` : "/domains";
      body = { host: data.host.trim(), node_id: data.node_id || null, proxied: data.proxied === "true", notes: data.notes.trim() };
    } else if (kind === "group") {
      path = editing ? `/groups/${id}` : "/groups";
      body = { name: data.name.trim(), note: (data.note || "").trim() };
    } else if (kind === "profile") {
      path = editing ? `/routing-profiles/${id}` : "/routing-profiles";
      const geosite = [], geoip = [];
      if (data.direct_cn === "true") { geosite.push("cn"); geoip.push("cn"); }
      if (data.direct_ru === "true") { geosite.push("ru"); geoip.push("ru"); }
      const domains = (v) => (v || "").split(",").map((s) => s.trim().toLowerCase()).filter(Boolean);
      const appRules = (data.app_rules || "").split("\n").map((line) => line.trim().split(/\s+/)).filter((p) => p[0]).map((p) => ({ geosite: p[0].toLowerCase(), action: (p[1] || "proxy").toLowerCase() })).filter((r) => ["direct", "proxy", "block"].includes(r.action));
      body = { name: data.name.trim(), block_ads: data.block_ads === "true", direct_private: data.direct_private === "true", direct_geosite: geosite, direct_geoip: geoip, final_proxy: data.final_proxy === "true", is_default: data.is_default === "true", notes: data.notes.trim(), block_adult: data.block_adult === "true", block_gambling: data.block_gambling === "true", blocked_domains: domains(data.blocked_domains), direct_domains: domains(data.direct_domains), proxy_domains: domains(data.proxy_domains), app_rules: appRules, dns_doh: (data.dns_doh || "").trim(), dns_fakeip: data.dns_fakeip === "true", dns_block_plain: data.dns_block_plain === "true" };
    } else if (kind === "user") {
      path = editing ? `/users/${id}` : "/users";
      const expiresDays = Number(data.expires_days || 0);
      if (!Number.isInteger(expiresDays) || expiresDays < 0) {
        $(".field-error", $("#form-body")).textContent = "Expiry must be a whole number of days, or 0 for unlimited.";
        return;
      }
      body = {
        username: data.username.trim(),
        subscription_title: data.subscription_title.trim() || null,
        subscription_description: data.subscription_description.trim() || null,
        subscription_group: data.subscription_group.trim() || null,
        subscription_traffic_policy: data.subscription_traffic_policy || "inherit",
        traffic_limit_bytes: Math.round(Number(data.traffic_limit_gb || 0) * 1024 ** 3),
        expires_at: expiresDays === 0 ? null : new Date(Date.now() + expiresDays * 86_400_000).toISOString(),
        device_limit: Math.max(0, Math.round(Number(data.device_limit || 0)))
      };
      if (!editing) body.password = data.password;
      if (editing) body.enabled = data.enabled === "true";
    } else {
      path = editing ? `/inbounds/${id}` : "/inbounds";
      body = { tag: data.tag.trim(), kind: data.kind, core: data.core, listen_port: Number(data.listen_port), flow: data.flow.trim(), enabled: data.enabled === "true", tls_enabled: data.tls_enabled === "true", server_name: data.server_name.trim() || null, cert_path: data.cert_path.trim() || null, key_path: data.key_path.trim() || null, reality: data.reality === "true", reality_public_key: data.reality_public_key.trim() || null, reality_short_ids: data.reality_short_ids.split(",").map((v) => v.trim()).filter(Boolean), reality_handshake_server: data.reality_handshake_server.trim() || null, reality_handshake_port: data.reality_handshake_port ? Number(data.reality_handshake_port) : null, network: data.network, transport_path: data.transport_path.trim() || null, transport_host: data.transport_host.trim() || null, transport_service_name: data.transport_service_name.trim() || null, transport_mode: data.network === "xhttp" ? (data.transport_mode || "auto") : null, ech: data.ech === "true", utls_fingerprint: (data.tls_enabled === "true" || data.reality === "true") ? (data.utls_fingerprint || "qq") : null, shadowtls_handshake_server: data.shadowtls_handshake_server.trim() || null, shadowtls_handshake_port: data.shadowtls_handshake_port ? Number(data.shadowtls_handshake_port) : null, udp_idle_timeout: (data.udp_idle_timeout || "60s").trim(), extra };
      if (!editing) body.node_id = data.node_id;
      if (data.reality_private_key) body.reality_private_key = data.reality_private_key;
    }
    const errorNode = $(".field-error", $("#form-body"));
    try {
      $("#form-submit").disabled = true;
      const result = await api(path, { method: editing ? "PATCH" : "POST", body: JSON.stringify(body) });
      formDialog.close();
      toast(`${kind} ${editing ? "saved" : "added"}`);
      await loadData({ quiet: true });
      if (!editing && kind === "user" && result.subscription_path) {
        showResult("User created", [["UUID", result.uuid], ["Subscription", location.origin + result.subscription_path]], "users");
      } else if (!editing) {
        go(kind === "node" ? "nodes" : kind === "inbound" ? "inbounds" : "users");
      }
    } catch (error) {
      errorNode.textContent = error.message;
    } finally {
      $("#form-submit").disabled = false;
    }
  }

  function showResult(title, rows, returnRoute = "") {
    $("#form-eyebrow").textContent = "shown once";
    $("#form-title").textContent = title;
    $("#form-submit").textContent = "Done";
    entityForm.dataset.kind = "result";
    state.resultReturnRoute = returnRoute;
    $("#form-body").innerHTML = `<p class="form-note">Copy these values now. Sensitive tokens are not returned by list endpoints.</p>${rows.filter(([, value]) => value).map(([label, value]) => `<label><span>${esc(label)}</span><div style="display:flex;gap:7px"><input readonly value="${esc(value)}"><button type="button" class="button secondary" data-copy="${esc(value)}">${icon("copy")}</button></div></label>`).join("")}`;
    formDialog.showModal();
  }

  function showList(title, html) {
    $("#form-eyebrow").textContent = "inspect";
    $("#form-title").textContent = title;
    $("#form-submit").textContent = "Done";
    entityForm.dataset.kind = "result";
    $("#form-body").innerHTML = html;
    if (!formDialog.open) formDialog.showModal();
  }

  function entityForLabels(kind, id) {
    const items = kind === "node" ? state.nodes : kind === "inbound" ? state.inbounds : state.users;
    return items.find((item) => item.id === id);
  }

  function editLabels(kind, id) {
    const entity = entityForLabels(kind, id);
    if (!entity) throw new Error(`${kind} not found`);
    showList(`Labels · ${entity.name || entity.tag || entity.username}`, `<div class="form-body">
      <label><span>Labels</span><input id="label-editor" value="${esc(labelsOf(entity).join(", "))}" placeholder="region:pl, production"></label>
      <p class="form-note">Up to 16 lowercase labels. Letters, digits, dot, underscore, colon and dash are accepted. Labels are organizational metadata and do not change node configuration or subscriptions.</p>
      <button type="button" class="button primary" data-action="save-labels" data-kind="${kind}" data-id="${id}">Save labels</button>
    </div>`);
    requestAnimationFrame(() => $("#label-editor")?.focus());
  }

  async function saveLabels(kind, id) {
    const labels = ($("#label-editor")?.value || "").split(",").map((label) => label.trim()).filter(Boolean);
    const path = kind === "inbound" ? "inbounds" : `${kind}s`;
    await api(`/${path}/${id}/labels`, { method: "PUT", body: JSON.stringify({ labels }) });
    formDialog.close();
    toast("labels saved");
    await loadData({ quiet: true });
  }

  function savedDefinition(resource) {
    const config = viewConfig(resource);
    const definition = { search: config.search, labels: config.labels, sort: config.sort, columns: config.columns };
    if (resource === "issues") Object.assign(definition, state.issueFilters);
    return definition;
  }

  function openSavedViewName(resource, mode, id = "") {
    const current = state.savedViews.find((item) => item.id === id);
    const title = mode === "rename" ? "Rename saved view" : "Save current view";
    showList(title, `<div class="form-body"><label><span>Name</span><input id="saved-view-name" maxlength="80" value="${esc(current?.name || "")}" placeholder="Production in Poland"></label><button type="button" class="button primary" data-action="commit-saved-view" data-resource="${resource}" data-mode="${mode}" data-id="${id}">${mode === "rename" ? "Rename" : "Save view"}</button></div>`);
    requestAnimationFrame(() => $("#saved-view-name")?.focus());
  }

  async function commitSavedView(resource, mode, id) {
    const name = ($("#saved-view-name")?.value || "").trim();
    if (!name) throw new Error("view name is required");
    if (mode === "rename") {
      await api(`/saved-views/${id}`, { method: "PATCH", body: JSON.stringify({ name }) });
    } else {
      const created = await api("/saved-views", { method: "POST", body: JSON.stringify({ name, resource, definition: savedDefinition(resource) }) });
      state.activeSavedViews[resource] = created.id;
    }
    formDialog.close();
    toast(mode === "rename" ? "view renamed" : "view saved");
    await loadData({ quiet: true });
  }

  async function updateSavedView(resource, id) {
    await api(`/saved-views/${id}`, { method: "PATCH", body: JSON.stringify({ definition: savedDefinition(resource) }) });
    toast("saved view updated");
    await loadData({ quiet: true });
  }

  async function deleteSavedView(resource, id) {
    if (!confirm("Delete this saved view?")) return;
    await api(`/saved-views/${id}`, { method: "DELETE" });
    state.activeSavedViews[resource] = "";
    toast("saved view deleted");
    await loadData({ quiet: true });
  }

  function applySavedView(resource, id) {
    state.activeSavedViews[resource] = id;
    const saved = state.savedViews.find((item) => item.id === id);
    if (saved) {
      const defaults = viewConfig(resource);
      const definition = saved.definition || {};
      state.tableViews[resource] = {
        search: definition.search || "",
        labels: Array.isArray(definition.labels) ? definition.labels : [],
        sort: definition.sort || defaults.sort,
        columns: Array.isArray(definition.columns) && definition.columns.length ? definition.columns : defaults.columns,
      };
      if (resource === "issues") {
        state.issueFilters = { severity: definition.severity || "", kind: definition.kind || "", node: definition.node || "" };
      }
    }
    render();
  }

  function previewRows(label, rows, cls = "") {
    if (!rows.length) return "";
    return `<h4 style="margin:16px 0 8px">${esc(label)}</h4><div class="check-list">${rows.map((row) => `<div class="check-row"><span><b>${esc(row.tag)}</b><small>${esc(row.core)} · ${esc(row.protocol)} · ${esc(row.listen)}:${esc(row.port)} · ${esc(row.network)} · ${esc(row.security)} · ${esc(row.user_count)} user(s)</small></span><span class="status ${cls}">${esc(label.toLowerCase())}</span></div>`).join("")}</div>`;
  }

  async function previewNode(node) {
    const result = await api(`/nodes/${node.id}/config-preview`);
    const baseline = result.baseline_available
      ? "Compared with the last successfully applied sanitized snapshot."
      : "No applied snapshot exists yet; all desired inbounds are shown as additions.";
    const changes = previewRows("Added", result.added, "ok")
      + previewRows("Modified", result.modified, "warn")
      + previewRows("Removed", result.removed, "bad");
    showList(`Config preview · ${node.name}`, `<div class="form-body">
      <p class="form-note">${esc(baseline)} No credentials, private keys, hosts, paths, or extra JSON are included here.</p>
      <div class="check-list"><div class="check-row"><span><b>${result.changed ? "Changes pending" : "Already applied"}</b><small>desired ${esc(result.desired_hash.slice(0, 16))} · applied ${esc((result.applied_hash || "none").slice(0, 16))}</small></span></div>
      <div class="check-row"><span><b>Cores affected</b><small>${esc(result.restart_cores.join(", ") || "none")}</small></span></div></div>
      ${changes || '<div class="empty" style="min-height:90px"><div><h3>No structural changes</h3></div></div>'}
      <div style="display:flex;gap:8px;margin-top:16px"><button type="button" class="button secondary" data-action="dry-run-node" data-id="${esc(node.id)}">Validate candidate</button>${result.changed ? `<button type="button" class="button primary" data-action="apply-node" data-id="${esc(node.id)}">Apply now</button>` : ""}</div>
    </div>`);
  }

  async function dryRunNode(node) {
    const result = await api(`/nodes/${node.id}/dry-run`, { method: "POST" });
    const rejected = result.state === "Errored";
    showList(`Dry-run · ${node.name}`, `<div class="form-body"><div class="check-row"><span><b>${rejected ? "Candidate rejected" : "Candidate valid"}</b><small>${esc(result.message)}</small></span><span class="status ${rejected ? "bad" : "ok"}">${esc(result.state)}</span></div><p class="form-note">No core process, live config, firewall rule, or recovery marker was changed.</p></div>`);
  }

  async function manageRuntimeSettings() {
    try {
      const s = await api("/settings");
      state.settings = s;
      const profiles = s.subscription_client_profiles || {};
      const profileRow = (key, label) => {
        const p = profiles[key] || {};
        return `<div class="form-row"><label><span>${label} · XHTTP mode</span><select id="set-profile-${key}-mode">${["auto","packet-up","stream-up","stream-one"].map((v) => option(v, v, p.xhttp_mode || "auto")).join("")}</select></label><label><span>${label} · fingerprint</span><select id="set-profile-${key}-fp">${["chrome","firefox","safari","ios","android","edge","360","qq","random","randomized"].map((v) => option(v, v, p.fingerprint || "qq")).join("")}</select></label></div>`;
      };
      showList("Runtime settings", `<div class="form-body">
        <div class="section-label">AUTO-PUSH</div>
        <div class="form-row"><label><span>Automatic configuration delivery</span><select id="set-autopush"><option value="true" ${s.auto_push_enabled !== false ? "selected" : ""}>on</option><option value="false" ${s.auto_push_enabled === false ? "selected" : ""}>off (manual push only)</option></select></label><span class="form-note">When on, changes, quota enforcement, scheduled operations, CDN rotation and drift reconciliation are pushed automatically. Manual node push always works.</span></div>
        <div class="form-row"><label><span>Reconcile interval, s</span><input id="set-reconcile" type="number" min="5" max="86400" value="${s.reconcile_secs}"></label><label><span>Default inbound core</span><select id="set-core">${["singbox", "xray"].map((v) => `<option value="${v}" ${s.default_inbound_core === v ? "selected" : ""}>${v === "singbox" ? "sing-box" : "xray"}</option>`).join("")}</select></label></div>
        <div class="form-row"><label><span>Audit retention, rows</span><input id="set-audit" type="number" min="10" max="5000" value="${s.audit_retention}"></label><label><span>Runtime log lines</span><input id="set-log" type="number" min="10" max="5000" value="${s.runtime_log_limit}"></label></div>
         <div class="form-row"><label><span>Traffic history, days</span><input id="set-traffic-history" type="number" min="7" max="3650" value="${s.traffic_history_days || 180}"></label><span class="form-note">Hourly buckets are retained for this period.</span></div>
        <div class="section-label">PUBLIC SUBSCRIPTIONS</div>
        <div class="form-row"><label><span>Global subscription title</span><input id="set-sub-title" maxlength="25" value="${esc(s.default_subscription_title || "")}" placeholder="VPN Elusion"></label><label><span>Global client group</span><input id="set-sub-group" maxlength="40" value="${esc(s.default_subscription_group || "")}" placeholder="Premium"></label></div>
        <label><span>Global subscription description</span><textarea id="set-sub-description" maxlength="200" rows="3" placeholder="Traffic: {TRAFFIC_SPENT} · left: {DAYS_LEFT}">${esc(s.default_subscription_description || "")}</textarea></label>
        <p class="form-note">Per-user title and description override these defaults. Supported tags: {USERNAME}, {DAYS_ELAPSED}, {TRAFFIC_SPENT}, {DAYS_LEFT}. Happ receives Telegram as a support button.</p>
        <div class="form-row"><label><span>Telegram / support URL</span><input id="set-sub-support" value="${esc(s.subscription_support_url || "")}" placeholder="https://t.me/example"></label><label><span>Update interval, hours</span><input id="set-sub-interval" type="number" min="1" max="168" value="${Number(s.profile_update_interval_hours || 1)}"></label></div>
        <div class="form-row"><label><span>Fallback subscription origin</span><input id="set-sub-fallback" value="${esc(s.subscription_fallback_base_url || "")}" placeholder="https://sub-fi.example.com"></label><span class="form-note">Optional HTTPS origin served through the reserve node. New imports and QR codes use it, while this panel remains the source of truth.</span></div>
        <div class="form-row"><label><span>Traffic row default</span><select id="set-sub-traffic-policy">${["auto","always","never"].map((v) => option(v, v, s.subscription_traffic_policy || "auto")).join("")}</select></label><span class="form-note"><b>auto</b> emits traffic metadata only for users with a finite traffic limit.</span></div>
        <div class="section-label">CLIENT COMPATIBILITY PROFILES</div>
        ${profileRow("happ-android", "Happ Android")}
        ${profileRow("happ-desktop", "Happ Desktop")}
        ${profileRow("karing", "Karing")}
        ${profileRow("generic", "Generic Xray")}
        <p class="form-note">Profiles override only generated XHTTP mode and uTLS fingerprint. They do not modify or restart server inbounds.</p>
        <div class="form-row"><label><span>Guard</span><select id="set-sub-guard"><option value="true" ${s.subscription_guard_enabled ? "selected" : ""}>on</option><option value="false" ${!s.subscription_guard_enabled ? "selected" : ""}>off (diagnostic only)</option></select></label><label><span>Requests per window</span><input id="set-sub-max" type="number" min="10" max="10000" value="${s.subscription_guard_max_requests}"></label></div>
        <div class="form-row"><label><span>Window, s</span><input id="set-sub-window" type="number" min="10" max="3600" value="${s.subscription_guard_window_secs}"></label><label><span>Block, s</span><input id="set-sub-block" type="number" min="10" max="86400" value="${s.subscription_guard_block_secs}"></label></div>
        <div class="check-list"><div class="check-row"><span><b>Guard telemetry</b><small>${Number(s.subscription_guard_allowed_total || 0)} allowed · ${Number(s.subscription_guard_blocked_total || 0)} blocked since restart · ${Number(s.subscription_guard_recent_blocks || 0)} recent persisted blocks · ${Number(s.subscription_guard_active_buckets || 0)} active buckets</small></span></div></div>
        <p class="form-note">Applied live on the next request. Limits are isolated by hashed client and subscription identity; token and alias traffic share one budget. Disable the guard only for short diagnostics.</p>
        <div class="section-label">TRAFFIC ANOMALY (ANTI-ABUSE)</div>
        <div class="form-row"><label><span>Detection</span><select id="set-anom-enabled"><option value="true" ${s.anomaly_enabled ? "selected" : ""}>on</option><option value="false" ${!s.anomaly_enabled ? "selected" : ""}>off</option></select></label><label><span>Spike threshold, % of baseline</span><input id="set-anom-factor" type="number" min="150" max="100000" value="${s.anomaly_factor_pct ?? 500}"></label></div>
        <div class="form-row"><label><span>Minimum spike, MiB</span><input id="set-anom-min" type="number" min="0" max="10485760" value="${s.anomaly_min_mib ?? 5120}"></label><label><span>Baseline window, hours</span><input id="set-anom-baseline" type="number" min="6" max="720" value="${s.anomaly_baseline_hours ?? 72}"></label></div>
        <div class="form-row"><label><span>Min history, active hours</span><input id="set-anom-history" type="number" min="1" max="240" value="${s.anomaly_min_history_hours ?? 6}"></label><span class="form-note">Hourly scan compares each user's last completed hour to their own active-hour average; a hit above the floor and threshold raises a <b>Traffic spike</b> alert.</span></div>
        <div class="section-label">SOFTWARE SELF-UPDATE</div>
        <div class="form-row"><label><span>One-click self-update</span><select id="set-selfupdate"><option value="false" ${!s.self_update_enabled ? "selected" : ""}>off</option><option value="true" ${s.self_update_enabled ? "selected" : ""}>on (owner can install from panel)</option></select></label><span class="form-note">When on, an owner can download+verify+install a newer master from GitHub in one click. Binary is SHA-256-verified; the supervisor restarts the process. Off = check-only.</span></div>
        <div class="section-label">CDN ROTATION (by latency)</div>
        <div class="form-row"><label><span>Proactive rotation</span><select id="set-cdnrot-enabled"><option value="false" ${!s.cdn_rotate_enabled ? "selected" : ""}>off</option><option value="true" ${s.cdn_rotate_enabled ? "selected" : ""}>on</option></select></label><label><span>Switch margin, %</span><input id="set-cdnrot-margin" type="number" min="1" max="90" value="${s.cdn_rotate_margin_pct ?? 30}"></label></div>
        <span class="form-note">Measures TCP latency to each inbound's CDN pool and points its fronting host at the fastest reachable edge — only switching when a candidate is faster by this margin (avoids flapping). Set a CDN pool on the inbound.</span>
        <div class="section-label">PRE-ROLLOUT GATE</div>
        <div class="form-row"><label><span>Preflight before push</span><select id="set-preflight-gate">${["off", "warn", "block"].map((v) => option(v, v === "off" ? "off" : v === "warn" ? "warn (log only)" : "block the push", s.preflight_gate || "warn")).join("")}</select></label><span class="form-note">Probes the node's control port and inbound ports before a manual push. A probe proves reachability from the master's network only — not a "clean IP" guarantee.</span></div>
        <div class="section-label">DEVICE LIMITS (ANTI-SHARING)</div>
        <div class="form-row"><label><span>Enforcement</span><select id="set-devlimit-enforce"><option value="false" ${!s.device_limit_enforce ? "selected" : ""}>alert only</option><option value="true" ${s.device_limit_enforce ? "selected" : ""}>close excess connections</option></select></label><span class="form-note">Per-user device limit = distinct concurrent source IPs (set on each user). Over-limit always alerts; enforcement additionally closes the newest connections via the Clash API.</span></div>
        <button class="button primary" data-action="save-settings" style="margin-top:6px">Save</button>
        <p class="field-error" id="set-err"></p>
      </div>`);
    } catch (error) { toast(error.message, true); }
  }
  async function saveRuntimeSettings() {
    const body = {
      auto_push_enabled: document.getElementById("set-autopush").value === "true",
      reconcile_secs: Number(document.getElementById("set-reconcile").value),
      audit_retention: Number(document.getElementById("set-audit").value),
      runtime_log_limit: Number(document.getElementById("set-log").value),
      traffic_history_days: Number(document.getElementById("set-traffic-history").value),
      default_inbound_core: document.getElementById("set-core").value,
      default_subscription_title: document.getElementById("set-sub-title").value.trim(),
      default_subscription_description: document.getElementById("set-sub-description").value.trim(),
      default_subscription_group: document.getElementById("set-sub-group").value.trim(),
      subscription_traffic_policy: document.getElementById("set-sub-traffic-policy").value,
      profile_update_interval_hours: Number(document.getElementById("set-sub-interval").value),
      subscription_fallback_base_url: document.getElementById("set-sub-fallback").value.trim(),
      subscription_client_profiles: Object.fromEntries(["happ-android","happ-desktop","karing","generic"].map((key) => [key, {
        xhttp_mode: document.getElementById(`set-profile-${key}-mode`).value,
        fingerprint: document.getElementById(`set-profile-${key}-fp`).value,
      }])),
      subscription_support_url: document.getElementById("set-sub-support").value.trim(),
      subscription_guard_enabled: document.getElementById("set-sub-guard").value === "true",
      subscription_guard_max_requests: Number(document.getElementById("set-sub-max").value),
      subscription_guard_window_secs: Number(document.getElementById("set-sub-window").value),
      subscription_guard_block_secs: Number(document.getElementById("set-sub-block").value),
      anomaly_enabled: document.getElementById("set-anom-enabled").value === "true",
      anomaly_factor_pct: Number(document.getElementById("set-anom-factor").value),
      anomaly_min_mib: Number(document.getElementById("set-anom-min").value),
      anomaly_baseline_hours: Number(document.getElementById("set-anom-baseline").value),
      anomaly_min_history_hours: Number(document.getElementById("set-anom-history").value),
      device_limit_enforce: document.getElementById("set-devlimit-enforce").value === "true",
      preflight_gate: document.getElementById("set-preflight-gate").value,
      cdn_rotate_enabled: document.getElementById("set-cdnrot-enabled").value === "true",
      cdn_rotate_margin_pct: Number(document.getElementById("set-cdnrot-margin").value),
      self_update_enabled: document.getElementById("set-selfupdate").value === "true",
    };
    try {
      state.settings = await api("/settings", { method: "PATCH", body: JSON.stringify(body) });
      formDialog.close();
      toast("settings saved");
    } catch (error) { const el = document.getElementById("set-err"); if (el) el.textContent = error.message; }
  }

  function generateUserPassword() {
    const input = document.querySelector('#form-body input[name="password"]');
    if (!input) return;
    const alphabet = "ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789!@#$%";
    const bytes = new Uint8Array(20);
    crypto.getRandomValues(bytes);
    input.value = Array.from(bytes, (b) => alphabet[b % alphabet.length]).join("");
    input.type = "text";
    input.focus();
  }

  async function manageApiKeys() {
    try {
      const keys = await api("/api-keys");
      const row = (k) => {
        const lifecycle = k.status || (k.revoked_at ? "revoked" : k.expires_at && new Date(k.expires_at) < Date.now() ? "expired" : "active");
        const statusClass = lifecycle === "active" ? "ok" : lifecycle === "expired" ? "warn" : "bad";
        return `<div class="check-row session-row"><span><b>${esc(k.name)} · ${esc(k.role)}</b><small>created ${dateLabel(k.created_at)} · last used ${dateLabel(k.last_used_at)} · expires ${dateLabel(k.expires_at)}</small></span><span class="status ${statusClass}">${esc(lifecycle)}</span>${lifecycle === "active" ? `<button type="button" class="row-button danger" data-action="revoke-api-key" data-id="${esc(k.id)}">revoke</button>` : ""}</div>`;
      };
      showList("API keys", `<div class="form-body">
        ${keys.length ? `<div class="check-list">${keys.map(row).join("")}</div>` : '<p class="form-note">No API keys yet.</p>'}
        <div class="form-row" style="margin-top:12px"><label><span>Name</span><input id="ak-name" placeholder="ci-deploy" autocomplete="off"></label><label><span>Scope</span><select id="ak-role">${["viewer", "operator", "admin", "owner"].map((r) => `<option value="${r}">${r}</option>`).join("")}</select></label></div>
        <label><span>Expires in days (0 = never)</span><input id="ak-days" type="number" min="0" max="3650" step="1" value="30"></label>
        <button class="button primary" data-action="create-api-key" style="margin-top:6px">${icon("plus")} Create key</button>
        <p class="form-note">The key (<code>hny_…</code>) is shown once on creation — store it now. Scope maps to a role: viewer=read-only, up to owner=full.</p>
        <p class="field-error" id="ak-err"></p>
      </div>`);
    } catch (error) { toast(error.message, true); }
  }
  async function createApiKey() {
    const err = document.getElementById("ak-err");
    const name = document.getElementById("ak-name").value.trim();
    if (!name) { if (err) err.textContent = "name is required"; return; }
    const expiresDays = Number(document.getElementById("ak-days").value || 0);
    if (!Number.isInteger(expiresDays) || expiresDays < 0 || expiresDays > 3650) { if (err) err.textContent = "expiry must be 0-3650 days"; return; }
    const body = { name, role: document.getElementById("ak-role").value, expires_days: expiresDays };
    try {
      const res = await api("/api-keys", { method: "POST", body: JSON.stringify(body) });
      showResult("API key created", [["Bearer token", res.token], ["Role", res.role], ["Authorization header", `Bearer ${res.token}`]]);
      toast("api key created");
    } catch (error) { if (err) err.textContent = error.message; }
  }
  async function revokeApiKey(id) {
    if (!confirm("Revoke this API key? Anything using it will stop working immediately.")) return;
    try {
      await api(`/api-keys/${id}`, { method: "DELETE" });
      toast("api key revoked");
      await manageApiKeys();
    } catch (error) { toast(error.message, true); }
  }

  const rbacDomains = ["nodes", "inbounds", "users", "groups", "routing", "domains", "config", "notifications", "audit", "admin"];
  const rbacLevel = ["none", "read", "write"];
  async function manageCustomRoles() {
    try {
      const roles = await api("/custom-roles");
      state.customRoles = roles;
      const row = (r) => `<div class="check-row" style="justify-content:space-between"><span><b>${esc(r.name)}</b><small>${Object.entries(r.permissions || {}).filter(([, l]) => l > 0).map(([d, l]) => `${d}:${["-", "r", "rw"][l]}`).join(" · ") || "no grants"}</small></span><span class="row-actions"><button class="row-button" data-action="edit-custom-role" data-id="${r.id}">edit</button><button class="row-button danger" data-action="delete-custom-role" data-id="${r.id}">delete</button></span></div>`;
      showList("Custom roles (RBAC)", `<div class="form-body">
        ${roles.length ? `<div class="check-list">${roles.map(row).join("")}</div>` : '<p class="form-note">No custom roles yet.</p>'}
        <button class="button primary" data-action="new-custom-role" style="margin-top:12px">${icon("plus")} New role</button>
        <p class="form-note">A custom role is a matrix of domain → level (none/read/write). Assign it to an admin (Manage admins) and it overrides the rank role. The dashboard/overview is always visible.</p>
      </div>`);
    } catch (error) { toast(error.message, true); }
  }
  function customRoleForm(role) {
    const perms = role?.permissions || {};
    const rows = rbacDomains.map((d) => `<label><span>${d}</span><select data-perm="${d}">${rbacLevel.map((l, i) => `<option value="${i}" ${(perms[d] || 0) === i ? "selected" : ""}>${l}</option>`).join("")}</select></label>`).join("");
    showList(role ? `Edit · ${esc(role.name)}` : "New custom role", `<div class="form-body">
      <label><span>Role name</span><input id="cr-name" value="${esc(role?.name || "")}" placeholder="ops-nodes" autocomplete="off"></label>
      <p class="form-note" style="margin:8px 0 4px">Permissions per domain:</p>
      <div class="perm-grid">${rows}</div>
      <button class="button primary" data-action="save-custom-role" ${role ? `data-id="${role.id}"` : ""} style="margin-top:10px">Save role</button>
      <p class="field-error" id="cr-err"></p>
    </div>`);
  }
  async function saveCustomRole(id) {
    const err = document.getElementById("cr-err");
    const name = document.getElementById("cr-name").value.trim();
    if (!name) { if (err) err.textContent = "name is required"; return; }
    const permissions = {};
    document.querySelectorAll("[data-perm]").forEach((el) => { permissions[el.dataset.perm] = Number(el.value); });
    try {
      await api(id ? `/custom-roles/${id}` : "/custom-roles", { method: id ? "PATCH" : "POST", body: JSON.stringify({ name, permissions }) });
      toast("role saved");
      manageCustomRoles();
    } catch (error) { if (err) err.textContent = error.message; }
  }

  async function manageBranding() {
    try {
      const b = state.branding || (await api("/branding"));
      state.branding = b;
      const tog = (id, on, label) => `<label><span>${label}</span><select id="${id}">${option("true", "shown", String(on))}${option("false", "hidden", String(on))}</select></label>`;
      showList("White-label branding", `<div class="form-body">
        <div class="form-row"><label><span>Brand name</span><input id="br-name" value="${esc(b.brand_name || "")}" placeholder="honey" autocomplete="off"></label><label><span>Accent color (#rrggbb)</span><input id="br-accent" value="${esc(b.accent_color || "")}" placeholder="#4d8eff" autocomplete="off"></label></div>
        <label><span>Logo URL (optional)</span><input id="br-logo" value="${esc(b.logo_url || "")}" placeholder="https://… or /path" autocomplete="off"></label>
        <div class="form-row"><label><span>Support URL</span><input id="br-surl" value="${esc(b.support_url || "")}" placeholder="https://t.me/…" autocomplete="off"></label><label><span>Support text</span><input id="br-stext" value="${esc(b.support_text || "")}" placeholder="Need help?" autocomplete="off"></label></div>
        <label><span>Footer text</span><input id="br-footer" value="${esc(b.footer_text || "")}" placeholder="© your brand" autocomplete="off"></label>
        <label><span>Subscription welcome message</span><input id="br-welcome" value="${esc(b.sub_welcome || "")}" placeholder="Welcome — pick your app below." autocomplete="off"></label>
        <div class="form-row">${tog("br-imports", b.sub_show_imports, "Sub: import buttons")}${tog("br-downloads", b.sub_show_downloads, "Sub: downloads")}</div>
        <div class="form-row">${tog("br-endpoints", b.sub_show_endpoints, "Sub: endpoints")}<label><span>&nbsp;</span><span class="form-note" style="padding-top:9px">Applies to the public subscription & status pages.</span></label></div>
        <button class="button primary" data-action="save-branding" style="margin-top:6px">Save branding</button>
        <p class="form-note">All fields are plain text / toggles (rendered escaped) — no raw HTML, so there is no XSS surface.</p>
        <p class="field-error" id="br-err"></p>
      </div>`);
    } catch (error) { toast(error.message, true); }
  }
  async function saveBranding() {
    const err = document.getElementById("br-err");
    const body = {
      brand_name: document.getElementById("br-name").value.trim() || "honey",
      accent_color: document.getElementById("br-accent").value.trim(),
      logo_url: document.getElementById("br-logo").value.trim(),
      support_url: document.getElementById("br-surl").value.trim(),
      support_text: document.getElementById("br-stext").value.trim(),
      footer_text: document.getElementById("br-footer").value.trim(),
      sub_welcome: document.getElementById("br-welcome").value.trim(),
      sub_show_imports: document.getElementById("br-imports").value === "true",
      sub_show_downloads: document.getElementById("br-downloads").value === "true",
      sub_show_endpoints: document.getElementById("br-endpoints").value === "true",
    };
    try {
      state.branding = await api("/branding", { method: "PATCH", body: JSON.stringify(body) });
      applyBranding(state.branding);
      formDialog.close();
      toast("branding saved");
    } catch (error) { if (err) err.textContent = error.message; }
  }

  function downloadText(filename, text, type = "application/json") {
    const url = URL.createObjectURL(new Blob([text], { type }));
    const a = document.createElement("a");
    a.href = url; a.download = filename;
    document.body.append(a); a.click(); a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
  function manageImport() {
    showList("Import users", `<div class="form-body">
      <p class="form-note">Paste users from another panel. Generic: <code>{"users":[{"username":"a","traffic_limit_bytes":0}]}</code>. Marzban export also works (<code>data_limit</code>/<code>expire</code>/<code>status</code>). Existing usernames are skipped; each gets a fresh honey credential.</p>
      <textarea id="imp-json" rows="9" placeholder='{"users":[...]}' style="width:100%;font-family:ui-monospace,monospace;font-size:12px"></textarea>
      <button class="button primary" data-action="run-import" style="margin-top:8px">Import users</button>
      <p class="field-error" id="imp-err"></p>
    </div>`);
  }
  async function runImport() {
    const err = document.getElementById("imp-err");
    let payload;
    try { payload = JSON.parse(document.getElementById("imp-json").value); } catch { if (err) err.textContent = "invalid JSON"; return; }
    if (Array.isArray(payload)) payload = { users: payload };
    try {
      const r = await api("/import/users", { method: "POST", body: JSON.stringify(payload) });
      formDialog.close();
      toast(`imported ${r.created} · skipped ${r.skipped}${r.failed ? " · failed " + r.failed : ""}`, r.failed > 0);
      await loadData({ quiet: true });
    } catch (error) { if (err) err.textContent = error.message; }
  }
  function manageGitops() {
    showList("Config as code (GitOps)", `<div class="form-body">
      <p class="form-note">Declarative fleet config — groups, routing profiles, nodes (users excluded). Export to version-control it; apply to converge (create/update, matched by name).</p>
      <button class="button secondary" data-action="export-config">${icon("logs")} Download config.json</button>
      <label style="display:block;margin-top:12px"><span>Apply config (paste JSON)</span><textarea id="gitops-json" rows="8" placeholder='{"groups":[...],"nodes":[...]}' style="width:100%;font-family:ui-monospace,monospace;font-size:12px"></textarea></label>
      <label class="check-row"><input type="checkbox" id="gitops-prune"><span><b>Prune</b><small>delete entities not in the doc (destructive)</small></span></label>
      <div class="rail-actions" style="margin-top:8px"><button class="button" data-action="apply-config-dry">Dry-run</button><button class="button primary" data-action="apply-config">Apply</button></div>
      <pre id="gitops-out" style="white-space:pre-wrap;font-size:11px;color:var(--muted);margin-top:10px"></pre>
      <p class="field-error" id="gitops-err"></p>
    </div>`);
  }
  async function exportConfig() {
    try { downloadText("honey-config.json", JSON.stringify(await api("/config/export"), null, 2)); toast("config exported"); }
    catch (error) { toast(error.message, true); }
  }
  async function applyConfig(dry) {
    const err = document.getElementById("gitops-err"), out = document.getElementById("gitops-out");
    if (err) err.textContent = "";
    let doc;
    try { doc = JSON.parse(document.getElementById("gitops-json").value); } catch { if (err) err.textContent = "invalid JSON"; return; }
    doc.dry_run = dry; doc.prune = document.getElementById("gitops-prune").checked;
    try {
      const r = await api("/config/apply", { method: "POST", body: JSON.stringify(doc) });
      if (out) out.textContent = JSON.stringify(r, null, 2);
      if (!dry) { toast("config applied"); await loadData({ quiet: true }); }
    } catch (error) { if (err) err.textContent = error.message; }
  }

  async function manageAnnouncements() {
    try {
      const list = await api("/announcements");
      const row = (a) => `<div class="check-row" style="justify-content:space-between"><span><b>${esc(a.title)}</b><small>${esc(a.level)} · ${a.enabled ? "live" : "off"} · ${dateLabel(a.created_at)}</small></span><span class="row-actions"><button class="row-button" data-action="toggle-announce" data-id="${a.id}" data-enabled="${a.enabled}">${a.enabled ? "hide" : "show"}</button><button class="row-button danger" data-action="delete-announce" data-id="${a.id}">delete</button></span></div>`;
      showList("Announcements", `<div class="form-body">
        ${list.length ? `<div class="check-list">${list.map(row).join("")}</div>` : '<p class="form-note">No announcements. The most recent live one is shown to users.</p>'}
        <div class="form-row" style="margin-top:12px"><label><span>Title</span><input id="an-title" placeholder="Scheduled maintenance" autocomplete="off"></label><label><span>Level</span><select id="an-level">${["info", "warning", "critical"].map((l) => `<option value="${l}">${l}</option>`).join("")}</select></label></div>
        <label><span>Body (optional)</span><input id="an-body" placeholder="Nodes may briefly restart at 02:00 UTC." autocomplete="off"></label>
        <button class="button primary" data-action="create-announce" style="margin-top:6px">${icon("plus")} Publish</button>
        <p class="form-note">The newest <b>live</b> announcement shows as a banner on every subscription page and the status page.</p>
        <p class="field-error" id="an-err"></p>
      </div>`);
    } catch (error) { toast(error.message, true); }
  }
  async function createAnnouncement() {
    const err = document.getElementById("an-err");
    const title = document.getElementById("an-title").value.trim();
    if (!title) { if (err) err.textContent = "title is required"; return; }
    try {
      await api("/announcements", { method: "POST", body: JSON.stringify({ title, body: document.getElementById("an-body").value.trim(), level: document.getElementById("an-level").value }) });
      toast("announcement published");
      manageAnnouncements();
    } catch (error) { if (err) err.textContent = error.message; }
  }

  const scheduleActions = {
    node: [["enable", "Enable node"], ["disable", "Disable node"], ["push", "Push desired state"]],
    user: [["enable", "Enable user"], ["disable", "Disable user"], ["reset-traffic", "Reset traffic"], ["rotate-sub", "Rotate subscription"]],
    inbound: [["enable", "Enable inbound"], ["disable", "Disable inbound"]],
  };
  function manageSchedule(kind, id) {
    const acts = scheduleActions[kind] || [];
    const soon = new Date(Date.now() + 3600_000).toISOString().slice(0, 16);
    showList("Schedule an operation", `<div class="form-body">
      <p class="form-note">Run a deferred action on this ${kind} at a future time (e.g. enable a plan from a date).</p>
      <div class="form-row"><label><span>Action</span><select id="sc-action">${acts.map(([v, l]) => `<option value="${v}">${esc(l)}</option>`).join("")}</select></label><label><span>Run at</span><input id="sc-when" type="datetime-local" value="${soon}"></label></div>
      <button class="button primary" data-action="save-schedule" data-kind="${kind}" data-id="${id}">Schedule</button>
      <p class="field-error" id="sc-err"></p>
    </div>`);
  }
  async function saveSchedule(kind, id) {
    const err = document.getElementById("sc-err");
    const when = document.getElementById("sc-when").value;
    if (!when) { if (err) err.textContent = "pick a time"; return; }
    const body = { resource_type: kind, resource_id: id, action: document.getElementById("sc-action").value, run_at: new Date(when).toISOString() };
    try {
      await api("/scheduled-ops", { method: "POST", body: JSON.stringify(body) });
      formDialog.close();
      toast("operation scheduled");
    } catch (error) { if (err) err.textContent = error.message; }
  }
  async function showScheduledOps() {
    try {
      const ops = await api("/scheduled-ops");
      const nameFor = (o) => {
        const e = o.resource_type === "node" ? state.nodes.find((n) => n.id === o.resource_id) : o.resource_type === "user" ? state.users.find((u) => u.id === o.resource_id) : state.inbounds.find((i) => i.id === o.resource_id);
        return e?.name || e?.username || e?.tag || o.resource_id.slice(0, 8);
      };
      const row = (o) => `<div class="check-row" style="justify-content:space-between"><span><b>${esc(o.action)} · ${esc(o.resource_type)} ${esc(nameFor(o))}</b><small>${esc(o.status)} · ${dateLabel(o.run_at)}${o.result ? " · " + esc(o.result) : ""}</small></span>${o.status === "pending" ? `<button class="row-button danger" data-action="cancel-op" data-id="${o.id}">cancel</button>` : ""}</div>`;
      showList("Scheduled operations", ops.length ? `<div class="form-body"><div class="check-list">${ops.map(row).join("")}</div></div>` : '<div class="empty"><div><h3>No scheduled operations</h3><p>Schedule one from a node, user or inbound page.</p></div></div>');
    } catch (error) { toast(error.message, true); }
  }
  async function showVersions(kind, id) {
    try {
      const versions = await api(`/${kind}s/${id}/history`);
      const row = (v, i) => `<div class="check-row" style="justify-content:space-between"><span><b>v${versions.length - i} · ${esc(v.actor || "system")}</b><small>${dateLabel(v.created_at)}</small></span>${i === 0 ? '<span class="onb-tag">current</span>' : `<button class="row-button" data-action="revert-version" data-kind="${kind}" data-id="${id}" data-version="${v.id}">revert</button>`}</div>`;
      showList(`Change history · ${kind}`, versions.length ? `<div class="form-body"><p class="form-note">Snapshots captured on each change (newest first). Revert re-applies a prior version and re-pushes. Secrets are preserved.</p><div class="check-list">${versions.map(row).join("")}</div></div>` : '<div class="empty"><div><h3>No history yet</h3><p>Edit this ${kind} to start capturing versions.</p></div></div>');
    } catch (error) { toast(error.message, true); }
  }
  async function revertVersion(kind, id, version) {
    if (!confirm("Revert to this version? Current settings will be overwritten (secrets preserved).")) return;
    try {
      await api(`/${kind}s/${id}/revert/${version}`, { method: "POST" });
      formDialog.close();
      toast("reverted");
      await loadData({ quiet: true });
    } catch (error) { toast(error.message, true); }
  }

  async function showAdmins() {
    const admins = await api("/admins");
    state.admins = admins;
    const row = (a) => {
      const caps = a.role === "reseller"
        ? ` · ${a.max_users ? a.max_users + " users max" : "unlimited users"}${a.user_traffic_ceiling_bytes ? " · ≤" + bytes(a.user_traffic_ceiling_bytes) + "/user" : ""}`
        : "";
      let usage = "";
      if (a.role === "reseller") {
        const own = state.users.filter((u) => u.created_by === a.id);
        const used = own.reduce((s, u) => s + Number(u.used_traffic_bytes || 0), 0);
        const budget = Number(a.traffic_limit_bytes || 0);
        usage = `<small class="reseller-usage">${own.length} user${own.length === 1 ? "" : "s"} · ${bytes(used)}${budget ? " / " + bytes(budget) + " budget" : " used"}${a.commission_percent ? " · " + a.commission_percent + "% commission" : ""}</small>`;
      }
      return `<div class="check-row" style="justify-content:space-between"><span><b>${esc(a.username)}</b><small>${esc(a.role)} · ${a.enabled ? "enabled" : "disabled"}${caps} · last login ${relativeTime(a.last_login_at)}</small>${usage}</span><span class="row-actions"><button class="row-button" data-action="manage-sessions" data-id="${a.id}">sessions</button><button class="row-button" data-action="edit-admin" data-id="${a.id}">edit</button></span></div>`;
    };
    showList("Administrators", `<div class="form-body">
      ${admins.length ? `<div class="check-list">${admins.map(row).join("")}</div>` : '<p class="form-note">No administrators yet.</p>'}
      <button class="button primary" data-action="add-admin-acct" style="margin-top:12px">${icon("plus")} Add account</button>
      <p class="form-note">Resellers are scoped sub-admins: they manage only their own users and can grant only the groups you entitle them to.</p>
    </div>`);
  }

  function loginOutcome(outcome) {
    const labels = {
      success: ["ok", "signed in"],
      bad_credentials: ["bad", "bad credentials"],
      bad_totp: ["bad", "bad 2FA code"],
      ip_denied: ["bad", "IP denied"],
      rate_limited: ["warn", "rate limited"],
    };
    const [cls, label] = labels[outcome] || ["", outcome];
    return `<span class="status ${cls}">${esc(label)}</span>`;
  }

  async function manageSessions(adminId = "") {
    const ownId = state.admin?.admin_id || "";
    const targetId = adminId || ownId;
    const query = targetId && targetId !== ownId ? `?admin_id=${encodeURIComponent(targetId)}` : "";
    const [sessions, history] = await Promise.all([
      api(`/auth/sessions${query}`),
      api(`/auth/login-history${query}`),
    ]);
    const title = sessions[0]?.username || state.admin?.username || "account";
    const sessionRows = sessions.length ? sessions.map((session) => `<div class="check-row session-row"><span><b>${session.current ? "Current session" : esc(session.user_agent || "Unknown client")}</b><small>${esc(session.remote_addr || "unknown address")} · created ${dateLabel(session.created_at)} · seen ${relativeTime(session.last_seen_at)} · expires ${dateLabel(session.expires_at)}</small>${session.current ? `<small>${esc(session.user_agent || "Unknown client")}</small>` : ""}</span><span class="row-actions">${session.current ? '<span class="status ok">current</span>' : ""}<button class="row-button danger" data-action="revoke-session" data-id="${session.id}" data-admin="${targetId}" data-current="${session.current}">${session.current ? "sign out" : "revoke"}</button></span></div>`).join("") : '<div class="empty" style="min-height:100px"><div><h3>No active sessions</h3></div></div>';
    const historyRows = history.length ? history.map((event) => `<div class="check-row session-row"><span><b>${esc(event.username)} · ${esc(event.remote_addr || "unknown address")}</b><small>${dateLabel(event.created_at)} · ${esc(event.user_agent || "Unknown client")}</small></span>${loginOutcome(event.outcome)}</div>`).join("") : '<p class="form-note">No retained login events yet.</p>';
    const ownTarget = targetId === ownId;
    showList(`Sessions · ${title}`, `<div class="form-body">
      <div style="display:flex;align-items:center;justify-content:space-between;gap:12px"><h4 style="margin:0">Active sessions</h4>${ownTarget ? '<button type="button" class="row-button danger" data-action="revoke-other-sessions">revoke all others</button>' : ""}</div>
      <div class="check-list">${sessionRows}</div>
      <h4 style="margin:8px 0 0">Login history</h4>
      <div class="check-list login-history">${historyRows}</div>
      <p class="form-note">History is retained for 90 days. Session credentials and token hashes are never returned by this view.</p>
    </div>`);
  }

  async function revokeSession(id, adminId, current) {
    if (!confirm(current ? "Sign out this current session?" : "Revoke this session?")) return;
    await api(`/auth/sessions/${id}`, { method: "DELETE" });
    if (current) {
      state.admin = null;
      state.loaded = false;
      state.notifications = [];
      state.notificationUnread = 0;
      stopNotificationPolling();
      $("#notification-center").hidden = true;
      formDialog.close();
      renderLocked();
      openLogin();
      return;
    }
    toast("session revoked");
    await manageSessions(adminId || "");
  }

  async function revokeOtherSessions() {
    if (!confirm("Revoke every other session for this account?")) return;
    const result = await api("/auth/sessions/revoke-others", { method: "POST" });
    toast(`${result.revoked} session(s) revoked`);
    await manageSessions();
  }

  function adminForm(admin) {
    const editing = Boolean(admin);
    const role = admin?.role || "reseller";
    const roles = ["owner", "admin", "operator", "viewer", "reseller"];
    const ceilingGb = admin?.user_traffic_ceiling_bytes ? Number(admin.user_traffic_ceiling_bytes) / 1024 ** 3 : 0;
    const groupChecks = state.groups.length
      ? state.groups.map((g) => `<label class="check-row"><input type="checkbox" data-agroup="${g.id}"><span><b>${esc(g.name)}</b><small>${g.is_default ? "default" : ""}</small></span></label>`).join("")
      : '<div class="check-row"><span><small>no groups yet — create some in Groups</small></span></div>';
    showList(editing ? `Edit · ${esc(admin.username)}` : "Add account", `<div class="form-body">
      <label><span>Username</span><input id="adm-user" value="${esc(admin?.username || "")}" ${editing ? "readonly" : ""} placeholder="reseller1" autocomplete="off"></label>
      <label><span>${editing ? "New password (blank = keep)" : "Password"}</span><input id="adm-pass" type="password" placeholder="${editing ? "leave unchanged" : "min 10 chars"}" autocomplete="new-password"></label>
      <label><span>Role</span><select id="adm-role">${roles.map((r) => `<option value="${r}" ${r === role ? "selected" : ""}>${r}</option>`).join("")}</select></label>
      ${editing ? `<label><span>Enabled</span><select id="adm-enabled"><option value="true" ${admin.enabled ? "selected" : ""}>on</option><option value="false" ${!admin.enabled ? "selected" : ""}>off</option></select></label>` : ""}
      ${editing ? `<label><span>Custom role (RBAC, overrides rank)</span><select id="adm-customrole"><option value="">— none (use rank role) —</option>${(state.customRoles || []).map((r) => `<option value="${r.id}" ${admin.custom_role_id === r.id ? "selected" : ""}>${esc(r.name)}</option>`).join("")}</select></label>` : ""}
      <div id="adm-reseller" style="${role === "reseller" ? "" : "display:none"}">
        <div class="form-row"><label><span>Max users (0 = ∞)</span><input id="adm-maxusers" type="number" min="0" step="1" value="${admin?.max_users || 0}"></label><label><span>Per-user limit, GB (0 = ∞)</span><input id="adm-ceiling" type="number" min="0" step=".01" value="${ceilingGb}"></label></div>
        <div class="form-row"><label><span>Total budget, GB (0 = ∞)</span><input id="adm-budget" type="number" min="0" step=".01" value="${admin?.traffic_limit_bytes ? Number(admin.traffic_limit_bytes) / 1024 ** 3 : 0}"></label><label><span>Commission, %</span><input id="adm-commission" type="number" min="0" max="100" step="1" value="${admin?.commission_percent || 0}"></label></div>
        <p class="form-note" style="margin:6px 0">Groups this reseller may sell:</p>
        <div class="check-list">${groupChecks}</div>
      </div>
      <button class="button primary" data-action="save-admin" ${editing ? `data-id="${admin.id}"` : ""} style="margin-top:12px">${editing ? "Save" : "Create"}</button>
      <p class="field-error" id="adm-err"></p>
    </div>`);
    const roleSel = document.getElementById("adm-role");
    const box = document.getElementById("adm-reseller");
    roleSel?.addEventListener("change", () => { box.style.display = roleSel.value === "reseller" ? "" : "none"; });
    if (editing && admin.role === "reseller") {
      api(`/admins/${admin.id}/groups`).then((ids) => {
        const set = new Set(ids);
        document.querySelectorAll("[data-agroup]").forEach((el) => { if (set.has(el.dataset.agroup)) el.checked = true; });
      }).catch(() => {});
    }
  }

  async function saveAdmin(id) {
    const err = document.getElementById("adm-err");
    const role = document.getElementById("adm-role").value;
    const pass = document.getElementById("adm-pass").value;
    const body = { role };
    if (pass) body.password = pass;
    if (role === "reseller") {
      body.max_users = Number(document.getElementById("adm-maxusers").value || 0);
      body.user_traffic_ceiling_bytes = Math.round(Number(document.getElementById("adm-ceiling").value || 0) * 1024 ** 3);
      body.traffic_limit_bytes = Math.round(Number(document.getElementById("adm-budget").value || 0) * 1024 ** 3);
      body.commission_percent = Number(document.getElementById("adm-commission").value || 0);
      body.group_ids = Array.from(document.querySelectorAll("[data-agroup]:checked")).map((el) => el.dataset.agroup);
    }
    try {
      if (id) {
        const en = document.getElementById("adm-enabled");
        if (en) body.enabled = en.value === "true";
        const cr = document.getElementById("adm-customrole");
        if (cr) body.custom_role_id = cr.value || null;
        await api(`/admins/${id}`, { method: "PATCH", body: JSON.stringify(body) });
      } else {
        body.username = document.getElementById("adm-user").value.trim();
        if (!body.username || !pass) { if (err) err.textContent = "username and password are required"; return; }
        await api("/admins", { method: "POST", body: JSON.stringify(body) });
      }
      formDialog.close();
      toast("account saved");
    } catch (error) { if (err) err.textContent = error.message; }
  }

  async function showAudit() {
    const events = await api("/audit");
    const list = events.length
      ? `<div class="check-list audit-list">${events.map((event) => `<div class="check-row"><span><b>${esc(event.action)} · ${esc(event.resource_type)}</b><small>${esc(event.actor_name || "system")} · ${dateLabel(event.created_at)}${event.resource_id ? " · " + esc(event.resource_id) : ""}</small></span></div>`).join("")}</div>`
      : '<div class="empty"><div><h3>No audit events yet</h3></div></div>';
    showList("Audit log", `<div class="form-body"><div class="rail-actions" style="margin-bottom:10px"><button class="button secondary" data-action="verify-audit">${icon("check")} Verify tamper-evidence</button><span id="audit-verify" class="form-note"></span></div>${list}</div>`);
  }
  async function verifyAudit() {
    const el = document.getElementById("audit-verify");
    if (el) el.textContent = "checking…";
    try {
      const r = await api("/audit/verify");
      if (el) el.innerHTML = r.intact
        ? `<span class="status ok">chain intact · ${r.verified_entries} entries</span>`
        : `<span class="status bad">TAMPERING at id ${r.broken_at}</span>`;
    } catch (error) { if (el) el.textContent = error.message; }
  }

  async function showNodeHistory(node, kind) {
    const rows = await api(kind === "certs" ? `/nodes/${node.id}/certificates` : `/nodes/${node.id}/pushes`);
    if (kind === "certs") {
      showList(`Certificates · ${node.name}`, rows.length
        ? `<div class="check-list">${rows.map((cert) => `<div class="check-row"><span><b>${esc(cert.serial_number)}</b><small>expires ${dateLabel(cert.not_after)} · ${cert.revoked_at ? "revoked" : "active"} · ${esc(cert.fingerprint_sha256)}</small></span>${cert.revoked_at ? '<span class="status bad">revoked</span>' : `<button type="button" class="row-button danger" data-action="revoke-cert" data-id="${esc(cert.id)}" data-node="${esc(node.id)}">revoke</button>`}</div>`).join("")}</div><p class="form-note" style="margin-top:10px">Revocation immediately evicts the live channel. Enroll a replacement certificate before revoking the current one when uninterrupted access matters.</p>`
        : '<div class="empty"><div><h3>No issued certificates</h3><p>Use enroll to issue the first node identity.</p></div></div>');
      return;
    }
    showList(`Push history ? ${node.name}`, rows.length
      ? `<div class="check-list">${rows.map((push) => `<div class="check-row"><span><b>${esc(push.status)} ? ${esc(push.source)}</b><small>${dateLabel(push.started_at)} ? ${esc(push.desired_hash.slice(0, 16))}?${push.message ? " ? " + esc(push.message) : ""}</small></span></div>`).join("")}</div>`
      : '<div class="empty"><div><h3>No pushes recorded</h3></div></div>');
  }

  async function runAction(action, element) {
    try {
      if (action === "toggle-notifications") return toggleNotifications();
      if (action === "mark-all-notifications") return markAllNotificationsRead();
      if (action === "open-notification") return openNotification(element);
      if (action === "new-saved-view") return openSavedViewName(element.dataset.resource, "create");
      if (action === "commit-saved-view") return commitSavedView(element.dataset.resource, element.dataset.mode, element.dataset.id);
      if (action === "update-saved-view") return updateSavedView(element.dataset.resource, element.dataset.id);
      if (action === "rename-saved-view") return openSavedViewName(element.dataset.resource, "rename", element.dataset.id);
      if (action === "delete-saved-view") return deleteSavedView(element.dataset.resource, element.dataset.id);
      if (action === "edit-labels") return editLabels(element.dataset.kind, element.dataset.id);
      if (action === "save-labels") return saveLabels(element.dataset.kind, element.dataset.id);
      if (action === "add-node") { state.nodeEdit = null; return go("new-node"); }
      if (action === "add-inbound") { state.wiz = wizDefaults(); return go("new-inbound"); }
      if (action === "add-user") { state.userWiz = userWizDefaults(); return go("new-user"); }
      if (action?.startsWith("batch-")) return runBatch(action.slice(6), element?.dataset.resource);
      if (action?.startsWith("add-")) return openEntity(action.slice(4));
      if (action === "gen-reality") return genReality();
      if (action === "preset-cf-ws") return applyCdnPreset("ws");
      if (action === "preset-cf-xhttp") return applyCdnPreset("xhttp");
      if (action === "create-inbound") return createInboundFromWiz();
      if (action === "create-node") return createNodeFromWiz();
      if (action === "save-user-wiz") return saveUserWizard();
      if (action === "generate-wiz-password") return generateWizardPassword();
      if (action === "refresh") return loadData();
      if (action === "traffic-range") { state.trafficAnalytics.range = element?.dataset.range || "24h"; state.trafficAnalytics.data = null; return loadTrafficAnalytics(); }
      if (action === "traffic-refresh") { state.trafficAnalytics.data = null; return loadTrafficAnalytics(); }
      if (action === "live-refresh") { loadGeo(); return loadLiveConnections(); }
      if (action === "geo-refresh") return loadGeo();
      if (action === "show-ha") return showHa();
      if (action === "show-update") return showUpdate();
      if (action === "apply-update") {
        if (!confirm("Download and install the latest master? The process will need a restart (your supervisor handles it).")) return;
        try {
          const r = await api("/update/apply", { method: "POST" });
          showResult("Update staged", [["New version", r.staged_version], ["Restart", "required — supervisor will restart the process"]]);
        } catch (error) { toast(error.message, true); }
        return;
      }
      if (action === "refresh-metrics") return loadNodeMetrics(id);
      if (action === "refresh-drift") return loadNodeDrift(id);
      if (action === "preflight-node") return runPreflight(id);
      if (action === "benchmark-node") return runBenchmark(id);
      if (action === "manage-wg") return manageWg(id);
      if (action === "manage-services") return manageServices(id);
      if (action === "create-service") {
        const g = (k) => document.getElementById(k);
        const kind = g("svc-kind").value;
        const config = kind === "mtproto"
          ? {
              host: g("svc-host").value.trim() || "www.cloudflare.com",
              concurrency: Math.max(0, Math.round(Number(g("svc-mt-conc").value || 0))),
              prefer_ip: g("svc-mt-ip").value,
              domain_fronting_port: Math.max(0, Math.round(Number(g("svc-mt-dfport").value || 0))),
              anti_replay: g("svc-mt-replay").value === "true",
            }
          : { username: g("svc-user").value.trim() || "user", domain: g("svc-domain").value.trim() };
        const body = { kind, name: g("svc-name").value.trim(), listen_port: Number(g("svc-port").value), config };
        if (!body.name) { const e = g("svc-err"); if (e) e.textContent = "name is required"; return; }
        await api(`/nodes/${id}/services`, { method: "POST", body: JSON.stringify(body) });
        toast("service created");
        return manageServices(id);
      }
      if (action === "delete-service") {
        await api(`/services/${element.dataset.svcid}`, { method: "DELETE" });
        toast("service removed");
        return manageServices(id);
      }
      if (action === "toggle-service") {
        await api(`/services/${element.dataset.svcid}`, { method: "PATCH", body: JSON.stringify({ enabled: element.dataset.enabled !== "true" }) });
        return manageServices(id);
      }
      if (action === "create-wg") {
        const g = (k) => document.getElementById(k);
        const body = {
          name: g("wg-name").value.trim(),
          listen_port: Number(g("wg-port").value),
          address_cidr: g("wg-cidr").value.trim(),
          dns: g("wg-dns").value.trim(),
          mtu: Number(g("wg-mtu").value),
          amnezia: g("wg-amnezia").value === "true",
          endpoint_host: g("wg-endpoint").value.trim() || null,
        };
        if (!body.name) { const e = g("wg-err"); if (e) e.textContent = "name is required"; return; }
        await api(`/nodes/${id}/wireguard`, { method: "POST", body: JSON.stringify(body) });
        toast("wireguard interface created");
        return manageWg(id);
      }
      if (action === "delete-wg") {
        await api(`/wireguard/${element.dataset.wgid}`, { method: "DELETE" });
        toast("wireguard interface removed");
        return manageWg(id);
      }
      if (action === "toggle-wg") {
        await api(`/wireguard/${element.dataset.wgid}`, { method: "PATCH", body: JSON.stringify({ enabled: element.dataset.enabled !== "true" }) });
        return manageWg(id);
      }
      if (action === "traffic-csv") return exportTrafficAnalytics();
      if (action === "traffic-report") { window.open(`/reports/period?${trafficQueryParams()}`, "_blank", "noopener"); return; }
      if (action === "open-log-search") {
        state.runtimeLogFilters = { ...state.runtimeLogFilters, code: element?.dataset.code || "" };
        return go("logs");
      }
      if (action === "open-login") return openLogin();
      if (action === "logout") return logout();
      if (action === "theme") return toggleTheme();
      if (action === "manage-sessions") return manageSessions(element?.dataset.id || "");
      if (action === "revoke-session") return revokeSession(element.dataset.id, element.dataset.admin, element.dataset.current === "true");
      if (action === "revoke-other-sessions") return revokeOtherSessions();
      if (action === "show-admins") return showAdmins();
      if (action === "export-csv") return exportCsv(element?.dataset.resource);
      if (action === "manage-settings") return manageRuntimeSettings();
      if (action === "manage-api-keys") return manageApiKeys();
      if (action === "create-api-key") return createApiKey();
      if (action === "revoke-api-key") return revokeApiKey(element?.dataset.id);
      if (action === "manage-branding") return manageBranding();
      if (action === "save-branding") return saveBranding();
      if (action === "manage-custom-roles") return manageCustomRoles();
      if (action === "new-custom-role") return customRoleForm(null);
      if (action === "edit-custom-role") return customRoleForm((state.customRoles || []).find((r) => r.id === id));
      if (action === "save-custom-role") return saveCustomRole(element?.dataset.id);
      if (action === "delete-custom-role") { if (!confirm("Delete this custom role? Admins using it revert to their rank role.")) return; try { await api(`/custom-roles/${id}`, { method: "DELETE" }); manageCustomRoles(); } catch (e) { toast(e.message, true); } return; }
      if (action === "manage-import") return manageImport();
      if (action === "run-import") return runImport();
      if (action === "manage-gitops") return manageGitops();
      if (action === "export-config") return exportConfig();
      if (action === "apply-config-dry") return applyConfig(true);
      if (action === "apply-config") return applyConfig(false);
      if (action === "manage-announcements") return manageAnnouncements();
      if (action === "create-announce") return createAnnouncement();
      if (action === "toggle-announce") { try { await api(`/announcements/${id}`, { method: "PATCH", body: JSON.stringify({ enabled: element.dataset.enabled !== "true" }) }); manageAnnouncements(); } catch (e) { toast(e.message, true); } return; }
      if (action === "delete-announce") { if (!confirm("Delete this announcement?")) return; try { await api(`/announcements/${id}`, { method: "DELETE" }); manageAnnouncements(); } catch (e) { toast(e.message, true); } return; }
      if (action === "show-scheduled") return showScheduledOps();
      if (action === "schedule-op") return manageSchedule(element?.dataset.kind, id);
      if (action === "save-schedule") return saveSchedule(element?.dataset.kind, id);
      if (action === "cancel-op") { try { await api(`/scheduled-ops/${id}`, { method: "DELETE" }); toast("canceled"); showScheduledOps(); } catch (e) { toast(e.message, true); } return; }
      if (action === "entity-history") return showVersions(element?.dataset.kind, id);
      if (action === "revert-version") return revertVersion(element?.dataset.kind, id, element?.dataset.version);
      if (action === "save-settings") return saveRuntimeSettings();
      if (action === "generate-user-password") return generateUserPassword();
      if (action === "add-admin-acct") return adminForm(null);
      if (action === "edit-admin") return adminForm((state.admins || []).find((a) => a.id === element?.dataset.id));
      if (action === "save-admin") return saveAdmin(element?.dataset.id);
      if (action === "show-audit") return showAudit();
      if (action === "verify-audit") return verifyAudit();
      if (action === "gdpr-export") {
        try {
          const dump = await api(`/users/${id}/gdpr-export`);
          downloadText(`gdpr-${id}.json`, JSON.stringify(dump, null, 2));
          toast("GDPR export downloaded");
        } catch (e) { toast(e.message, true); }
        return;
      }
      if (action === "gdpr-erase") {
        if (!confirm("GDPR erase: permanently delete this user and all their data? This cannot be undone.")) return;
        try {
          await api(`/users/${id}/gdpr-erase`, { method: "POST" });
          toast("user erased");
          go("users");
          await loadData({ quiet: true });
        } catch (e) { toast(e.message, true); }
        return;
      }
      if (action === "manage-2fa") return manage2fa();
      if (action === "totp-enable") return totpEnable();
      if (action === "totp-disable") return totpDisable();
      if (action === "generate-recovery") return generateRecoveryCodes();
      if (action === "manage-ips") return manageIps();
      if (action === "add-ip") return addIp();
      if (action === "manage-notifications") return manageNotifications();
      if (action === "add-channel") return addChannel();
      if (action === "manage-telegram") return manageTelegram();
      if (action === "add-tgchat") return addTgChat();
      const id = element?.dataset.id;
      const node = state.nodes.find((item) => item.id === id);
      const user = state.users.find((item) => item.id === id);
      const inbound = state.inbounds.find((item) => item.id === id);
      if (action === "edit-node") { state.nodeEdit = node; return go("new-node"); }
      if (action === "edit-user") { state.userWiz = userWizDefaults(user); return go("new-user"); }
      if (action === "edit-inbound") { state.wiz = wizFromInbound(inbound); return go("new-inbound"); }
      if (action === "verify-domain") return verifyDomain(id);
      if (action === "probe-inbound") return probeInbound(id);
      if (action === "rotate-sni") {
        try {
          const r = await api(`/inbounds/${id}/rotate-sni`, { method: "POST" });
          toast(`SNI rotated to ${r.server_name}`);
          await loadData({ quiet: true });
        } catch (error) { toast(error.message, true); }
        return;
      }
      if (action === "edit-domain") return openEntity("domain", state.domains.find((d) => d.id === id));
      if (action === "default-profile") return setDefaultProfile(id);
      if (action === "edit-profile") return openEntity("profile", state.profiles.find((p) => p.id === id));
      if (action === "delete-profile") return deleteProfile(id);
      if (action === "edit-group") return openEntity("group", state.groups.find((g) => g.id === id));
      if (action === "node-groups") return manageGroups("node", id);
      if (action === "user-groups") return manageGroups("user", id);
      if (action === "save-groups") return saveGroups(element.dataset.kind, id);
      if (action === "set-alias") return manageAlias(id);
      if (action === "save-alias") return saveAlias(id);
      if (action === "del-ip") return delIp(id);
      if (action === "test-channel") return testChannel(id);
      if (action === "del-channel") return delChannel(id);
      if (action === "del-tgchat") return delTgChat(id);
      if (action === "node-history") return showNodeHistory(node, "pushes");
      if (action === "node-certs") return showNodeHistory(node, "certs");
      if (action === "revoke-cert") {
        if (!confirm("Revoke this node certificate? Its active channel will be disconnected immediately.")) return;
        await api(`/certificates/${id}/revoke`, { method: "POST" });
        toast("certificate revoked");
        const certNode = state.nodes.find((item) => item.id === element?.dataset.node);
        if (certNode) return showNodeHistory(certNode, "certs");
        return;
      }
      if (action === "dry-run-node") return dryRunNode(node);
      if (action?.startsWith("delete-")) {
        const kind = action.slice(7);
        const entity = kind === "node" ? node : kind === "user" ? user : kind === "group" ? state.groups.find((g) => g.id === id) : inbound;
        const label = entity?.name || entity?.username || entity?.tag || kind;
        if (!confirm(`Delete ${label}? This cannot be undone.`)) return;
        await api(`/${kind === "inbound" ? "inbounds" : kind + "s"}/${id}`, { method: "DELETE" });
        toast(`${kind} deleted`);
        if (state.detailId === id) go(kind === "inbound" ? "inbounds" : `${kind}s`);
        return loadData({ quiet: true });
      }
      if (action === "enroll-node") {
        const result = await api(`/nodes/${id}/enrollments`, {
          method: "POST",
          body: JSON.stringify({ expires_in_minutes: 30 })
        });
        return showResult("Node enrollment", [
          ["One-time token", result.token],
          ["Install command", result.install_command],
          ["Expires", result.expires_at]
        ]);
      }
      if (action === "rotate-credentials") {
        if (!confirm(`Rotate credentials for ${user?.username}? Existing client configs will stop working.`)) return;
        const result = await api(`/users/${id}/rotate`, { method: "POST", body: "{}" });
        showResult("Credentials rotated", [["UUID", result.uuid], ["Password", result.password]]);
        await loadData({ quiet: true });
        return;
      }
      if (action === "push-node") {
        return previewNode(node);
      } else if (action === "apply-node") {
        await api(`/nodes/${id}/push`, { method: "POST" });
        formDialog.close();
        toast("spec pushed to node");
        await loadData({ quiet: true });
      } else if (action === "reset-traffic") {
        await api(`/users/${id}/reset-traffic`, { method: "POST" });
        toast("traffic reset");
        await loadData({ quiet: true });
      } else if (action === "reveal-sub") {
        const result = await api(`/users/${id}/subscription`);
        const rows = [["Permanent subscription", location.origin + result.subscription_path]];
        if (result.revocable_subscription_path) rows.push(["Optional revocable link", location.origin + result.revocable_subscription_path]);
        showResult("Subscription links", rows);
      } else if (action === "preview-sub") {
        return previewSubscription(id);
      } else if (action === "manage-subs") {
        return manageNamedSubs(id);
      } else if (action === "reveal-named-sub") {
        const result = await api(`/users/${id}/subscriptions/${element.dataset.sid}`);
        if (!result.subscription_token) { toast("no stored link — recreate this named link", true); return; }
        showResult("Named subscription link", [["Subscription", location.origin + result.subscription_path]]);
      } else if (action === "delete-named-sub") {
        await api(`/users/${id}/subscriptions/${element.dataset.sid}`, { method: "DELETE" });
        toast("named link revoked");
        return manageNamedSubs(id);
      } else if (action === "create-named-sub") {
        const input = document.getElementById("named-sub-input");
        const name = (input?.value || "").trim();
        if (!name) { const el = document.getElementById("named-sub-err"); if (el) el.textContent = "name is required"; return; }
        const result = await api(`/users/${id}/subscriptions`, { method: "POST", body: JSON.stringify({ name }) });
        showResult("Named link created", [["Name", result.name], ["Subscription", location.origin + result.subscription_path]]);
      } else if (action === "rotate-sub") {
        if (!confirm("Create a new optional revocable link? The previous revocable link will stop working; the permanent UUID link remains valid.")) return;
        const result = await api(`/users/${id}/rotate-sub`, { method: "POST" });
        showResult("Revocable subscription updated", [["Permanent subscription", `${location.origin}/sub/${id}`], ["Revocable subscription", location.origin + result.subscription_path]]);
      } else if (action === "toggle-node") {
        await api(`/nodes/${id}`, { method: "PATCH", body: JSON.stringify({ enabled: element.dataset.enabled !== "true" }) });
        await loadData({ quiet: true });
      } else if (action === "toggle-maintenance") {
        const on = element.dataset.maint !== "true";
        await api(`/nodes/${id}`, { method: "PATCH", body: JSON.stringify({ maintenance: on }) });
        toast(on ? "node draining (maintenance)" : "node back in service");
        await loadData({ quiet: true });
      } else if (action === "toggle-user") {
        await api(`/users/${id}`, { method: "PATCH", body: JSON.stringify({ enabled: element.dataset.enabled !== "true" }) });
        await loadData({ quiet: true });
      }
    } catch (error) {
      if (error.auth) openLogin();
      toast(error.message, true);
    }
  }

  function openLogin() {
    renderLocked();
  }

  async function login() {
    const form = $("#token-form");
    const totpRow = $("#totp-row");
    const recoveryRow = $("#recovery-row");
    const recoveryToggle = $("#recovery-toggle");
    const body = {
      username: form.elements.username.value.trim(),
      password: form.elements.password.value,
    };
    const code = form.elements.totp_code?.value.trim();
    if (code) body.totp_code = code;
    const recoveryCode = form.elements.recovery_code?.value.trim();
    if (recoveryCode) body.recovery_code = recoveryCode;
    try {
      const response = await fetch("/auth/login", {
        method: "POST",
        credentials: "same-origin",
        headers: { "content-type": "application/json", accept: "application/json" },
        body: JSON.stringify(body),
      });
      const data = response.status === 204 ? {} : await response.json().catch(() => ({}));
      if (!response.ok) {
        if (data && data.totp_required) {
          totpRow.hidden = false;
          recoveryToggle.hidden = !(data.recovery_available);
          $("#token-error").textContent = "enter your two-factor code";
          requestAnimationFrame(() => form.elements.totp_code.focus());
          return;
        }
        throw new Error(data?.error || `login failed (${response.status})`);
      }
      state.admin = data.admin;
      form.reset();
      totpRow.hidden = true;
      recoveryRow.hidden = true;
      recoveryToggle.hidden = true;
      state.loaded = false;
      await loadData();
    } catch (error) {
      $("#token-error").textContent = error.message;
    }
  }

  async function logout() {
    await api("/auth/logout", { method: "POST" });
    state.admin = null;
    state.notifications = [];
    state.notificationUnread = 0;
    stopNotificationPolling();
    $("#notification-center").hidden = true;
    $("#token-button").textContent = t("sign in");
    state.loaded = false;
    renderLocked();
  }

  function toggleTheme() {
    const next = document.documentElement.dataset.theme === "dark" ? "light" : "dark";
    document.documentElement.dataset.theme = next;
    writeStore(localStorage, "honey-theme", next);
    document.querySelector('meta[name="theme-color"]').content = next === "dark" ? "#080808" : "#ffffff";
  }

  function bindTableFilter() {
    const input = $("[data-table-filter]", view);
    if (!input) return;
    const toolbar = input.closest("[data-view-resource]");
    const resource = toolbar?.dataset.viewResource;
    const labelInput = $("[data-label-filter]", toolbar || view);
    const apply = () => {
      const query = input.value.trim().toLowerCase();
      const labels = (labelInput?.value || "").split(",").map((label) => label.trim().toLowerCase()).filter(Boolean);
      if (resource) {
        viewConfig(resource).search = input.value;
        viewConfig(resource).labels = labels;
      }
      $$("tbody tr[data-search]", view).forEach((row) => {
        const labelText = row.dataset.labels || row.dataset.search;
        row.hidden = Boolean((query && !row.dataset.search.includes(query)) || labels.some((label) => !labelText.split("|").includes(label) && !labelText.includes(label)));
      });
    };
    input.addEventListener("input", apply);
    labelInput?.addEventListener("input", apply);
    if (resource) applyColumnVisibility(resource);
    apply();
  }

  function applyColumnVisibility(resource) {
    const selected = viewConfig(resource).columns;
    tableColumns[resource].forEach(([key], index) => {
      const hidden = !selected.includes(key);
      $$(`.table-shell tr`, view).forEach((row) => {
        if (row.closest("tbody") && row.cells.length !== tableColumns[resource].length) return;
        if (row.cells[index]) row.cells[index].hidden = hidden;
      });
    });
  }

  function isNodeOnline(node) {
    return Boolean(node.enabled && node.last_seen && Date.now() - new Date(node.last_seen).getTime() < 120000);
  }

  function bytes(value) {
    let number = Number(value || 0);
    const units = ["B", "KB", "MB", "GB", "TB", "PB"];
    let unit = 0;
    while (Math.abs(number) >= 1024 && unit < units.length - 1) { number /= 1024; unit += 1; }
    const digits = unit === 0 || Math.abs(number) >= 100 ? 0 : Math.abs(number) >= 10 ? 1 : 2;
    return `${number.toFixed(digits)} ${units[unit]}`;
  }

  function relativeTime(value) {
    if (!value) return "never";
    const seconds = Math.max(0, Math.round((Date.now() - new Date(value).getTime()) / 1000));
    if (seconds < 60) return `${seconds}s ago`;
    if (seconds < 3600) return `${Math.floor(seconds / 60)}m ago`;
    if (seconds < 86400) return `${Math.floor(seconds / 3600)}h ago`;
    return `${Math.floor(seconds / 86400)}d ago`;
  }

  function expiryDays(value) {
    if (!value) return 0;
    const remaining = new Date(value).getTime() - Date.now();
    if (!Number.isFinite(remaining)) return 0;
    return Math.max(0, Math.ceil(remaining / 86_400_000));
  }

  function dateLabel(value) {
    if (!value) return "Never";
    return new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" }).format(new Date(value));
  }

  function csvCell(value) {
    const s = value === null || value === undefined ? "" : String(value);
    return /[",\n\r]/.test(s) ? `"${s.replace(/"/g, '""')}"` : s;
  }
  function toCsv(headers, rows) {
    const lines = [headers.map(csvCell).join(",")];
    for (const row of rows) lines.push(row.map(csvCell).join(","));
    return "﻿" + lines.join("\r\n"); // BOM for Excel
  }
  function downloadCsv(filename, csv) {
    const blob = new Blob([csv], { type: "text/csv;charset=utf-8" });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = filename;
    document.body.append(a);
    a.click();
    a.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  }
  function isoOrEmpty(value) { return value ? new Date(value).toISOString() : ""; }
  function exportCsv(resource) {
    const stamp = new Date().toISOString().slice(0, 10);
    let headers, rows, name;
    if (resource === "nodes") {
      name = `honey-nodes-${stamp}.csv`;
      headers = ["name", "address", "grpc_port", "transport", "enabled", "online", "agent_version", "singbox_version", "xray_version", "last_seen", "labels", "created_at"];
      rows = sortResource("nodes", state.nodes).map((n) => [n.name, n.address, n.grpc_port, n.transport, n.enabled, isNodeOnline(n), n.agent_version || "", n.singbox_version || "", n.xray_version || "", isoOrEmpty(n.last_seen), (n.labels || []).join("|"), isoOrEmpty(n.created_at)]);
    } else if (resource === "inbounds") {
      const nodeMap = new Map(state.nodes.map((n) => [n.id, n.name]));
      name = `honey-inbounds-${stamp}.csv`;
      headers = ["tag", "node", "protocol", "core", "listen", "listen_port", "network", "tls", "reality", "enabled", "flow", "server_name", "labels"];
      rows = sortResource("inbounds", state.inbounds).map((i) => [i.tag, nodeMap.get(i.node_id) || "", i.kind, i.core, i.listen, i.listen_port, i.network || "tcp", i.tls_enabled, i.reality, i.enabled, i.flow || "", i.server_name || "", (i.labels || []).join("|")]);
    } else if (resource === "traffic") {
      name = `honey-traffic-${stamp}.csv`;
      headers = ["username", "used_bytes", "limit_bytes", "used_human", "limit_human", "percent", "quota_interval", "active"];
      rows = [...state.users].sort((a, b) => Number(b.used_traffic_bytes) - Number(a.used_traffic_bytes)).map((u) => {
        const used = Number(u.used_traffic_bytes || 0), limit = Number(u.traffic_limit_bytes || 0);
        return [u.username, used, limit, bytes(used), limit ? bytes(limit) : "unlimited", limit ? Math.round(Math.min(100, used / limit * 100)) : "", u.quota_interval || "none", u.active];
      });
    } else {
      name = `honey-users-${stamp}.csv`;
      headers = ["username", "uuid", "status", "enabled", "used_bytes", "limit_bytes", "used_human", "limit_human", "quota_interval", "expires_at", "created_at", "labels"];
      rows = sortResource("users", state.users).map((u) => {
        const used = Number(u.used_traffic_bytes || 0), limit = Number(u.traffic_limit_bytes || 0);
        return [u.username, u.uuid, u.active ? "active" : (u.suppressed_reason || "inactive"), u.enabled, used, limit, bytes(used), limit ? bytes(limit) : "unlimited", u.quota_interval || "none", isoOrEmpty(u.expires_at), isoOrEmpty(u.created_at), (u.labels || []).join("|")];
      });
    }
    if (!rows.length) return toast("nothing to export", true);
    downloadCsv(name, toCsv(headers, rows));
    toast(`exported ${rows.length} row${rows.length === 1 ? "" : "s"}`);
  }

  function toast(message, bad = false) {
    const node = document.createElement("div");
    node.className = `toast ${bad ? "bad" : ""}`;
    node.textContent = message;
    $("#toasts").append(node);
    setTimeout(() => node.remove(), 4200);
  }

  document.addEventListener("click", (event) => {
    if (!event.target.closest("#notification-center")) closeNotifications();
    const route = event.target.closest("[data-route]");
    if (route) { go(route.dataset.route, undefined, route.dataset.section); return; }
    const open = event.target.closest("[data-open]");
    if (open) { const [dest, id] = open.dataset.open.split("/"); go(dest, id || undefined); return; }
    const scope = event.target.closest("[data-scope]");
    if (scope) { go(state.route, state.detailId, scope.dataset.scope); return; }
    const wizSet = event.target.closest("[data-wiz-set]");
    if (wizSet) { if (state.wiz) { state.wiz[wizSet.dataset.wizSet] = wizSet.dataset.val; repaintWiz(); } return; }
    const groupToggle = event.target.closest("[data-group-toggle]");
    if (groupToggle) { toggleGroup(groupToggle.dataset.groupToggle); return; }
    if (event.target.closest("[data-command-open]")) { openCommand(); return; }
    const action = event.target.closest("[data-action]");
    if (action) { runAction(action.dataset.action, action); return; }
    const copy = event.target.closest("[data-copy]");
    if (copy) navigator.clipboard.writeText(copy.dataset.copy).then(() => toast("copied")).catch(() => toast("copy failed", true));
  });

  document.addEventListener("change", (event) => {
    const language = event.target.closest("[data-language-select]");
    if (language) { setLang(language.value); return; }
    const notificationFilter = event.target.closest("[data-notification-filter]");
    if (notificationFilter) {
      const key = notificationFilter.dataset.notificationFilter;
      state.notificationFilters[key] = notificationFilter.type === "checkbox" ? notificationFilter.checked : notificationFilter.value;
      refreshNotifications();
      return;
    }
    const sel = event.target.closest("[data-sel]");
    if (sel) { toggleSelection(sel.dataset.sel, sel.dataset.id, sel.checked); return; }
    const selAll = event.target.closest("[data-sel-all]");
    if (selAll) { toggleSelectAll(selAll.dataset.selAll, selAll.checked); return; }
    const savedView = event.target.closest("[data-saved-view]");
    if (savedView) { applySavedView(savedView.closest("[data-view-resource]").dataset.viewResource, savedView.value); return; }
    const viewSort = event.target.closest("[data-view-sort]");
    if (viewSort) { viewConfig(viewSort.closest("[data-view-resource]").dataset.viewResource).sort = viewSort.value; render(); return; }
    const viewColumn = event.target.closest("[data-view-column]");
    if (viewColumn) {
      const resource = viewColumn.closest("[data-view-resource]").dataset.viewResource;
      const columns = new Set(viewConfig(resource).columns);
      viewColumn.checked ? columns.add(viewColumn.dataset.viewColumn) : columns.delete(viewColumn.dataset.viewColumn);
      if (!columns.size) { viewColumn.checked = true; return toast("keep at least one column", true); }
      viewConfig(resource).columns = tableColumns[resource].map(([key]) => key).filter((key) => columns.has(key));
      applyColumnVisibility(resource);
      return;
    }
    const issueFilter = event.target.closest("[data-issue-filter]");
    if (issueFilter) { state.issueFilters[issueFilter.dataset.issueFilter] = issueFilter.value; render(); return; }
    const logFilter = event.target.closest("[data-log-filter]");
    if (logFilter) { state.logFilters[logFilter.dataset.logFilter] = logFilter.value; renderFilteredActivity(); return; }
    const runtimeFilter = event.target.closest("[data-runtime-log-filter]");
    if (runtimeFilter) {
      state.runtimeLogFilters[runtimeFilter.dataset.runtimeLogFilter] = runtimeFilter.value;
      loadRuntimeLogs();
      return;
    }
    const trafficFilter = event.target.closest("[data-traffic-filter]");
    if (trafficFilter) { state.trafficAnalytics[trafficFilter.dataset.trafficFilter] = trafficFilter.value; state.trafficAnalytics.data = null; loadTrafficAnalytics(); return; }
    const up = event.target.closest("[data-user-profile]");
    if (up) { assignUserProfile(up.dataset.userProfile, up.value); return; }
    const uq = event.target.closest("[data-user-quota]");
    if (uq) { assignUserQuota(uq.dataset.userQuota, uq.value); return; }
    const field = event.target.closest("[data-wiz]");
    if (!field || !state.wiz) return;
    state.wiz[field.dataset.wiz] = field.value;
    if (field.hasAttribute("data-struct")) repaintWiz();
  });
  document.addEventListener("input", (event) => {
    const logFilter = event.target.closest("[data-log-filter]");
    if (logFilter) { state.logFilters[logFilter.dataset.logFilter] = logFilter.value; renderFilteredActivity(); return; }
    const runtimeFilter = event.target.closest("[data-runtime-log-filter]");
    if (runtimeFilter) {
      state.runtimeLogFilters[runtimeFilter.dataset.runtimeLogFilter] = runtimeFilter.value;
      scheduleRuntimeLogSearch();
      return;
    }
    const field = event.target.closest("[data-wiz]");
    if (!field || !state.wiz || field.hasAttribute("data-struct")) return;
    state.wiz[field.dataset.wiz] = field.value;
  });

  commandInput.addEventListener("input", () => { state.commandIndex = 0; renderCommandResults(); });
  commandInput.addEventListener("keydown", (event) => {
    if (event.key === "ArrowDown") { event.preventDefault(); state.commandIndex = Math.min(state.commandIndex + 1, Math.max(state.commandMatches.length - 1, 0)); renderCommandResults(); }
    if (event.key === "ArrowUp") { event.preventDefault(); state.commandIndex = Math.max(0, state.commandIndex - 1); renderCommandResults(); }
    if (event.key === "Enter") { event.preventDefault(); runCommand(state.commandIndex); }
  });
  commandResults.addEventListener("click", (event) => {
    const result = event.target.closest("[data-command-index]");
    if (result) runCommand(Number(result.dataset.commandIndex));
  });
  commandDialog.addEventListener("click", (event) => { if (event.target === commandDialog) commandDialog.close(); });
  formDialog.addEventListener("click", (event) => { if (event.target === formDialog) formDialog.close(); });
  entityForm.addEventListener("submit", (event) => {
    event.preventDefault();
    if (event.submitter?.value === "cancel") {
      formDialog.close();
      return;
    }
    if (entityForm.dataset.kind === "result") {
      formDialog.close();
      const route = state.resultReturnRoute;
      state.resultReturnRoute = "";
      if (route) go(route);
    }
    else submitEntity();
  });
  document.addEventListener("submit", (event) => {
    if (event.target.id !== "token-form") return;
    event.preventDefault();
    login();
  });
  document.addEventListener("click", (event) => {
    if (!event.target.closest("#recovery-toggle")) return;
    const form = $("#token-form");
    const totpRow = $("#totp-row");
    const recoveryRow = $("#recovery-row");
    const toggle = $("#recovery-toggle");
    const recoveryMode = !recoveryRow.hidden;
    totpRow.hidden = recoveryMode;
    recoveryRow.hidden = !recoveryMode;
    toggle.textContent = recoveryMode ? "Use an authenticator code instead" : "Use a recovery code instead";
    if (recoveryMode) requestAnimationFrame(() => form.elements.recovery_code.focus());
    else requestAnimationFrame(() => form.elements.totp_code.focus());
  });
  $("#token-button").addEventListener("click", () => state.admin ? go("settings") : openLogin());
  $("#theme-button").addEventListener("click", toggleTheme);
  $("#menu-toggle").addEventListener("click", () => document.body.classList.toggle("sidebar-open"));
  $("#collapse-button").addEventListener("click", () => document.body.classList.toggle("sidebar-collapsed"));
  document.addEventListener("keydown", (event) => {
    if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "k") {
      event.preventDefault();
      openCommand();
    }
  });
  window.addEventListener("hashchange", () => { const parsed = parseHash(); applyRoute(parsed.route, parsed.id, parsed.section, parsed.categorySection); });
  window.addEventListener("popstate", () => { const parsed = parseHash(); applyRoute(parsed.route, parsed.id, parsed.section, parsed.categorySection); });

  const savedTheme = readStore(localStorage, "honey-theme");
  if (savedTheme === "light" || savedTheme === "dark") document.documentElement.dataset.theme = savedTheme;
  document.documentElement.lang = lang;
  applyStaticI18n();
  buildMainNav();
  restoreGroups();
  ensureGroupOpen(state.route);
  checkHealth();
  loadData();
})();
