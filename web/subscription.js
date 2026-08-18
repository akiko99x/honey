(() => {
  const root = document.documentElement;
  const toast = document.querySelector("#toast");
  const savedTheme = localStorage.getItem("neko-sub-theme");
  if (savedTheme === "light" || savedTheme === "dark") root.dataset.theme = savedTheme;

  const I18N = {
    ru: {
      protected: "защищено", "private access": "личный доступ",
      "Ready on all your devices": "Готово для всех ваших устройств",
      Traffic: "Трафик", used: "использовано", "choose device": "выберите устройство",
      "Connect Neko VPN": "Подключить Neko VPN", "choose app": "выберите приложение",
      recommended: "рекомендуем", "copy link": "копировать ссылку", "all platforms": "все платформы",
      "your subscription": "ваша подписка", "One link for every device": "Одна ссылка для всех устройств",
      "Keep it private. Anyone with this link can use your VPN.": "Не публикуйте её: тот, у кого есть ссылка, сможет пользоваться вашим VPN.",
      copy: "копировать", copied: "скопировано", "Imports use the reserve delivery server": "Импорт идёт через резервный сервер выдачи",
      advanced: "дополнительно", "Manual setup and formats": "Ручная настройка и форматы",
      "Universal link": "Универсальная ссылка", download: "скачать", "auto-select group": "автовыбор сервера",
      "system-wide VPN preset": "системный VPN", "Raw links": "Сырые ссылки", "one URI per endpoint": "по одной на сервер",
      "extra services": "доп. сервисы", "Extra services": "Дополнительные сервисы", "Alternative connection": "Альтернативное подключение",
      "available now": "доступно сейчас", Servers: "Серверы", "connection check": "проверка соединения",
      "Measures the route to the subscription server": "Измеряет маршрут до сервера подписки", "run test": "запустить",
      "Need help? Contact support.": "Нужна помощь? Напишите в поддержку.",
      "subscription id": "id подписки", started: "начало", expires: "истекает", access: "доступ",
      "No accessible endpoints are available yet.": "Пока нет доступных серверов.", qr: "qr",
      "measuring…": "измеряем…", "speed test failed": "не удалось измерить",
    },
  };
  let lang = localStorage.getItem("neko-sub-lang");
  if (lang !== "ru" && lang !== "en") lang = navigator.language?.toLowerCase().startsWith("ru") ? "ru" : "en";
  const t = (key) => (lang === "en" ? key : I18N.ru[key] || key);
  const updateDynamicCopy = () => {
    const hours = Math.max(1, Number(root.dataset.updateHours) || 1);
    const el = document.querySelector("#refresh-copy");
    if (!el) return;
    if (lang === "ru") {
      const form = hours % 10 === 1 && hours % 100 !== 11 ? "час" : hours % 10 >= 2 && hours % 10 <= 4 && !(hours % 100 >= 12 && hours % 100 <= 14) ? "часа" : "часов";
      el.textContent = `обновление каждые ${hours} ${form}`;
    } else el.textContent = `updates every ${hours} ${hours === 1 ? "hour" : "hours"}`;
  };
  const applyI18n = () => {
    document.querySelectorAll("[data-i18n]").forEach((el) => { el.textContent = t(el.dataset.i18n); });
    root.lang = lang;
    updateDynamicCopy();
  };
  const langSelect = document.querySelector("#lang-select");
  if (langSelect) {
    langSelect.value = lang;
    langSelect.addEventListener("change", (event) => { lang = event.target.value === "en" ? "en" : "ru"; localStorage.setItem("neko-sub-lang", lang); applyI18n(); });
  }
  applyI18n();

  document.querySelector("#theme")?.addEventListener("click", () => {
    root.dataset.theme = root.dataset.theme === "light" ? "dark" : "light";
    localStorage.setItem("neko-sub-theme", root.dataset.theme);
  });

  const showToast = (message = t("copied")) => {
    const span = toast?.querySelector("span");
    if (span) span.textContent = message;
    toast?.classList.add("show");
    window.setTimeout(() => toast?.classList.remove("show"), 1500);
  };
  const copyText = async (value) => {
    try { await navigator.clipboard.writeText(value); }
    catch {
      const input = document.createElement("textarea"); input.value = value; document.body.append(input); input.select(); document.execCommand("copy"); input.remove();
    }
    showToast();
  };

  const localAbs = (id) => {
    const href = document.getElementById(id)?.getAttribute("href");
    try { return href ? new URL(href, location.href).href : ""; } catch { return ""; }
  };
  const deliveryBase = root.dataset.subscriptionBase?.trim().replace(/\/$/, "") || "";
  const throughDelivery = (value) => {
    if (!value || !deliveryBase) return value;
    try { const url = new URL(value); return `${deliveryBase}${url.pathname}${url.search}${url.hash}`; } catch { return value; }
  };
  const localV2rayUrl = localAbs("dl-v2ray");
  const localUniUrl = localV2rayUrl ? localV2rayUrl.replace(/\/v2ray$/, "") : "";
  const v2rayUrl = throughDelivery(localV2rayUrl);
  const singboxUrl = throughDelivery(localAbs("dl-singbox"));
  const clashUrl = throughDelivery(localAbs("dl-clash"));
  const uniUrl = throughDelivery(localUniUrl);
  document.querySelectorAll("[data-delivery]").forEach((link) => { link.href = throughDelivery(link.href); });
  const profileUrl = (client) => uniUrl ? `${uniUrl}/profile/${client}` : "";
  const enc = encodeURIComponent;
  const profileName = document.title.split(" · ")[0] || "Neko VPN";

  const uniEl = document.querySelector("#uni-url");
  const uniCopy = document.querySelector("#uni-copy");
  if (uniUrl && uniEl) { uniEl.textContent = uniUrl; if (uniCopy) uniCopy.dataset.copy = uniUrl; }
  if (deliveryBase) document.querySelector("#delivery-note")?.removeAttribute("hidden");

  const guessedPlatform = () => {
    const ua = navigator.userAgent.toLowerCase();
    if (/iphone|ipad|ipod/.test(ua)) return "ios";
    if (ua.includes("android")) return /tv|aft/.test(ua) ? "tv" : "android";
    if (ua.includes("win")) return "windows";
    if (ua.includes("mac")) return "macos";
    if (ua.includes("linux")) return "linux";
    return "android";
  };
  let platform = guessedPlatform();
  const deepLinks = () => {
    const generic = profileUrl("generic") || v2rayUrl;
    const happUrl = profileUrl(platform === "android" || platform === "tv" ? "happ-android" : "happ-desktop") || v2rayUrl;
    return {
      v2rayng: generic && `v2rayng://install-sub?url=${enc(generic)}&name=${enc(profileName)}`,
      hiddify: generic && `hiddify://install-sub?url=${enc(generic)}#${enc(profileName)}`,
      singbox: singboxUrl && `sing-box://import-remote-profile?url=${enc(singboxUrl)}#${enc(profileName)}`,
      karing: profileUrl("karing") && `karing://install-config?url=${enc(profileUrl("karing"))}&name=${enc(profileName)}`,
      streisand: generic && `streisand://import/${generic}`,
      clash: clashUrl && `clash://install-config?url=${enc(clashUrl)}`,
      v2raytun: generic && `v2raytun://import/${generic}`,
      happ: happUrl && `happ://add/${enc(happUrl)}`,
    };
  };
  const refreshApps = () => {
    document.querySelectorAll("[data-platform]").forEach((button) => { const active = button.dataset.platform === platform; button.classList.toggle("active", active); button.setAttribute("aria-selected", String(active)); });
    document.querySelectorAll("[data-platforms]").forEach((card) => { card.hidden = !card.dataset.platforms.split(/\s+/).includes(platform); });
    const links = deepLinks();
    document.querySelectorAll("[data-import]").forEach((card) => {
      const link = links[card.dataset.import];
      if (card.tagName === "A") card.setAttribute("href", link || "#");
    });
  };
  document.querySelectorAll("[data-platform]").forEach((button) => button.addEventListener("click", () => { platform = button.dataset.platform; refreshApps(); }));
  document.querySelectorAll("button[data-import]").forEach((button) => button.addEventListener("click", () => { const link = deepLinks()[button.dataset.import]; if (link) location.href = link; }));
  document.querySelectorAll("[data-copy-app]").forEach((button) => button.addEventListener("click", () => copyText(profileUrl(button.dataset.copyApp) || v2rayUrl)));
  refreshApps();

  document.addEventListener("click", (event) => {
    const button = event.target.closest("[data-copy]");
    if (button?.dataset.copy) copyText(button.dataset.copy);
  });

  fetch("/branding", { headers: { accept: "application/json" } }).then((r) => r.ok ? r.json() : null).then((b) => {
    if (!b) return;
    if (b.accent_color) root.style.setProperty("--accent", b.accent_color);
    if (b.brand_name) { const el = document.querySelector(".wordmark b"); if (el) el.textContent = b.brand_name.replace(/\s*vpn$/i, ""); document.title = `${b.brand_name} · subscription`; }
    if (b.logo_url) { const mark = document.querySelector(".mark"); if (mark) { const image = document.createElement("img"); image.src = b.logo_url; image.alt = ""; mark.replaceChildren(image); } }
    if (b.sub_welcome) { const hero = document.querySelector(".account-copy p"); if (hero) hero.textContent = b.sub_welcome; }
    if (b.footer_text || b.support_url) { const foot = document.querySelector("footer"); if (foot) { const label = document.createElement("span"); label.textContent = b.footer_text || "Neko VPN"; foot.replaceChildren(label); if (b.support_url) { const a = document.createElement("a"); a.href = b.support_url; a.target = "_blank"; a.rel = "noopener"; a.textContent = b.support_text || "support"; foot.append(a); } } }
  }).catch(() => {});

  fetch("/announcement", { headers: { accept: "application/json" } }).then((r) => r.ok ? r.json() : null).then((a) => {
    if (!a?.title) return; const el = document.createElement("div"); el.className = `announce ${a.level || "info"}`; const title = document.createElement("b"); title.textContent = a.title; el.append(title); if (a.body) { const p = document.createElement("p"); p.textContent = a.body; el.append(p); } document.querySelector("main")?.prepend(el);
  }).catch(() => {});

  if (localUniUrl) {
    const speed = document.querySelector("#speedtest"), run = document.querySelector("#speedtest-run"), result = document.querySelector("#speedtest-result");
    if (speed && run && result) { speed.hidden = false; run.addEventListener("click", async () => { run.disabled = true; result.textContent = t("measuring…"); const started = performance.now(); try { const response = await fetch(`${localUniUrl}/speedtest?mb=10`, { cache: "no-store" }); if (!response.ok) throw new Error(); const blob = await response.blob(); const mbps = (blob.size * 8) / ((performance.now() - started) / 1000) / 1e6; result.textContent = `${mbps.toFixed(1)} Mbps`; } catch { result.textContent = t("speed test failed"); } finally { run.disabled = false; } }); }
    fetch(`${localUniUrl}/services`, { headers: { accept: "application/json" } }).then((r) => r.ok ? r.json() : []).then((list) => { if (!Array.isArray(list) || !list.length) return; const section = document.querySelector("#services"), box = document.querySelector("#svc-list"); if (!section || !box) return; list.forEach((service) => { const row = document.createElement("article"); row.className = "endpoint"; row.innerHTML = `<div class="protocol">${service.kind === "mtproto" ? "TG" : "NV"}</div><div class="endpoint-main"><b></b><small></small></div><div class="endpoint-actions"><button type="button" data-copy="">${t("copy")}</button></div>`; row.querySelector("b").textContent = service.name; row.querySelector("small").textContent = service.kind; row.querySelector("button").dataset.copy = service.link; box.append(row); }); section.hidden = false; }).catch(() => {});
    fetch(`${localUniUrl}/wireguard`, { headers: { accept: "application/json" } }).then((r) => r.ok ? r.json() : []).then((list) => { if (!Array.isArray(list) || !list.length) return; const section = document.querySelector("#wireguard"), grid = document.querySelector("#wg-grid"); if (!section || !grid) return; list.forEach((item) => { const link = document.createElement("a"); link.className = "download"; link.href = `${localUniUrl}/wireguard/${encodeURIComponent(item.id)}`; link.download = `${item.name}.conf`; link.innerHTML = `<span><b></b><small>${item.amnezia ? "AmneziaWG" : "WireGuard"} · .conf</small></span><i>${t("download")}</i>`; link.querySelector("b").textContent = item.name; grid.append(link); }); section.hidden = false; }).catch(() => {});
  }
})();
