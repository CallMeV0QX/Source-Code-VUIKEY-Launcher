(() => {
  if (window.top !== window.self || window.vuikey) return;
  const pending = new Map();
  const allowed = new Set(['status', 'storage', 'launch', 'finish', 'reload', 'open_google_login', 'app_access', 'offline_start', 'update_check', 'update_start']);
  const receive = message => {
    const waiter = pending.get(message.id);
    if (!waiter) return;
    pending.delete(message.id);
    clearTimeout(waiter.timer);
    if (message.ok) waiter.resolve(message.data);
    else waiter.reject(new Error(message.error || 'Launcher không thể hoàn tất thao tác.'));
  };
  Object.defineProperty(window, '__vuikeyReceive', {value:receive, configurable:false, writable:false});
  Object.defineProperty(window, 'vuikey', {configurable:false, writable:false, value:Object.freeze({
    version:1,
    features:Object.freeze(['google_login', 'app_access', 'app_entry', 'storage_v1', 'offline_start_v1', 'session_revoke_v1', 'app_update_v1']),
    invoke(action, payload={}) {
      if (!allowed.has(action)) return Promise.reject(new Error('Lệnh không được hỗ trợ.'));
      return new Promise((resolve,reject) => {
        const id = crypto.randomUUID();
        const timer = setTimeout(() => {pending.delete(id);reject(new Error('Launcher phản hồi quá lâu. Kiểm tra Steam rồi thử lại.'));},75000);
        pending.set(id,{resolve,reject,timer});
        window.ipc.postMessage(JSON.stringify({id,action,payload}));
      });
    }
  })});
})()