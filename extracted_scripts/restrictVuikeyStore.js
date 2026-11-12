(function restrictVuikeyStore(config) {
  "use strict";
  const KEY = "__vuikeyStoreV1", REVISION = 1;
  const BLOCKED = "https://vuikey.invalid/store-blocked";
  if (!config || config.version !== 1 || !Number.isSafeInteger(config.app_id) || config.app_id <= 0 || typeof config.enabled !== "boolean")
    return { installed: false, reason: "invalid_config" };
  if (document.title === "Sign in to Steam") return { installed: false, reason: "authentication_window" };
  if (window[KEY]?.revision === REVISION) return window[KEY].update(config);
  window[KEY]?.dispose();
  let policy = config, disposed = false, queued = false, blocked = 0, managerGuard = false;
  let wrappers = [], seen = new WeakMap();
  const links = new Map();
  const enabled = () => !disposed && policy.enabled;
  function storeURL(value, depth = 0) {
    if (typeof value !== "string" || value.length > 16384 || depth > 4) return false;
    let url;
    try { url = new URL(value, "https://steamloopback.host/"); } catch { return false; }
    const host = url.hostname.toLowerCase().replace(/\.$/, "");
    if (["https:", "http:"].includes(url.protocol))
      return /^(?:store|checkout)\.steampowered\.com$/.test(host) || host === "store.steamchina.com";
    if (url.protocol !== "steam:") return false;
    if (["store", "purchase", "advertise", "checkout", "cart", "wishlist"].includes(host)) return true;
    let path;
    try { path = decodeURIComponent(url.pathname).replace(/^\/+/, ""); } catch { return false; }
    if (host === "url") return /^(?:store[^/]*|steampreferences|steamstore[^/]*|steamcheckout[^/]*|addtocart)(?:\/|$)/i.test(path);
    if (host === "open") return /^(?:store|cart|checkout|wishlist)(?:\/|$)/i.test(path);
    if (["openurl", "openurl_external", "openurl_externalforpid", "openexternalforpid"].includes(host)) {
      if (host.endsWith("forpid")) path = path.replace(/^\d+\//, "");
      return storeURL(path, depth + 1);
    }
    return false;
  }
  function safeURL(value) {
    if (enabled() && storeURL(value)) { blocked++; return BLOCKED; }
    return value;
  }
  function firstURL(args) {
    const value = safeURL(args[0]);
    return value === args[0] ? args : [value, ...args.slice(1)];
  }
  function locationArgs(args) {
    const value = args[0];
    if (!value || typeof value !== "object" || !value.state) return firstURL(args);
    const state = { ...value.state };
    for (const name of ["strURL", "url"]) if (typeof state[name] === "string") state[name] = safeURL(state[name]);
    return [{ ...value, state }, ...args.slice(1)];
  }
  function wrap(owner, name, transform = firstURL, after) {
    if (!owner || wrappers.length >= 512) return false;
    let original, descriptor;
    try { original = owner[name]; descriptor = Object.getOwnPropertyDescriptor(owner, name); } catch { return false; }
    if (typeof original !== "function") return false;
    const prior = seen.get(owner)?.get(name);
    if (prior?.wrapper === original) return true;
    if (descriptor && (!Object.hasOwn(descriptor, "value") || !descriptor.configurable && !descriptor.writable)) return false;
    const wrapper = function (...args) {
      const next = enabled() ? transform(args) : args;
      if (next === null) return;
      const result = original.apply(this, next);
      if (enabled() && after) after(result);
      return result;
    };
    try {
      Object.defineProperty(owner, name, { ...(descriptor || { configurable: true, writable: true, enumerable: false }), value: wrapper });
      const entry = { owner, name, descriptor, wrapper };
      wrappers.push(entry);
      if (!seen.has(owner)) seen.set(owner, new Map());
      seen.get(owner).set(name, entry);
      return true;
    } catch { return false; }
  }
  function patchBrowser(browser, manager) {
    if (!browser) return false;
    const load = wrap(browser, "LoadURL"), replace = wrap(browser, "ReplaceURL");
    if (manager) {
      for (const [method, step] of [["GoBack", -1], ["GoForward", 1]]) {
        wrap(browser, method, args => {
          const history = manager.m_browserHistory, target = history?.entries?.[history.index + step]?.url;
          if ((target === BLOCKED || storeURL(target)) && typeof browser.ReplaceURL === "function") {
            blocked++; browser.ReplaceURL(BLOCKED); return null;
          }
          return args;
        });
      }
      wrap(browser, "Reload", args => {
        if (storeURL(manager.DisplayURL) && typeof browser.ReplaceURL === "function") {
          blocked++; browser.ReplaceURL(BLOCKED); return null;
        }
        return args;
      });
    }
    return load || replace;
  }
  function options(args) {
    const value = args[0];
    if (!value || typeof value !== "object" || typeof value.strInitialURL !== "string") return args;
    const url = safeURL(value.strInitialURL);
    return url === value.strInitialURL ? args : [{ ...value, strInitialURL: url }, ...args.slice(1)];
  }
  function manager() {
    try { return window.MainWindowBrowserManager || window.opener?.MainWindowBrowserManager; } catch { return null; }
  }
  function syncNavigation() {
    managerGuard = false;
    if (!enabled()) return;
    // These entrypoints/fields are present in installed Steam steamui/sp.js:
    // MainWindowBrowserManager.LoadURL/ShowURL, its browser LoadURL/ReplaceURL,
    // tab requests, and SteamClient.URL/System/BrowserView. No guessed API call.
    // Hook a shared manager only in its own context. Popups can route to their
    // opener, but must not stack cross-window wrappers on that same instance.
    const main = window.MainWindowBrowserManager;
    if (main) {
      managerGuard = wrap(main, "LoadURL");
      wrap(main, "ShowURL");
      managerGuard = patchBrowser(main.m_browser, main) || managerGuard;
      wrap(main.TabbedBrowserStore, "AddWebPageRequest");
      wrap(main.m_history, "push", locationArgs); wrap(main.m_history, "replace", locationArgs);
      wrap(main, "SyncWithNewRouterEvent", locationArgs);
      wrap(main, "SyncWithNewBrowserHistory", args => {
        const history = args[0];
        if (!history || !Array.isArray(history.entries)) return args;
        if (storeURL(history.entries[history.index]?.url)) main.m_browser?.ReplaceURL?.(BLOCKED);
        return [{ ...history, entries: history.entries.map(entry => ({ ...entry, url: safeURL(entry.url) })) }, ...args.slice(1)];
      });
      if (storeURL(main.DisplayURL)) {
        blocked++;
        if (typeof main.m_browser?.ReplaceURL === "function") main.m_browser.ReplaceURL(BLOCKED);
        else main.LoadURL?.(BLOCKED);
      }
    }
    const client = window.SteamClient;
    wrap(client?.URL, "ExecuteSteamURL", args => storeURL(args[0])
      ? (blocked++, ["steam://openurl/" + BLOCKED, ...args.slice(1)]) : args);
    wrap(client?.System, "OpenInSystemBrowser");
    wrap(client?.BrowserView, "Create", options, browser => patchBrowser(browser));
    wrap(client?.BrowserView, "CreatePopup", options, result => patchBrowser(result?.browserView));
    wrap(window, "open");
  }
  function restoreLinks() {
    for (const [link, href] of links) if (link.getAttribute("href") === BLOCKED) link.setAttribute("href", href);
    links.clear();
  }
  function scanLinks() {
    if (!enabled()) return;
    for (const link of document.querySelectorAll("a[href],area[href]")) {
      const href = link.getAttribute("href");
      if (storeURL(href)) {
        if (links.size < 4096 || links.has(link)) { links.set(link, href); link.setAttribute("href", BLOCKED); }
      }
    }
  }
  function intercept(event) {
    if (!enabled() || event.type === "keydown" && !["Enter", " "].includes(event.key)) return;
    const link = event.target?.closest?.("a[href],area[href]");
    if (!link || !storeURL(link.getAttribute("href")) && !(links.has(link) && link.getAttribute("href") === BLOCKED)) return;
    event.preventDefault(); event.stopImmediatePropagation(); blocked++;
    manager()?.ShowURL?.(BLOCKED);
  }
  for (const name of ["click", "auxclick", "keydown"]) document.addEventListener(name, intercept, true);
  const observer = new MutationObserver(() => {
    if (!enabled() || queued) return;
    queued = true;
    Promise.resolve().then(() => { queued = false; if (!disposed) { syncNavigation(); scanLinks(); } });
  });
  observer.observe(document.documentElement, { childList: true, subtree: true, attributes: true, attributeFilter: ["href"] });
  function restoreMethods() {
    for (const { owner, name, descriptor, wrapper } of wrappers.reverse()) {
      try {
        if (owner[name] !== wrapper) continue;
        if (descriptor) Object.defineProperty(owner, name, descriptor); else delete owner[name];
      } catch {}
    }
    wrappers = []; seen = new WeakMap();
  }
  function snapshot() {
    return { installed: !disposed, enabled: enabled(), navigation_applied: !enabled() || managerGuard,
      hooks: wrappers.length, blocked_count: blocked, revision: REVISION };
  }
  function update(next) {
    if (disposed || next.version !== 1 || next.app_id !== policy.app_id || typeof next.enabled !== "boolean")
      return { installed: false, reason: "policy_mismatch" };
    policy = next;
    if (!enabled()) { restoreLinks(); restoreMethods(); }
    else { syncNavigation(); scanLinks(); }
    return snapshot();
  }
  const api = { revision: REVISION, update, snapshot, dispose() {
    disposed = true; observer.disconnect();
    for (const name of ["click", "auxclick", "keydown"]) document.removeEventListener(name, intercept, true);
    restoreLinks(); restoreMethods();
    if (window[KEY] === api) delete window[KEY];
    return { disposed: true };
  } };
  Object.defineProperty(window, KEY, { value: api, configurable: true });
  return update(config);
})