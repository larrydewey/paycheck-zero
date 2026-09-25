/* PaycheckZero browser helpers.
 * - Money boundary helpers (dollars <-> integer cents, no floats for math).
 * - Client-side invariant checks before a request is sent (spec §6).
 * - Remembered category collapse state, confirmations, recoverable errors.
 * - PWA: service worker, offline banner, offline queue + resolvable conflicts (spec §13.7).
 */
(function () {
  "use strict";

  var strings = {};
  try { strings = JSON.parse(document.getElementById("pz-i18n").textContent || "{}"); } catch (_) { strings = {}; }

  function t(key, args) {
    var s = strings["client." + key] || key;
    if (args) Object.keys(args).forEach(function (k) { s = s.split("{" + k + "}").join(String(args[k])); });
    return s;
  }

  /** Parses "$1,234.56" / "12.5" into integer cents; NaN when malformed, null when empty. */
  function cents(input) {
    var s = String(input == null ? "" : input).replace(/[\s, ]/g, "");
    if (s === "") return null;
    s = s.replace(/^[^0-9.\-]+/, "");
    var m = /^(\d{0,13})(?:\.(\d{0,2}))?$/.exec(s);
    if (!m || (m[1] === "" && (m[2] === undefined || m[2] === ""))) return NaN;
    var whole = m[1] === "" ? 0 : parseInt(m[1], 10);
    var frac = m[2] === undefined ? 0 : parseInt((m[2] + "00").slice(0, 2), 10);
    return whole * 100 + frac;
  }

  function currency() { return document.documentElement.dataset.currency || "USD"; }

  /** Formats integer cents for display with the user's currency. */
  function fmt(c) {
    var neg = c < 0, abs = Math.abs(c);
    var whole = Math.floor(abs / 100), frac = abs % 100;
    var f = new Intl.NumberFormat("en-US", { style: "currency", currency: currency() });
    // Intl needs a Number; build it from integer parts to avoid float drift.
    var str = f.format(Number(whole + "." + (frac < 10 ? "0" : "") + frac));
    return neg ? "-" + str : str;
  }

  /** Client-side guard: blocks an allocation that would over-allocate a paycheck. */
  function guard(form) {
    var input = form.querySelector("input[data-max-cents]") || form.querySelector("input.money");
    if (!input) return true;
    var err = form.querySelector(".field-error");
    var value = cents(input.value);
    var msg = "";
    if (Number.isNaN(value)) msg = t("invalid_amount");
    else if (input.dataset.maxCents !== undefined && value !== null && value > Number(input.dataset.maxCents)) {
      msg = t("over_allocated", { amount: fmt(value - Number(input.dataset.maxCents)) });
    }
    if (err) err.textContent = msg;
    if (msg) {
      input.setAttribute("aria-invalid", "true");
      if (err && !err.id) err.id = "err-" + Math.random().toString(36).slice(2);
      if (err) input.setAttribute("aria-describedby", err.id);
      input.focus();
      return false;
    }
    input.removeAttribute("aria-invalid");
    return true;
  }

  function byId(id) { return document.getElementById(id); }

  function fillIncome(s) {
    var set = function (id, v) { var el = byId(id); if (el && v !== undefined && v !== "") el.value = v; };
    set("new-name", s.name); set("new-amount", s.amount); set("new-kind", s.kind);
    set("new-date", s.date); set("new-anchor", s.anchor); set("new-days", s.days);
  }

  function toast(kind, message) {
    var box = byId("toasts");
    if (!box) return;
    var d = document.createElement("div");
    d.className = "toast " + kind;
    d.setAttribute("role", kind === "error" ? "alert" : "status");
    d.setAttribute("data-toast", "");
    var p = document.createElement("p"); p.className = "toast-msg"; p.textContent = message; d.appendChild(p);
    var b = document.createElement("button"); b.type = "button"; b.className = "toast-close"; b.textContent = "×";
    b.setAttribute("aria-label", "Dismiss"); b.onclick = function () { d.remove(); }; d.appendChild(b);
    box.appendChild(d);
    if (kind !== "error") setTimeout(function () { d.remove(); }, 5000);
  }

  /** After a rejected edit, inline fields snap back to the server's values. */
  function resetInline() {
    document.querySelectorAll("#content li.line form, #content .cat-tools form").forEach(function (f) {
      f.querySelectorAll("input").forEach(function (i) { i.value = i.defaultValue; i.removeAttribute("aria-invalid"); });
    });
  }

  window.pz = { cents: cents, fmt: fmt, guard: guard, t: t, fillIncome: fillIncome, toast: toast, resetInline: resetInline };

  // ------------------------------------------------------------------
  // Confirmations: buttons with data-confirm must be confirmed first.
  document.addEventListener("submit", function (e) {
    var btn = e.submitter;
    if (btn && btn.dataset && btn.dataset.confirm && !window.confirm(btn.dataset.confirm)) {
      e.preventDefault();
      e.stopImmediatePropagation();
    }
  }, true);

  // Category collapse state, remembered for the session (spec §14.7).
  document.addEventListener("toggle", function (e) {
    var d = e.target;
    if (!(d instanceof HTMLDetailsElement) || !d.dataset.category) return;
    var list = readCollapsed().filter(function (n) { return n !== d.dataset.category; });
    if (!d.open) list.push(d.dataset.category);
    document.cookie = "pz_collapsed=" + list.map(encodeURIComponent).join("|") + "; path=/; SameSite=Lax";
  }, true);

  function readCollapsed() {
    var m = /(?:^|; )pz_collapsed=([^;]*)/.exec(document.cookie);
    if (!m || !m[1]) return [];
    return m[1].split("|").filter(Boolean).map(function (s) { try { return decodeURIComponent(s); } catch (_) { return s; } });
  }

  // Recoverable errors for Datastar requests.
  document.addEventListener("datastar-fetch", function (e) {
    var d = e.detail || {};
    var el = d.el;
    var loader = el && el.id === "content" && el.getAttribute("aria-busy") === "true";
    if (d.type === "error" || d.type === "retries-failed") {
      if (loader) {
        var sk = el.querySelector(".skeleton"); if (sk) sk.hidden = true;
        var le = el.querySelector(".load-error"); if (le) le.hidden = false;
      } else if (d.type === "retries-failed" || (d.argsRaw && Number(d.argsRaw.status) >= 400)) {
        toast("error", t("error"));
      }
    }
  });

  document.addEventListener("click", function (e) {
    var target = e.target instanceof Element ? e.target : null;
    if (!target) return;
    if (target.closest("[data-retry]")) { window.location.reload(); }
    var closer = target.closest("[data-close-dialog]");
    if (closer) { var dlg = closer.closest("dialog"); if (dlg) dlg.close(); }
  });

  document.addEventListener("DOMContentLoaded", function () {
    // Timezone defaults to the browser's at registration (spec §13.9).
    document.querySelectorAll("input[data-timezone]").forEach(function (i) {
      try { i.value = Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC"; } catch (_) { i.value = "UTC"; }
    });
    // Toast carried across a redirect.
    try {
      var pending = sessionStorage.getItem("pz-toast");
      if (pending) {
        sessionStorage.removeItem("pz-toast");
        var box = byId("toasts"); if (box) box.insertAdjacentHTML("beforeend", pending);
      }
    } catch (_) { /* storage unavailable */ }
    // Prefill the new-month dialog from ?new=YYYY-MM and open it.
    var q = new URLSearchParams(location.search);
    if (q.get("new")) {
      var tryOpen = function () {
        var dlg = byId("new-month"), input = document.querySelector("[data-new-month]");
        if (dlg && input) { input.value = q.get("new"); if (!dlg.open) dlg.showModal(); return true; }
        return false;
      };
      if (!tryOpen()) {
        var obs = new MutationObserver(function () { if (tryOpen()) obs.disconnect(); });
        obs.observe(document.body, { childList: true, subtree: true });
      }
    }
    if (location.pathname === "/login" && window.caches) { caches.delete("pz-pages-v1"); }
    updateOnline();
    refreshSyncBanner();
    flush();
  });

  // Restore from snapshot (JSON upload).
  document.addEventListener("submit", function (e) {
    var form = e.target;
    if (!(form instanceof HTMLFormElement) || !form.hasAttribute("data-restore")) return;
    e.preventDefault();
    var file = form.querySelector("input[type=file]").files[0];
    if (!file) return;
    file.text().then(function (text) {
      var snapshot;
      try { snapshot = JSON.parse(text); } catch (_) { toast("error", t("restore_failed")); return null; }
      return fetch("/ui/months/import", {
        method: "POST", credentials: "same-origin",
        headers: { "Content-Type": "application/json", "Datastar-Request": "true" },
        body: JSON.stringify({ snapshot: snapshot, replace: !!form.querySelector("[name=replace]:checked") })
      }).then(function (r) { return r.json(); }).then(function (res) {
        if (res.redirect) location.assign(res.redirect);
        else toast("error", (res.error || t("restore_failed")));
      });
    }).catch(function () { toast("error", t("restore_failed")); });
  });

  // ------------------------------------------------------------------
  // PWA: service worker + offline queue.
  if ("serviceWorker" in navigator) {
    window.addEventListener("load", function () { navigator.serviceWorker.register("/sw.js").catch(function () {}); });
  }

  function updateOnline() {
    var b = byId("offline-banner");
    if (b) b.hidden = navigator.onLine;
  }
  window.addEventListener("online", function () { updateOnline(); flush(); });
  window.addEventListener("offline", updateOnline);

  var DB = "pz-offline", STORE_Q = "queue", STORE_C = "conflicts";
  function db() {
    return new Promise(function (resolve, reject) {
      if (!window.indexedDB) { reject(new Error("no indexeddb")); return; }
      var req = indexedDB.open(DB, 1);
      req.onupgradeneeded = function () {
        var d = req.result;
        if (!d.objectStoreNames.contains(STORE_Q)) d.createObjectStore(STORE_Q, { keyPath: "op_id" });
        if (!d.objectStoreNames.contains(STORE_C)) d.createObjectStore(STORE_C, { keyPath: "op_id" });
      };
      req.onsuccess = function () { resolve(req.result); };
      req.onerror = function () { reject(req.error); };
    });
  }
  function tx(store, mode, fn) {
    return db().then(function (d) {
      return new Promise(function (resolve, reject) {
        var tr = d.transaction(store, mode), s = tr.objectStore(store), out;
        var r = fn(s);
        if (r) r.onsuccess = function () { out = r.result; };
        tr.oncomplete = function () { resolve(out); };
        tr.onerror = function () { reject(tr.error); };
      });
    });
  }
  var all = function (store) { return tx(store, "readonly", function (s) { return s.getAll(); }); };
  var put = function (store, v) { return tx(store, "readwrite", function (s) { return s.put(v); }); };
  var del = function (store, k) { return tx(store, "readwrite", function (s) { return s.delete(k); }); };

  function uuid() {
    if (window.crypto && crypto.randomUUID) return crypto.randomUUID();
    return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, function (c) {
      var r = Math.random() * 16 | 0; return (c === "x" ? r : (r & 3 | 8)).toString(16);
    });
  }

  function txFromForm(form) {
    var fd = new FormData(form);
    var amount = cents(fd.get("amount"));
    if (amount === null || Number.isNaN(amount) || amount === 0) return null;
    var signed = fd.get("direction") === "income" ? amount : -amount;
    var text = function (k) { var v = String(fd.get(k) || "").trim(); return v === "" ? null : v; };
    return {
      date: String(fd.get("date") || ""), amount: signed, payee: text("payee"), notes: text("notes"),
      expense_line_id: text("expense_line_id"), paycheck_id: text("paycheck_id")
    };
  }

  function opFromForm(form) {
    var kind = form.dataset.offline, month = form.dataset.month, op = { op_id: uuid(), kind: kind, month_id: month };
    if (kind === "create_transaction") {
      op.id = uuid(); op.tx = txFromForm(form); if (!op.tx || !op.tx.date) return null;
    } else if (kind === "update_transaction") {
      op.id = form.dataset.tx; op.base = JSON.parse(form.dataset.base); op.tx = txFromForm(form); if (!op.tx) return null;
    } else if (kind === "delete_transaction") {
      op.id = form.dataset.tx; op.base = JSON.parse(form.dataset.base);
    } else if (kind === "actual") {
      op.kind = "set_actual"; op.paycheck_id = form.dataset.paycheck;
      op.base_actual = form.dataset.baseActual === undefined ? null : Number(form.dataset.baseActual);
      var v = cents(new FormData(form).get("amount"));
      if (Number.isNaN(v)) return null;
      op.actual = v;
    } else return null;
    return op;
  }

  function optimistic(form, op) {
    var badge = '<span class="badge">' + escapeHtml(t("pending")) + "</span>";
    if (op.kind === "create_transaction") {
      var table = byId("tx-table");
      var row = document.createElement("tr");
      row.className = "pending";
      row.innerHTML = "<td>" + escapeHtml(op.tx.date) + "</td><td>" + escapeHtml(op.tx.payee || "") + " " + badge +
        "</td><td></td><td></td><td class=\"num\">" + escapeHtml(fmt(op.tx.amount)) + "</td><td></td>";
      if (table) table.querySelector("tbody").prepend(row);
      else form.insertAdjacentHTML("afterend", '<table class="table" id="tx-table"><tbody></tbody></table>'), byId("tx-table").querySelector("tbody").append(row);
      form.reset();
    } else if (op.kind === "update_transaction" || op.kind === "delete_transaction") {
      var tr = form.closest("tr");
      if (tr) { tr.classList.add("pending"); tr.cells[1].insertAdjacentHTML("beforeend", " " + badge); }
      var det = form.closest("details"); if (det) det.open = false;
    } else if (op.kind === "set_actual") {
      form.insertAdjacentHTML("beforeend", " " + badge);
    }
  }

  function escapeHtml(s) {
    return String(s).replace(/[&<>"']/g, function (c) { return { "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]; });
  }

  // Offline submissions are queued instead of sent.
  document.addEventListener("submit", function (e) {
    var form = e.target;
    if (navigator.onLine || !(form instanceof HTMLFormElement) || !form.dataset.offline) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    var op = opFromForm(form);
    if (!op) { toast("error", t("invalid_amount")); return; }
    put(STORE_Q, op).then(function () {
      optimistic(form, op);
      toast("warning", t("queued"));
      refreshSyncBanner();
    }).catch(function () { toast("error", t("error")); });
  }, true);

  var flushing = false;
  function flush() {
    if (flushing || !navigator.onLine) return Promise.resolve();
    flushing = true;
    return all(STORE_Q).then(function (ops) {
      if (!ops || ops.length === 0) return null;
      return fetch("/sync", {
        method: "POST", credentials: "same-origin",
        headers: { "Content-Type": "application/json", "x-pz-sync": "1" },
        body: JSON.stringify({ ops: ops })
      }).then(function (r) { if (!r.ok) throw new Error("sync " + r.status); return r.json(); }).then(function (res) {
        var applied = 0, conflicts = 0, work = [];
        (res.results || []).forEach(function (r) {
          var op = ops.find(function (o) { return o.op_id === r.op_id; });
          if (!op) return;
          if (r.status === "applied") applied++;
          else { conflicts++; work.push(put(STORE_C, { op_id: op.op_id, op: op, result: r })); }
          work.push(del(STORE_Q, op.op_id));
        });
        return Promise.all(work).then(function () {
          if (applied) { try { sessionStorage.setItem("pz-toast", '<div class="toast success" role="status" data-toast><p class="toast-msg">' + escapeHtml(t("synced", { n: applied })) + "</p></div>"); } catch (_) {} }
          if (applied || conflicts) location.reload();
        });
      });
    }).catch(function () { /* stay queued; retry on next online event */ })
      .then(function () { flushing = false; refreshSyncBanner(); });
  }

  function refreshSyncBanner() {
    var banner = byId("sync-banner");
    if (!banner) return;
    Promise.all([all(STORE_Q), all(STORE_C)]).then(function (r) {
      var q = r[0] || [], c = r[1] || [];
      var text = [];
      if (q.length) text.push(q.length + " " + t("pending").toLowerCase());
      if (c.length) text.push(t("conflicts", { n: c.length }));
      byId("sync-text").textContent = text.join(" · ");
      byId("sync-review").hidden = c.length === 0;
      banner.hidden = text.length === 0;
    }).catch(function () { banner.hidden = true; });
  }

  function describe(v) {
    if (v === null || v === undefined) return t("deleted");
    if (typeof v !== "object") return String(v);
    if ("amount" in v) return [v.date, v.payee || "", fmt(v.amount)].join(" · ");
    if ("actual_amount" in v) return v.actual_amount === null ? "—" : fmt(v.actual_amount);
    return JSON.stringify(v);
  }

  document.addEventListener("click", function (e) {
    if (!(e.target instanceof Element) || !e.target.closest("#sync-review")) return;
    all(STORE_C).then(function (items) {
      var list = byId("conflict-list");
      list.innerHTML = "";
      items.forEach(function (c) {
        var mine = c.op.tx ? describe(c.op.tx) : (c.op.kind === "set_actual" ? describe({ actual_amount: c.op.actual }) : t("deleted"));
        var div = document.createElement("div");
        div.className = "card";
        div.innerHTML = "<p><strong>" + escapeHtml(c.result.message || "") + "</strong></p>" +
          "<p>" + escapeHtml(t("mine")) + ": " + escapeHtml(mine) + "</p>" +
          (c.result.status === "conflict" ? "<p>" + escapeHtml(t("server")) + ": " + escapeHtml(describe(c.result.server)) + "</p>" : "");
        var keep = document.createElement("button"); keep.type = "button"; keep.className = "btn primary small"; keep.textContent = t("keep_mine");
        keep.onclick = function () { var op = c.op; op.force = true; op.op_id = uuid(); del(STORE_C, c.op_id).then(function () { return put(STORE_Q, op); }).then(function () { byId("conflict-dialog").close(); flush(); }); };
        var drop = document.createElement("button"); drop.type = "button"; drop.className = "btn small"; drop.textContent = t("keep_server");
        drop.onclick = function () { del(STORE_C, c.op_id).then(function () { div.remove(); refreshSyncBanner(); }); };
        if (c.result.status === "conflict") div.appendChild(keep);
        div.appendChild(document.createTextNode(" "));
        div.appendChild(drop);
        list.appendChild(div);
      });
      byId("conflict-dialog").showModal();
    });
  });
})();
