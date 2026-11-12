(function installVuikeyLibrary(config) {
  "use strict";
  const KEY = "__vuikeyLibraryV1";
  if (!config || config.version !== 1 || !Number.isSafeInteger(config.app_id) || config.app_id < 1 || config.app_id > 4294967295)
    return { installed: false, reason: "invalid_library_policy" };
  if (document.title === "Sign in to Steam") return { installed: false, reason: "authentication_window" };
  if (window[KEY]) return window[KEY].update(config);

  let policy = config, disposed = false, queued = false, navigated = false, readyPasses = 0;
  let storeApplied = false, overviewApplied = false, domApplied = false, restoreFailed = false;
  const hooks = [], views = new WeakMap(), collections = new Set();
  const apiScope = document.title === "SharedJSContext" || Boolean(window.appStore && window.collectionStore);
  const enabled = () => !disposed && policy.enabled === true;
  const appID = value => {
    const id = typeof value === "object" && value ? value.appid ?? value.app_id ?? value.appId : value;
    if (typeof id !== "number" && !(typeof id === "string" && /^\d{1,10}$/.test(id))) return null;
    const number = Number(id);
    return Number.isSafeInteger(number) && number > 0 && number <= 4294967295 ? number : null;
  };
  const allowed = value => appID(value) === policy.app_id;
  const onlyRental = list => enabled() && Array.isArray(list) ? list.filter(allowed) : list;
  function descriptor(owner, key) {
    for (let object = owner, depth = 0; object && depth < 8; object = Object.getPrototypeOf(object), depth++) {
      const result = Object.getOwnPropertyDescriptor(object, key); if (result) return result;
    }
    return null;
  }
  function patch(owner, key, factory, getter = false) {
    if (!owner) return false;
    try {
      const installed = hooks.find(hook => hook.owner === owner && hook.key === key);
      const own = Object.getOwnPropertyDescriptor(owner, key);
      if (installed && (own?.value === installed.wrapper || own?.get === installed.wrapper)) return true;
      const desc = descriptor(owner, key), original = getter ? desc?.get : owner[key];
      if (typeof original !== "function" || own && !own.configurable && (getter || !own.writable)) return false;
      const wrapper = factory(original);
      Object.defineProperty(owner, key, getter
        ? { configurable: true, enumerable: desc.enumerable, get: wrapper, set: desc.set }
        : own ? { ...own, value: wrapper } : { configurable: true, writable: true, value: wrapper });
      if (installed) { installed.original = own; installed.wrapper = wrapper; }
      else hooks.push({ owner, key, original: own, wrapper });
      return true;
    } catch { return false; }
  }
  function refreshCollections() {
    for (const item of collections) {
      try {
        item.ClearAppCounts?.();
        // Re-evaluate MobX presentation caches with the SAME membership/order.
        // SetApps is the inspected in-memory setter, NOT SaveCollection/SetAppsAsHidden.
        if (typeof item.SetApps === "function" && item.apps?.values)
          item.SetApps(Array.from(item.apps.values()));
      } catch { /* A removed/loading collection can be refreshed by Steam itself. */ }
    }
  }
  function restoreHooks() {
    restoreFailed = false;
    for (const hook of hooks.splice(0).reverse()) {
      try {
        const current = Object.getOwnPropertyDescriptor(hook.owner, hook.key);
        if (current?.value !== hook.wrapper && current?.get !== hook.wrapper) continue;
        if (hook.original) Object.defineProperty(hook.owner, hook.key, hook.original);
        else delete hook.owner[hook.key];
      } catch { hooks.push(hook); restoreFailed = true; }
    }
    refreshCollections(); collections.clear();
  }
  function collectionView(collection) {
    if (!collection || typeof collection !== "object") return collection;
    if (views.has(collection)) return views.get(collection);
    if (collections.size < 2048) collections.add(collection);
    const methods = new Map();
    const view = new Proxy(collection, { get(target, key, receiver) {
      const value = Reflect.get(target, key, receiver);
      if (!enabled()) return value;
      // Respect Proxy invariants for Steam builds with frozen data properties.
      const own = Object.getOwnPropertyDescriptor(target, key);
      if (own && !own.configurable && "value" in own && !own.writable) return value;
      if (key === "allApps" || key === "visibleApps") return onlyRental(value);
      if (key === "GetAppCountWithToolsFilter" && typeof value === "function") return (...args) =>
        Math.min(Reflect.apply(value, target, args), onlyRental(target.visibleApps)?.length ?? 0);
      // Keep raw membership and all mutations/serialization bound to the REAL
      // collection. A filtered view must never save a truncated account library.
      if (typeof value === "function") {
        const cached = methods.get(key);
        if (cached?.original === value) return cached.bound;
        const bound = value.bind(target); methods.set(key, { original: value, bound }); return bound;
      }
      return value;
    } });
    views.set(collection, view); views.set(view, view);
    return view;
  }
  function syncStores() {
    if (!enabled()) {
      if (hooks.length) restoreHooks();
      storeApplied = overviewApplied = false; return;
    }
    if (!apiScope) return;
    const store = window.collectionStore, apps = window.appStore;
    const before = hooks.length;
    overviewApplied = patch(apps, "allApps", original => function () {
      return onlyRental(Reflect.apply(original, this, []));
    }, true);
    const visibility = patch(store, "BIsVisible", original => function (item, ...args) {
      return (!enabled() || allowed(item)) && Reflect.apply(original, this, [item, ...args]);
    });
    const lookup = patch(store, "GetCollection", original => function (...args) {
      const result = Reflect.apply(original, this, args);
      return enabled() ? collectionView(result) : result;
    });
    patch(store, "GetCollectionForAppType", original => function (...args) {
      const result = Reflect.apply(original, this, args);
      return enabled() ? collectionView(result) : result;
    });
    // Some Steam builds expose configurable getters, others expose MobX accessors.
    // Getter hooks are optional; the visibility/method and DOM layers remain active.
    for (const key of ["allAppsCollection", "allRecentAppsCollection", "recentAppsCollection", "allGamesCollection", "myGamesCollection", "uncategorizedCollection", "localGamesCollection"])
      patch(store, key, original => function () { return enabled() ? collectionView(Reflect.apply(original, this, [])) : Reflect.apply(original, this, []); }, true);
    try {
      let count = 0;
      for (const item of store?.collectionsFromStorage?.values?.() || []) {
        if (++count > 2048) break; collectionView(item);
      }
      for (const id of ["all-apps-alpha", "all-apps-recent", "recent", "local-install"])
        collectionView(store?.GetCollection?.(id));
    } catch { /* Collections can still be loading after authentication. */ }
    storeApplied = visibility && lookup;
    readyPasses = storeApplied ? readyPasses + 1 : 0;
    if (hooks.length !== before) refreshCollections();
    // Open the rented game's native details once, not every refresh: Downloads,
    // Settings and Friends remain usable afterwards. This does not launch a game.
    if (!navigated && readyPasses >= 2 && apps?.GetAppOverviewByAppID?.(policy.app_id) && typeof window.SteamClient?.URL?.ExecuteSteamURL === "function") {
      navigated = true;
      try {
        const work = window.SteamClient.URL.ExecuteSteamURL("steam://nav/games/details/" + policy.app_id);
        work?.catch?.(() => { navigated = false; });
      } catch { navigated = false; }
    }
  }

  // Observed selectors: Steam sp.js package 11123049, modules 39287/69649/66830.
  // Unknown library cards fail closed until their AppID is resolved. No title matching.
  const selectors = [
    "._2-O4ZG0KrnSrzISHBKctFQ", "._1vO6BoiVslZgs1kqDGdUs8", ".WYgDg9NyCcMIVuMyZ_NBC",
    "._3fp5y6tdU77MdrCQZt_aS9", "._21P7c4MWmsU2QSZeL-uyZo", "._2sCvZTD6ZTZz3p9_ekH_gI",
    ".B-vCdL38RiJlhfSRgZv78", "._3Rb_j3lgHaVVIud4Gla7_m",
    '[class*="gamelistentry_Container"]', '[class*="gamelistentry_GameListEntryContainer"]',
    '[class*="appportrait_LibraryItemBox"]', '[class*="appdetails_AppDetailsRoot"]',
    '[class*="downloadspage_SectionItemWrapper"]',
  ];
  const selector = selectors.join(","), marker = "data-vuikey-library-app", rootMarker = "data-vuikey-library-active";
  const root = document.documentElement, originalRoot = root.getAttribute(rootMarker);
  const style = document.createElement("style"); style.id = "vuikey-steam-library-v1";
  style.textContent = selectors.map(value => `html[${rootMarker}="${policy.app_id}"] ${value}:not([${marker}="${policy.app_id}"])`).join(",") + "{display:none!important;pointer-events:none!important;}";
  (document.head || root).appendChild(style);
  function propsID(value, depth = 0) {
    if (!value || typeof value !== "object" || depth > 3) return null;
    const direct = appID(value); if (direct) return direct;
    for (const key of ["app", "item", "overview", "appOverview", "game", "props"]) {
      const id = propsID(value[key], depth + 1); if (id) return id;
    }
    return null;
  }
  function nodeID(node) {
    try {
      const attr = appID(node.getAttribute("data-appid") ?? node.getAttribute("data-app-id")); if (attr) return attr;
      for (let element = node, level = 0; element && level < 4; element = element.parentElement, level++) {
        for (const key of Object.getOwnPropertyNames(element)) {
          if (key.startsWith("__reactProps$")) { const id = propsID(element[key]); if (id) return id; }
          if (key.startsWith("__reactFiber$")) {
            for (let fiber = element[key], steps = 0; fiber && steps < 6; fiber = fiber.return, steps++) {
              const id = propsID(fiber.memoizedProps) || propsID(fiber.pendingProps); if (id) return id;
            }
          }
        }
      }
      const ids = new Set(); let count = 0;
      for (const image of node.querySelectorAll("img[src]")) {
        if (++count > 32) break;
        const match = (image.getAttribute("src") || "").match(/\/(?:apps|clienticon|librarycache)\/(\d+)(?:\/|_)/);
        if (match) ids.add(Number(match[1]));
      }
      return ids.size === 1 ? [...ids][0] : null;
    } catch { return null; }
  }
  function clearDom() {
    if (originalRoot === null) root.removeAttribute(rootMarker); else root.setAttribute(rootMarker, originalRoot);
    for (const item of document.querySelectorAll("[" + marker + "]")) item.removeAttribute(marker);
  }
  function filterDom() {
    if (!enabled()) { clearDom(); domApplied = false; return; }
    if (root.getAttribute(rootMarker) !== String(policy.app_id)) root.setAttribute(rootMarker, String(policy.app_id));
    let count = 0;
    for (const item of document.querySelectorAll(selector)) {
      if (++count > 4096) break;
      const id = nodeID(item), value = id === null ? "unknown" : String(id);
      if (item.getAttribute(marker) !== value) item.setAttribute(marker, value);
    }
    domApplied = true;
  }
  const observer = new MutationObserver(() => {
    if (disposed || queued) return;
    queued = true; Promise.resolve().then(() => { queued = false; if (!disposed) filterDom(); });
  });
  observer.observe(root, { subtree: true, childList: true, characterData: true, attributes: true,
    attributeFilter: ["class", "src", "href", "data-appid", "data-app-id", "title", "alt", "aria-label"] });
  function blockOtherGame(event) {
    if (!enabled()) return;
    if (event.type === "keydown" && !["Enter", " ", "ContextMenu"].includes(event.key)) return;
    const item = event.target?.closest?.(selector);
    const link = event.target?.closest?.("a[href]")?.getAttribute("href") || "";
    const route = link.match(/^steam:\/\/(?:nav\/games\/details|rungameid|run|install)\/(\d+)(?:[/?#]|$)/i);
    if (item && nodeID(item) !== policy.app_id || route && Number(route[1]) !== policy.app_id) {
      event.preventDefault(); event.stopImmediatePropagation();
    }
  }
  const events = ["click", "dblclick", "contextmenu", "keydown"];
  for (const type of events) document.addEventListener(type, blockOtherGame, true);
  function snapshot() {
    const missing = [];
    if (enabled() && apiScope && !storeApplied) missing.push("library_collection_api");
    if (restoreFailed) missing.push("library_restore_failed");
    return { installed: !disposed, enabled: enabled(), phase: !enabled() ? "disabled" : missing.length ? "partial" : "filtered",
      store_applied: storeApplied, overview_applied: overviewApplied, dom_applied: domApplied, unsupported: missing };
  }
  function update(next) {
    if (disposed) return { installed: false, reason: "disposed" };
    if (next.version !== 1 || next.app_id !== policy.app_id) return { installed: false, reason: "library_policy_mismatch" };
    policy = next; syncStores(); filterDom(); return snapshot();
  }
  const api = { update, snapshot, dispose() {
    if (disposed) return { disposed: true, restored: !restoreFailed };
    disposed = true; observer.disconnect();
    for (const type of events) document.removeEventListener(type, blockOtherGame, true);
    restoreHooks(); clearDom(); style.remove();
    if (!restoreFailed) delete window[KEY];
    return { disposed: true, restored: !restoreFailed };
  } };
  Object.defineProperty(window, KEY, { value: api, configurable: true });
  return update(config);
})