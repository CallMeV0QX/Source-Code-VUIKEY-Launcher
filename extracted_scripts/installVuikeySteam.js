(function installVuikeySteam(config) {
  "use strict";
  const KEY = "__vuikeySteamV1", REVISION = 5;
  if (!config || config.version !== 1 || !Number.isSafeInteger(config.app_id) || config.app_id <= 0)
    return { installed: false, reason: "invalid_policy" };
  if (document.title === "Sign in to Steam") return { installed: false, reason: "authentication_window" };
  if (window[KEY]) {
    if (window[KEY].revision === REVISION) return window[KEY].update(config);
    window[KEY].dispose();
    return { installed: false, reason: "replacing_previous_payload" };
  }

  // Current Steam renders visible windows in about:blank popups WITHOUT SteamClient.
  // Only SharedJSContext owns the native API. Never gate DOM/CSS on that API.
  const apiScope = document.title === "SharedJSContext" || !!window.SteamClient?.User;
  const scope = apiScope ? document.title === "SharedJSContext" ? "api" : "both" : "ui";
  let policy = config, disposed = false, client = null, actionHook = null, actionAttempted = false;
  let remote = null, remoteBefore, remoteDisabled = false, remoteWork = null;
  let remoteUnsupported = apiScope, actionUnsupported = apiScope;
  let restoreFailed = false, disposeWork = null, queued = false, selectedLibrary = false;
  const style = document.createElement("style");
  style.id = "vuikey-steam-policy-v1";
  (document.head || document.documentElement).appendChild(style);
  // The hash selectors below are observed in installed sp.js package 11123049.
  // Keep stable semantic/legacy selectors as well; no arbitrary remote CSS is used.
  const accountCss = `
    [id$='/settings/account'],[id$='/settings/family'],
    [id$='/settings/security'],[id$='/settings/remoteplay'],
    [id$='/settings/broadcast'],[id$='/settings/developer'],
    [id$='/settings/account_Content'],
    ._3yD46y5pd3zOGR7CzKs0mC,
    [class*='titlebarcontrols_AccountMenu'],
    ._3AjoLnMNKxYmNTGTJCLfgs,[class*='appportrait_LibraryItemActionButton'],
    ._2dnxv1f3olKbOSmfSFezvJ,[class*='appdetailssectionoffline_OfflineSectionBody'] {
      display:none!important;pointer-events:none!important;
    }`;
  const presenceCss = `._1-9sir4j_KQiMqdkZjQN0u .tool-tip-source:has(.online,.offline){pointer-events:none!important;}`;
  const friendsCss = `[id$='/settings/friends'],[class*='bottombar_FriendsButton'],[class*='friendslist_FriendsListButton']{display:none!important;pointer-events:none!important;}`;
  const hiddenCss = `[data-vuikey-hidden="1"]{display:none!important;pointer-events:none!important;}`;
  const menuSelector = 'div[class*=contextMenuItem],div._1n7Wloe5jZ6fSuvV18NNWI,[role="menuitem"]';
  const navSelector = '._7AlhCx3XGzBeIrQaCneUD,[class*="supernav_SuperNavMenu"]';
  const rootButtonSelector = '._1Ky59qmywxOUtNcI1cgmkX>._2UyOBeiSdBayaFdRa39N2O,[class*="rootmenubar_RootMenuBar"]>[class*="rootmenubar_RootMenuButton"]';
  const norm = s => (s || "").trim().replace(/\s+/g, " ").replace(/[.…]+$/g, "").toLowerCase();
  const communityAction = /^(community|cộng đồng)$/;
  const socialAction = /^(community|cộng đồng|profile|hồ sơ|view my profile|xem hồ sơ của tôi|friends|bạn bè|friends list|view friends list|danh sách bạn bè|add a friend|thêm bạn bè|chat|trò chuyện|friends\s*(?:&|and)\s*chat|players)$/;
  const accountAction = /^(account details|store preferences|change account|sign out|log out|go offline|go online)$/;
  function mark(item, hide) {
    if (hide) {
      if (item.getAttribute("data-vuikey-hidden") !== "1") item.setAttribute("data-vuikey-hidden", "1");
    } else if (item.getAttribute("data-vuikey-hidden") === "1") item.removeAttribute("data-vuikey-hidden");
  }
  function filterChrome() {
    if (disposed) return;
    const title = document.title;
    const rootMenu = title === "Steam Root Menu";
    const accountMenu = title === "Account Menu";
    const otherRoot = /^(View|Friends|Games|Help) Root Menu$/.test(title);
    const otherNav = /^(Store|Community|Profile) Supernav$/.test(title);
    // This boolean comes from the server's allocation-mode snapshot. It is not
    // a client-visible switch. Community stays available in both rental modes;
    // persona Profile, Friends and presence controls remain online-only.
    const social = policy.allow_friends === true;
    const communityNav = title === "Community Supernav";
    const profileNav = title === "Profile Supernav";
    for (const item of document.querySelectorAll(menuSelector)) {
      const text = norm(item.textContent);
      let hide = false;
      if (policy.hide_account_settings && text) {
        if (rootMenu) hide = !/^(settings|exit)$/.test(text) && !communityAction.test(text) && !(social && socialAction.test(text));
        else if (accountMenu) hide = text !== "exit" && !communityAction.test(text) && !(social && socialAction.test(text));
        else if (communityNav || (social && profileNav)) hide = accountAction.test(text) ||
          (!social && socialAction.test(text) && !communityAction.test(text));
        else if (otherNav) hide = text !== "exit";
        else if (title === "Friends Root Menu" && social) hide = accountAction.test(text);
        else if (otherRoot) hide = !/^(library|downloads|settings|exit)$/.test(text) && !communityAction.test(text) &&
          !(social && title === "View Root Menu" && socialAction.test(text));
        else hide = accountAction.test(text) || /^(hide this game|remove from account|store)$/.test(text) ||
          (!social && /^(profile|hồ sơ|view my profile|xem hồ sơ của tôi)$/.test(text));
      }
      if (policy.disable_remote_play && (text === "stream" || item.classList?.contains("Stream"))) hide = true;
      mark(item, hide);
    }
    for (const item of document.querySelectorAll(rootButtonSelector)) {
      const text = norm(item.textContent || item.getAttribute("aria-label") || item.getAttribute("title"));
      // View contains a second Friends entry in Steam. Keep its parent button
      // (and compact icon-only Menu) instead of only restoring Friends' popup.
      const socialRoot = /^(view|xem|friends|bạn bè|menu|trình đơn)?$/.test(text);
      mark(item, policy.hide_account_settings && text !== "steam" && !(social && socialRoot));
    }
    for (const item of document.querySelectorAll(navSelector)) {
      const text = norm(item.textContent);
      const library = /^(library|thư viện)$/.test(text);
      const store = /^(store|cửa hàng)$/.test(text);
      // The Profile tab displays the current persona name, not a fixed label.
      // Keep Community in both modes. The remaining non-Store tab is Profile,
      // which is only restored for an online rental.
      mark(item, policy.hide_account_settings && !library && !communityAction.test(text) && !(social && !store));
      // Match the reference's one-time Library transition; never keep redirecting
      // users away from Downloads/Settings or interfere with authentication.
      if (library && policy.hide_account_settings && !selectedLibrary && document.title === "Steam") {
        selectedLibrary = true;
        item.click?.();
      }
    }
  }
  const observer = new MutationObserver(() => {
    if (queued || disposed) return;
    queued = true;
    Promise.resolve().then(() => { queued = false; filterChrome(); });
  });
  observer.observe(document.documentElement, { childList: true, subtree: true });

  function snapshot() {
    const missing = [];
    if (apiScope && policy.restrict_game_launch && actionUnsupported) missing.push("game_action_hook");
    if (apiScope && policy.disable_remote_play && remoteUnsupported) missing.push("remote_play_snapshot_api");
    if (remoteWork) missing.push("remote_play_pending");
    if (restoreFailed) missing.push("remote_play_restore_failed");
    return { installed: !disposed, applied: !disposed && missing.length === 0,
      version: 1, revision: REVISION, scope, ui_applied: !disposed && scope !== "api",
      api_applied: !disposed && apiScope && missing.length === 0, unsupported: missing };
  }
  function syncRemote() {
    if (!remote || remoteWork) return remoteWork;
    const disable = !disposed && policy.disable_remote_play;
    if (disable === remoteDisabled) return null;
    remoteWork = (async () => {
      try {
        if (!disposed && policy.disable_remote_play && !remoteDisabled) {
          const value = await remote.get();
          if (typeof value !== "boolean") { remoteUnsupported = true; return; }
          if (!disposed && policy.disable_remote_play) {
            remoteBefore = value;
            await remote.set(false);
            remoteDisabled = true;
            remoteUnsupported = false;
          }
        }
        if ((disposed || !policy.disable_remote_play) && remoteDisabled) {
          await remote.set(remoteBefore);
          remoteDisabled = false;
          restoreFailed = false;
        }
      } catch {
        if (remoteDisabled && (disposed || !policy.disable_remote_play)) restoreFailed = true;
        else remoteUnsupported = true;
      }
    })().finally(() => { remoteWork = null; });
    return remoteWork;
  }
  function syncApi() {
    if (!apiScope || !window.SteamClient?.User) return;
    client = window.SteamClient;
    if (!actionAttempted && policy.restrict_game_launch) {
      try {
        if (typeof client.Apps?.RegisterForGameActionStart === "function" && typeof client.Apps?.CancelGameAction === "function") {
          actionAttempted = true;
          actionHook = client.Apps.RegisterForGameActionStart((actionId, appId, action) => {
            if (!disposed && policy.restrict_game_launch && action === "LaunchApp" && Number(appId) !== policy.app_id)
              client.Apps.CancelGameAction(actionId);
          });
          actionUnsupported = typeof actionHook?.unregister !== "function";
        }
      } catch { actionUnsupported = true; }
    }
    if (!remote && typeof client.RemotePlay?.SetRemotePlayEnabled === "function") {
      // Current Steam exposes the snapshot through its own RemotePlay store,
      // not the old GetRemotePlayEnabled method. Read only the verified bool.
      remote = {
        get: typeof client.RemotePlay.GetRemotePlayEnabled === "function"
          ? () => client.RemotePlay.GetRemotePlayEnabled()
          : () => window.RemotePlayStore_SteamUI?.settings?.bRemotePlayEnabled,
        set: value => client.RemotePlay.SetRemotePlayEnabled(value),
      };
    }
    syncRemote();
  }
  function update(next) {
    if (disposed) return { installed: false, reason: "disposed" };
    if (next.version !== 1 || next.app_id !== policy.app_id) return { installed: false, reason: "policy_mismatch" };
    policy = next;
    const css = (policy.hide_account_settings ? accountCss + (policy.allow_friends === true ? "" : friendsCss + presenceCss) : "") +
      (policy.hide_account_settings || policy.disable_remote_play ? hiddenCss : "");
    if (style.textContent !== css) style.textContent = css;
    filterChrome(); syncApi();
    return snapshot();
  }
  // Never intercept SetUIMode, StartLogin, Guard or offline authentication.
  const api = {
    revision: REVISION, update, snapshot,
    dispose() {
      if (disposeWork) return disposeWork;
      disposed = true;
      try { actionHook?.unregister?.(); } catch {}
      observer.disconnect();
      for (const item of document.querySelectorAll('[data-vuikey-hidden="1"]')) item.removeAttribute("data-vuikey-hidden");
      style.remove();
      disposeWork = (async () => {
        await remoteWork; await syncRemote();
        const result = { disposed: true, restored: !remoteDisabled };
        if (result.restored) delete window[KEY];
        return result;
      })();
      return disposeWork;
    },
  };
  Object.defineProperty(window, KEY, { value: api, configurable: true });
  return update(config);
})