(() => {
  const root = document.documentElement;
  const toast = document.querySelector("#toast");
  const saved = localStorage.getItem("honey-sub-theme");
  if (saved === "light" || saved === "dark") root.dataset.theme = saved;

  document.querySelector("#theme")?.addEventListener("click", () => {
    root.dataset.theme = root.dataset.theme === "light" ? "dark" : "light";
    localStorage.setItem("honey-sub-theme", root.dataset.theme);
  });

  // --- i18n (public subscription page): EN default, RU translation. Keyed by the
  // canonical English string; t() falls back to the key.
  const I18N = {
    ru: {
      "subscription is live": "подписка активна", "theme": "тема",
      "your honey": "ваш honey",
      "Everything you need to connect. Keep this page private — its address is your access key.":
        "Всё для подключения. Держите эту страницу в секрете — её адрес и есть ваш ключ доступа.",
      "usage": "трафик", "Traffic": "Трафик", "used": "использовано",
      "one link": "одна ссылка", "Universal link": "Универсальная ссылка",
      "auto-detects supported apps": "определяет поддерживаемые приложения",
      "Paste this link into a supported client to receive a tailored config. If auto-detection fails, use a format below.":
        "Вставьте ссылку в поддерживаемый клиент. Если автоопределение не сработает, выберите формат ниже.",
      "copy": "копировать", "copied": "скопировано",
      "one tap": "в один тап", "Get config": "Получить конфиг",
      "updates every 12 hours": "обновляется каждые 12 часов",
      "download": "скачать", "auto-select group": "группа авто-выбора",
      "system-wide VPN preset": "системный VPN-пресет", "with auto-select group": "с группой авто-выбора",
      "Raw links": "Сырые ссылки", "one URI per endpoint": "по одному URI на эндпоинт",
      "one click": "в один клик", "Open in app": "Открыть в приложении",
      "installs this subscription directly": "устанавливает подписку напрямую",
      "all platforms": "все платформы",
      "available now": "доступно сейчас", "Endpoints": "Эндпоинты", "total": "всего",
      "check your line": "проверь канал", "Speed test": "Спидтест", "run": "запустить",
      "measures your connection to this panel (not to a node)": "измеряет соединение с этой панелью (не с нодой)",
      "measuring…": "измеряем…", "to this panel, not to a node": "до этой панели, не до ноды",
      "speed test failed": "спидтест не удался",
      "also available": "также доступно", "WireGuard": "WireGuard",
      "Extra services": "Доп. сервисы", "MTProto / NaiveProxy links": "ссылки MTProto / NaiveProxy",
      "import into the WireGuard or Amnezia app": "импорт в приложение WireGuard или Amnezia",
      "honey / private access": "honey / приватный доступ",
      "if it stops working, ask whoever gave it to you": "если перестало работать — спросите того, кто выдал доступ",
      "qr": "qr",
      // server-rendered labels
      "subscription id": "id подписки", "started": "начало", "expires": "истекает", "access": "доступ",
      "No accessible endpoints are available yet.": "Пока нет доступных эндпоинтов.",
    },
  };
  let lang = localStorage.getItem("honey-sub-lang");
  if (lang !== "ru" && lang !== "en") lang = "en";
  const t = (key) => (lang === "en" ? key : (I18N[lang] && I18N[lang][key]) || key);
  const applyI18n = () => {
    document.querySelectorAll("[data-i18n]").forEach((el) => { el.textContent = t(el.dataset.i18n); });
    root.lang = lang;
  };
  const langSelect = document.querySelector("#lang-select");
  if (langSelect) {
    langSelect.value = lang;
    langSelect.addEventListener("change", (e) => {
      lang = e.target.value === "ru" ? "ru" : "en";
      localStorage.setItem("honey-sub-lang", lang);
      applyI18n();
    });
  }
  applyI18n();

  // white-label branding (public): brand name, accent, welcome, section toggles.
  fetch("/branding", { headers: { accept: "application/json" } })
    .then((r) => (r.ok ? r.json() : null))
    .then((b) => {
      if (!b) return;
      if (b.accent_color) document.documentElement.style.setProperty("--accent", b.accent_color);
      if (b.brand_name) {
        const el = document.querySelector(".brand b");
        if (el) el.textContent = b.brand_name;
        document.title = `${b.brand_name} · subscription`;
      }
      if (b.logo_url) {
        const mark = document.querySelector(".brand .mark");
        if (mark) {
          mark.textContent = "";
          const img = document.createElement("img");
          img.src = b.logo_url;
          img.alt = "";
          img.style.cssText = "width:100%;height:100%;object-fit:contain;border-radius:inherit";
          mark.append(img);
        }
      }
      const hide = (sel) => { const el = document.querySelector(sel); if (el) el.hidden = true; };
      if (b.sub_show_imports === false) hide(".imports");
      if (b.sub_show_downloads === false) hide(".downloads");
      if (b.sub_show_endpoints === false) hide(".endpoints");
      if (b.sub_welcome) {
        const hero = document.querySelector(".hero p");
        if (hero) hero.textContent = b.sub_welcome;
      }
      if (b.footer_text || b.support_url) {
        const foot = document.querySelector("footer");
        if (foot) {
          foot.innerHTML = "";
          const left = document.createElement("span");
          left.textContent = b.footer_text || "";
          foot.append(left);
          if (b.support_url) {
            const a = document.createElement("a");
            a.href = b.support_url;
            a.target = "_blank";
            a.rel = "noopener";
            a.textContent = b.support_text || "support";
            foot.append(a);
          }
        }
      }
    })
    .catch(() => {});

  // operator announcement banner (public, may be null).
  fetch("/announcement", { headers: { accept: "application/json" } })
    .then((r) => (r.ok ? r.json() : null))
    .then((a) => {
      if (!a || !a.title) return;
      const main = document.querySelector("main");
      if (!main) return;
      const el = document.createElement("div");
      el.className = `announce ${a.level || "info"}`;
      const b = document.createElement("b");
      b.textContent = a.title;
      el.append(b);
      if (a.body) { const p = document.createElement("p"); p.textContent = a.body; el.append(p); }
      main.prepend(el);
    })
    .catch(() => {});

  // one-click import: build client deep-links from the rendered download urls.
  const absUrl = (id) => {
    const el = document.getElementById(id);
    if (!el) return "";
    try { return new URL(el.getAttribute("href"), location.href).href; } catch { return ""; }
  };
  const v2rayUrl = absUrl("dl-v2ray");
  const singboxUrl = absUrl("dl-singbox");
  const clashUrl = absUrl("dl-clash");
  const enc = encodeURIComponent;
  const name = document.title.split(" ")[0] || "honey";
  const deepLinks = {
    v2rayng: v2rayUrl && `v2rayng://install-sub?url=${enc(v2rayUrl)}&name=${enc(name)}`,
    hiddify: v2rayUrl && `hiddify://install-sub?url=${enc(v2rayUrl)}#${enc(name)}`,
    singbox: singboxUrl && `sing-box://import-remote-profile?url=${enc(singboxUrl)}#${enc(name)}`,
    karing: singboxUrl && `karing://install-config?url=${enc(singboxUrl)}&name=${enc(name)}`,
    streisand: v2rayUrl && `streisand://import/${v2rayUrl}`,
    clash: clashUrl && `clash://install-config?url=${enc(clashUrl)}`,
    v2raytun: v2rayUrl && `v2raytun://import/${v2rayUrl}`,
    // Happ's plain deep link keeps the nested subscription URL after
    // happ://add/; the URL itself must not be percent-encoded.
    happ: v2rayUrl && `happ://add/${v2rayUrl}`,
  };
  // universal link: the /sub/:token base (UA-tailored server-side). derive it
  // from the v2ray download url by stripping the format suffix.
  const uniUrl = v2rayUrl ? v2rayUrl.replace(/\/v2ray$/, "") : "";
  const uniEl = document.getElementById("uni-url");
  const uniCopy = document.getElementById("uni-copy");
  if (uniUrl && uniEl) {
    uniEl.textContent = uniUrl;
    if (uniCopy) uniCopy.dataset.copy = uniUrl;
  }

  // client-facing speed test: times a download from the panel. Honest scope —
  // this measures client <-> panel, not client <-> node.
  if (uniUrl) {
    const sec = document.getElementById("speedtest");
    const btn = document.getElementById("speedtest-run");
    const out = document.getElementById("speedtest-result");
    if (sec && btn && out) {
      sec.hidden = false;
      btn.addEventListener("click", async () => {
        btn.disabled = true;
        out.textContent = t("measuring…");
        const started = performance.now();
        try {
          const resp = await fetch(`${uniUrl}/speedtest?mb=10`, { cache: "no-store" });
          if (!resp.ok) throw new Error(String(resp.status));
          const blob = await resp.blob();
          const secs = (performance.now() - started) / 1000;
          const mbps = secs > 0 ? (blob.size * 8) / secs / 1e6 : 0;
          out.textContent = `${mbps.toFixed(1)} Mbps · ${t("to this panel, not to a node")}`;
        } catch {
          out.textContent = t("speed test failed");
        } finally {
          btn.disabled = false;
        }
      });
    }
  }

  // managed external services (MTProto / NaiveProxy) — copyable client links.
  if (uniUrl) {
    const escH = (s) => String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
    fetch(`${uniUrl}/services`, { headers: { accept: "application/json" } })
      .then((r) => (r.ok ? r.json() : []))
      .then((list) => {
        if (!Array.isArray(list) || !list.length) return;
        const sec = document.getElementById("services");
        const box = document.getElementById("svc-list");
        if (!sec || !box) return;
        box.innerHTML = list.map((s) => `<article class="endpoint"><div class="protocol">${escH(s.kind === "mtproto" ? "TG" : "NV")}</div><div class="endpoint-main"><b>${escH(s.name)}</b><small>${escH(s.kind)}</small></div><div class="endpoint-actions"><button data-copy="${escH(s.link)}" data-i18n="copy">${t("copy")}</button></div></article>`).join("");
        sec.hidden = false;
      })
      .catch(() => {});
  }

  // WireGuard / AmneziaWG configs (separate data-plane; may be empty).
  if (uniUrl) {
    const escHtml = (s) => String(s).replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
    fetch(`${uniUrl}/wireguard`, { headers: { accept: "application/json" } })
      .then((r) => (r.ok ? r.json() : []))
      .then((list) => {
        if (!Array.isArray(list) || !list.length) return;
        const sec = document.getElementById("wireguard");
        const grid = document.getElementById("wg-grid");
        if (!sec || !grid) return;
        grid.innerHTML = list.map((w) => {
          const cfg = `${uniUrl}/wireguard/${encodeURIComponent(w.id)}`;
          const label = w.amnezia ? "AmneziaWG" : "WireGuard";
          return `<a class="download" href="${cfg}" download="${escHtml(w.name)}.conf"><span><b>${escHtml(w.name)}</b><small>${label} · .conf</small></span><i>${t("download")}</i></a>`
            + `<a class="download" href="${cfg}/qr" target="_blank" rel="noopener"><span><b>${escHtml(w.name)} QR</b><small>${label}</small></span><i>QR</i></a>`;
        }).join("");
        sec.hidden = false;
      })
      .catch(() => {});
  }

  const importSection = document.getElementById("imports");
  if (importSection && (v2rayUrl || singboxUrl || clashUrl)) {
    importSection.hidden = false;
    importSection.querySelectorAll("[data-import]").forEach((btn) => {
      const link = deepLinks[btn.dataset.import];
      if (!link) { btn.hidden = true; return; }
      btn.addEventListener("click", () => { window.location.href = link; });
    });
  }

  document.addEventListener("click", async (event) => {
    const button = event.target.closest("[data-copy]");
    if (!button) return;
    try {
      await navigator.clipboard.writeText(button.dataset.copy);
      button.textContent = t("copied");
      toast.classList.add("show");
      setTimeout(() => {
        button.textContent = t("copy");
        toast.classList.remove("show");
      }, 1500);
    } catch {
      const input = document.createElement("textarea");
      input.value = button.dataset.copy;
      document.body.append(input);
      input.select();
      document.execCommand("copy");
      input.remove();
    }
  });
})();
