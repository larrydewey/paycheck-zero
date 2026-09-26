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

  /**
   * Parses money typed by the user into integer cents: "$1,234.56", "12.5",
   * or arithmetic like "600 + 80", "1200/2", "(50+25)*2". Uses exact BigInt
   * fractions and rounds half-to-even once, matching the server. NaN when
   * malformed or negative, null when empty.
   */
  function cents(input) {
    var s = String(input == null ? "" : input).replace(/[\s,\u00a0]/g, "");
    if (s === "") return null;
    if (s.length > 64) return NaN;
    s = s.replace(/[^0-9+\-*/().xX\u00d7\u00f7]/g, "").replace(/[xX\u00d7]/g, "*").replace(/\u00f7/g, "/");
    if (s === "") return NaN;
    try {
      var v = evaluate(s);
      if (v === null) return NaN;
      var num = v[0] * 100n, den = v[1];
      var q = num / den, r = num % den;
      if (r < 0n) { r += den; q -= 1n; }
      if (r * 2n > den || (r * 2n === den && q % 2n !== 0n)) q += 1n;
      if (q < 0n) return NaN;
      return Number(q);
    } catch (_) { return NaN; }
  }

  function evaluate(src) {
    var i = 0;
    var gcd = function (a, b) { a = a < 0n ? -a : a; b = b < 0n ? -b : b; while (b) { var t = a % b; a = b; b = t; } return a || 1n; };
    var norm = function (n, d) { if (d === 0n) throw 0; if (d < 0n) { n = -n; d = -d; } var g = gcd(n, d); return [n / g, d / g]; };
    function number() {
      var m = /^(\d{0,13})(?:\.(\d{0,6}))?/.exec(src.slice(i));
      if (!m || m[0] === "" || m[0] === ".") throw 0;
      i += m[0].length;
      var frac = m[2] || "";
      return norm(BigInt((m[1] || "") + frac || "0"), 10n ** BigInt(frac.length));
    }
    function factor() {
      if (src[i] === "-") { i++; var v = factor(); return [-v[0], v[1]]; }
      if (src[i] === "(") { i++; var e = expr(); if (src[i] !== ")") throw 0; i++; return e; }
      return number();
    }
    function term() {
      var v = factor();
      while (src[i] === "*" || src[i] === "/") {
        var op = src[i++], r = factor();
        v = op === "*" ? norm(v[0] * r[0], v[1] * r[1]) : norm(v[0] * r[1], v[1] * r[0]);
      }
      return v;
    }
    function expr() {
      var v = term();
      while (src[i] === "+" || src[i] === "-") {
        var op = src[i++], r = term();
        v = op === "+" ? norm(v[0] * r[1] + r[0] * v[1], v[1] * r[1]) : norm(v[0] * r[1] - r[0] * v[1], v[1] * r[1]);
      }
      return v;
    }
    var out = expr();
    if (i !== src.length) return null;
    var plain = !/[+*/()]/.test(src) && src.slice(1).indexOf("-") < 0;
    if (plain && /\.\d{3,}/.test(src)) return null;
    return out;
  }

  /** "600+80" → "680.00" in the field, so users see what will be saved. */
  function normalizeMoney(input) {
    if (!/[+\-*/()xX\u00d7\u00f7]/.test(input.value.replace(/^\s*-/, ""))) return;
    var c = cents(input.value);
    if (c === null || Number.isNaN(c)) return;
    input.value = (Math.floor(c / 100)) + "." + String(c % 100).padStart(2, "0");
  }
  document.addEventListener("blur", function (e) {
    var el = e.target;
    if (el instanceof HTMLInputElement && el.inputMode === "decimal") normalizeMoney(el);
  }, true);
  document.addEventListener("submit", function (e) {
    var f = e.target;
    if (f instanceof HTMLFormElement) f.querySelectorAll("input[inputmode=decimal]").forEach(normalizeMoney);
  }, true);

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

  /** After a successful create, empty the submitting form (marked data-clear). */
  var pendingClear = {};
  document.addEventListener("datastar-fetch", function (e) {
    var el = e.detail && e.detail.el;
    var f = el && el.closest ? el.closest("form[data-clear]") : null;
    if (f && f.id && e.detail.type === "started") pendingClear[f.id] = true;
    if (f && f.id && e.detail.type === "finished") setTimeout(function () { delete pendingClear[f.id]; }, 0);
  });
  function clearDone() {
    Object.keys(pendingClear).map(function (id) { return byId(id); }).filter(Boolean).forEach(function (f) {
      f.reset();
      f.querySelectorAll("select, input").forEach(function (el) {
        el.dispatchEvent(new Event("input", { bubbles: true }));
        el.dispatchEvent(new Event("change", { bubbles: true }));
      });
      var err = f.querySelector(".field-error"); if (err) err.textContent = "";
      // Done with this form: let a quick-add row settle now, not under the next tap.
      if (f.contains(document.activeElement)) document.activeElement.blur();
      f.classList.remove("keep");
    });
  }

  // Keep per-paycheck funding panels open across re-renders (a morph would
  // otherwise close them after every edit).
  var openFunding = {};
  try { openFunding = JSON.parse(sessionStorage.getItem("pz-funding-open") || "{}"); } catch (_) { openFunding = {}; }
  document.addEventListener("click", function (e) {
    var sum = e.target instanceof Element ? e.target.closest("details[data-line-funding] > summary") : null;
    if (!sum) return;
    var d = sum.parentElement, id = d.dataset.lineFunding;
    if (d.open) delete openFunding[id]; else openFunding[id] = true;
    try { sessionStorage.setItem("pz-funding-open", JSON.stringify(openFunding)); } catch (_) {}
  });
  function reopenFunding() {
    document.querySelectorAll("details[data-line-funding]").forEach(function (d) {
      if (openFunding[d.dataset.lineFunding] && !d.open) d.open = true;
    });
  }
  new MutationObserver(reopenFunding).observe(document.documentElement, { childList: true, subtree: true, attributes: true, attributeFilter: ["open"] });

  // Inline edits also save when the pointer leaves the row with a changed value.
  document.addEventListener("mouseout", function (e) {
    var from = e.target instanceof Element ? e.target.closest("li.line, .cat-tools, .debt-form") : null;
    if (!from || (e.relatedTarget instanceof Node && from.contains(e.relatedTarget))) return;
    from.querySelectorAll("form").forEach(function (f) {
      if (f.hasAttribute("data-clear") || f.classList.contains("is-busy")) return;
      var dirty = Array.prototype.some.call(f.querySelectorAll("input[type=text]"), function (i) { return i.value !== i.defaultValue; });
      if (dirty && f.checkValidity()) f.requestSubmit();
    });
  });

  // Unified transaction form: "Split into parts" turns the line/paycheck
  // fields into part rows; removing parts back to one makes it plain again.
  function editorOf(el) { return el && el.closest ? el.closest("[data-split-editor]") : null; }
  function formOf(ed) { return ed.closest("form"); }
  function relabel(ed) {
    var rows = ed.querySelectorAll("[data-part]"), split = rows.length > 1;
    ed.classList.toggle("is-split", split);
    Array.prototype.forEach.call(rows, function (row, i) {
      row.querySelectorAll("[name]").forEach(function (el) { el.name = el.name.replace(/_\d+$/, "_" + i); });
      row.querySelectorAll("[id]").forEach(function (el) { el.id = el.id.replace(/-\d+$/, "-" + i); });
      row.querySelectorAll("label[for]").forEach(function (el) { el.htmlFor = el.htmlFor.replace(/-\d+$/, "-" + i); });
      var n = String(i + 1), ls = row.querySelectorAll("select");
      ls[0].setAttribute("aria-label", split ? ed.dataset.labelPartLine.replace("{n}", n) : ed.dataset.labelLine);
      ls[1].setAttribute("aria-label", split ? ed.dataset.labelPartPaycheck.replace("{n}", n) : ed.dataset.labelPaycheck);
      row.querySelector("[data-part-amount]").setAttribute("aria-label", ed.dataset.labelPartAmount.replace("{n}", n));
      row.querySelector("[data-remove-part]").setAttribute("aria-label", ed.dataset.labelRemove.replace("{n}", n));
    });
  }
  function splitLeft(ed) {
    var total = cents(formOf(ed).querySelector("[data-split-total]").value) || 0;
    var sum = 0;
    ed.querySelectorAll("[data-part-amount]").forEach(function (i) { var v = cents(i.value); if (v && !Number.isNaN(v)) sum += v; });
    return total - sum;
  }
  function updateSplit(ed) {
    var out = ed.querySelector("[data-split-left-amt]");
    if (!out || !ed.classList.contains("is-split")) return;
    var left = splitLeft(ed);
    out.textContent = fmt(left);
    ed.querySelector("[data-split-left]").classList.toggle("bad", left !== 0);
  }
  document.addEventListener("input", function (e) {
    var f = e.target instanceof Element ? e.target.closest("form") : null;
    var ed = f && f.querySelector("[data-split-editor]");
    if (ed) updateSplit(ed);
  });
  document.addEventListener("click", function (e) {
    var t0 = e.target instanceof Element ? e.target : null;
    if (!t0) return;
    var add = t0.closest("[data-add-part]"), rm = t0.closest("[data-remove-part]");
    var ed = editorOf(add || rm);
    if (!ed) return;
    var list = ed.querySelector("[data-parts]"), rows = list.querySelectorAll("[data-part]");
    if (add) {
      if (rows.length === 1) {
        // Becoming a split: the whole amount starts in part 1.
        var total = formOf(ed).querySelector("[data-split-total]");
        normalizeMoney(total);
        rows[0].querySelector("[data-part-amount]").value = total.value;
      }
      var copy = rows[rows.length - 1].cloneNode(true);
      copy.querySelectorAll("input").forEach(function (i) { i.value = ""; i.defaultValue = ""; });
      copy.querySelectorAll("select").forEach(function (sel) { sel.selectedIndex = 0; });
      list.appendChild(copy);
      relabel(ed);
      copy.querySelector("select").focus();
    } else if (rows.length > 1) {
      rm.closest("[data-part]").remove();
      if (list.querySelectorAll("[data-part]").length === 1) list.querySelector("[data-part-amount]").value = "";
      relabel(ed);
    }
    updateSplit(ed);
  });
  /** Plain transactions pass; splits must add up to the Amount. */
  function checkSplit(form) {
    form.querySelectorAll("input[inputmode=decimal]").forEach(normalizeMoney);
    var ed = form.querySelector("[data-split-editor]");
    if (!ed || !ed.classList.contains("is-split")) return true;
    var err = ed.querySelector(".field-error");
    if (splitLeft(ed) !== 0) { if (err) err.textContent = t("split_mismatch"); return false; }
    if (err) err.textContent = "";
    return true;
  }
  // Forms that can't be queued offline explain why instead of failing.
  document.addEventListener("submit", function (e) {
    var f = e.target;
    if (navigator.onLine || !(f instanceof HTMLFormElement)) return;
    if (!f.hasAttribute("data-online-only") && !f.querySelector("[data-split-editor].is-split")) return;
    e.preventDefault(); e.stopImmediatePropagation();
    toast("warning", t("online_only"));
  }, true);

  // Report drill-down: category rows expand to their lines.
  document.addEventListener("click", function (e) {
    var b = e.target instanceof Element ? e.target.closest("[data-toggle-rows]") : null;
    if (!b) return;
    var open = b.getAttribute("aria-expanded") !== "true";
    b.setAttribute("aria-expanded", String(open));
    b.closest("table").querySelectorAll('[data-parent="' + b.dataset.toggleRows + '"]').forEach(function (r) { r.hidden = !open; });
  });

  // GET forms that produce a file download it (Safari would display it).
  document.addEventListener("submit", function (e) {
    var f = e.target;
    if (!(f instanceof HTMLFormElement) || !f.hasAttribute("data-download-form")) return;
    e.preventDefault();
    var url = new URL(f.action, location.href);
    new FormData(f).forEach(function (v, k) { url.searchParams.set(k, String(v)); });
    var a = document.createElement("a");
    a.href = url.pathname + url.search; a.download = ""; document.body.appendChild(a); a.click(); a.remove();
  });

  // Paycheck view filter: only lines this paycheck funds (remembered).
  document.addEventListener("click", function (e) {
    var b = e.target instanceof Element ? e.target.closest("[data-only-funded]") : null;
    if (!b) return;
    var on = b.getAttribute("aria-pressed") !== "true";
    b.setAttribute("aria-pressed", String(on));
    var sec = byId("funding"); if (sec) sec.classList.toggle("only-funded", on);
    document.cookie = "pz_only_funded=" + (on ? "1" : "0") + "; path=/; SameSite=Lax; max-age=31536000";
  });

  // Theme: follow the device, or a light/dark choice saved on this device.
  var THEMES = ["system", "light", "dark"];
  function currentTheme() {
    try { var v = localStorage.getItem("pz-theme"); return v === "light" || v === "dark" ? v : "system"; } catch (_) { return "system"; }
  }
  function applyTheme(v, announce) {
    var root = document.documentElement;
    if (v === "system") root.removeAttribute("data-theme"); else root.setAttribute("data-theme", v);
    try { if (v === "system") localStorage.removeItem("pz-theme"); else localStorage.setItem("pz-theme", v); } catch (_) {}
    document.querySelectorAll("[data-theme-toggle]").forEach(function (b) {
      b.dataset.mode = v;
      b.setAttribute("aria-label", t("theme_" + v)); b.title = t("theme_" + v);
    });
    document.querySelectorAll('input[name="pz-theme"]').forEach(function (r) { r.checked = r.value === v; });
    var dark = v === "dark" || (v === "system" && window.matchMedia && matchMedia("(prefers-color-scheme: dark)").matches);
    document.querySelectorAll('meta[name="theme-color"]').forEach(function (m) { m.setAttribute("content", dark ? "#0b1220" : "#0f766e"); m.removeAttribute("media"); });
    if (announce) toast("success", t("theme_now_" + v));
  }
  document.addEventListener("click", function (e) {
    var b = e.target.closest && e.target.closest("[data-theme-toggle]");
    if (!b) return;
    applyTheme(THEMES[(THEMES.indexOf(currentTheme()) + 1) % THEMES.length], true);
  });
  document.addEventListener("change", function (e) {
    if (e.target && e.target.name === "pz-theme") applyTheme(e.target.value, false);
  });
  function syncTheme() { applyTheme(currentTheme(), false); }

  // Bottom sheet: opens instantly with a placeholder, content streams in.
  var sheetOpener = null;
  /** Opens the sheet; returns false (so the caller skips its request) when
   *  offline, in which case an offline copy of the sheet is shown if there is one. */
  function openSheet(url) {
    var d = byId("sheet");
    if (!d) return false;
    sheetOpener = document.activeElement;
    var body = byId("sheet-body");
    var online = navigator.onLine;
    var offline = online ? null : offlineSheet(String(url || ""));
    if (offline) { body.replaceWith(offline); }
    else if (!online) { body.innerHTML = '<div class="sheet-head"><h2 id="sheet-title">' + escapeHtml(t("offline_title")) + '</h2><button type="button" class="icon-btn" data-close-dialog aria-label="' + escapeHtml(t("close")) + '">×</button></div><div class="sheet-content"><p>' + escapeHtml(t("online_only")) + "</p></div>"; }
    else body.innerHTML = '<div class="sheet-loading" aria-busy="true"><div class="sk sk-title"></div><div class="sk sk-row"></div><div class="sk sk-row"></div></div>';
    var toasts = byId("toasts"); if (toasts) d.appendChild(toasts); // keep messages above the sheet
    if (!d.open) d.showModal();
    d.classList.remove("closing");
    return online;
  }

  /** Builds a sheet from the offline templates a month page carries. */
  function offlineSheet(url) {
    var path = url.split("?")[0];
    var tpl, node;
    if (path.indexOf("/ui/sheet/tx/new/") === 0) {
      tpl = byId("offline-tx");
      return tpl ? tpl.content.firstElementChild.cloneNode(true) : null;
    }
    if (path.indexOf("/ui/sheet/paycheck/") === 0) {
      tpl = byId("offline-paycheck");
      if (!tpl || tpl.dataset.paycheck !== path.split("/").pop()) return null;
      return tpl.content.firstElementChild.cloneNode(true);
    }
    if (path.indexOf("/ui/sheet/tx/") === 0) {
      var id = path.split("/").pop();
      var row = document.querySelector('#tx-list li.tx[data-tx="' + id + '"]');
      tpl = byId("offline-tx");
      if (!row || !row.dataset.base || !tpl) return null;
      var base = JSON.parse(row.dataset.base);
      node = tpl.content.firstElementChild.cloneNode(true);
      var title = node.querySelector("#sheet-title"); if (title) title.textContent = t("edit_tx");
      var form = node.querySelector("form");
      form.removeAttribute("id"); form.removeAttribute("data-clear"); form.removeAttribute("data-on:submit__prevent");
      form.dataset.offline = "update_transaction"; form.dataset.tx = id; form.dataset.base = row.dataset.base;
      var set = function (name, v) { var el = form.querySelector('[name="' + name + '"]'); if (el) el.value = v == null ? "" : v; };
      var abs = Math.abs(base.amount);
      set("amount", Math.floor(abs / 100) + "." + String(abs % 100).padStart(2, "0"));
      set("payee", base.payee); set("notes", base.notes); set("date", base.date);
      set("part_line_0", base.expense_line_id); set("part_paycheck_0", base.paycheck_id); set("account_id", base.account_id);
      var acct = form.querySelector("[data-remember-account]"); if (acct) acct.removeAttribute("data-remember-account");
      var dir = form.querySelector('input[name="direction"][value="' + (base.amount > 0 ? "income" : "expense") + '"]'); if (dir) dir.checked = true;
      var save = form.querySelector('button[type="submit"]'); if (save) save.textContent = t("save");
      var del = document.createElement("form");
      del.className = "dialog-danger"; del.dataset.offline = "delete_transaction"; del.dataset.month = form.dataset.month;
      del.dataset.tx = id; del.dataset.base = row.dataset.base;
      del.innerHTML = '<button type="submit" class="btn small danger" data-confirm="' + escapeHtml(t("delete_tx_confirm")) + '">' + escapeHtml(t("delete_tx")) + "</button>";
      form.after(del);
      return node;
    }
    return null;
  }
  function closeSheet() {
    var d = byId("sheet");
    if (!d || !d.open) return;
    var toasts = byId("toasts"); if (toasts) document.body.appendChild(toasts);
    d.close();
    if (sheetOpener && document.contains(sheetOpener)) { try { sheetOpener.focus({ preventScroll: true }); } catch (_) {} }
  }
  document.addEventListener("click", function (e) {
    var d = byId("sheet");
    // Tap on the backdrop closes the sheet.
    if (d && e.target === d) closeSheet();
  });
  document.addEventListener("close", function (e) {
    if (e.target && e.target.id === "sheet") { var toasts = byId("toasts"); if (toasts && toasts.parentElement !== document.body) document.body.appendChild(toasts); }
  }, true);
  // Swipe the sheet down to dismiss (phones).
  (function () {
    var startY = null, dy = 0;
    document.addEventListener("touchstart", function (e) {
      var d = byId("sheet"); if (!d || !d.open) return;
      var grip = e.target instanceof Element && (e.target.closest(".sheet-grip") || e.target.closest(".sheet-head"));
      if (!grip) return;
      startY = e.touches[0].clientY; dy = 0;
    }, { passive: true });
    document.addEventListener("touchmove", function (e) {
      if (startY === null) return;
      dy = Math.max(0, e.touches[0].clientY - startY);
      byId("sheet").style.transform = "translateY(" + dy + "px)";
    }, { passive: true });
    document.addEventListener("touchend", function () {
      if (startY === null) return;
      var d = byId("sheet"); d.style.transform = "";
      if (dy > 90) closeSheet();
      startY = null;
    });
  })();

  // Live filter for lists with a search box (Spending).
  document.addEventListener("input", function (e) {
    var el = e.target;
    if (!(el instanceof HTMLInputElement) || !el.dataset.filterList) return;
    var q = el.value.trim().toLowerCase();
    var list = byId(el.dataset.filterList);
    if (!list) return;
    var shown = 0;
    list.querySelectorAll("li.tx").forEach(function (li) {
      var hit = !q || (li.dataset.search || li.textContent.toLowerCase()).indexOf(q) >= 0;
      li.hidden = !hit; if (hit) shown++;
    });
    list.querySelectorAll("li.tx-day").forEach(function (h) { h.hidden = !!q; });
    var empty = document.querySelector('[data-empty-for="' + el.dataset.filterList + '"]');
    if (empty) empty.hidden = shown !== 0;
  });

  window.pz = { openSheet: openSheet, closeSheet: closeSheet, clearDone: clearDone, checkSplit: checkSplit, cents: cents, fmt: fmt, guard: guard, t: t, fillIncome: fillIncome, toast: toast, resetInline: resetInline };

  // ------------------------------------------------------------------
  // Confirmations: buttons with data-confirm open a styled, accessible dialog.
  function confirmDialog() {
    var d = byId("pz-confirm");
    if (d) return d;
    d = document.createElement("dialog");
    d.id = "pz-confirm";
    d.setAttribute("aria-labelledby", "pz-confirm-msg");
    d.innerHTML = '<p id="pz-confirm-msg" class="confirm-msg"></p><div class="dialog-actions">' +
      '<button type="button" class="btn" data-act="cancel"></button><button type="button" class="btn danger-solid" data-act="ok"></button></div>';
    document.body.appendChild(d);
    return d;
  }
  document.addEventListener("submit", function (e) {
    var btn = e.submitter, form = e.target;
    if (!btn || !btn.dataset || !btn.dataset.confirm) return;
    if (form.dataset.confirmed === "1") { delete form.dataset.confirmed; return; }
    e.preventDefault();
    e.stopImmediatePropagation();
    var d = confirmDialog();
    d.querySelector("#pz-confirm-msg").textContent = btn.dataset.confirm;
    var ok = d.querySelector("[data-act=ok]"), cancel = d.querySelector("[data-act=cancel]");
    ok.textContent = (btn.textContent || "").trim() || t("confirm");
    cancel.textContent = t("cancel");
    ok.onclick = function () { d.close(); form.dataset.confirmed = "1"; form.requestSubmit(btn); };
    cancel.onclick = function () { d.close(); btn.focus(); };
    d.showModal();
    cancel.focus();
  }, true);

  // Saving feedback: the acting form is marked busy; a slim progress bar
  // appears when a request takes longer than a blink.
  var inflight = 0, barTimer = null;
  function bar(on) {
    var b = byId("pz-progress");
    if (!b) { b = document.createElement("div"); b.id = "pz-progress"; b.setAttribute("aria-hidden", "true"); document.body.appendChild(b); }
    b.classList.toggle("on", on);
  }
  document.addEventListener("datastar-fetch", function (e) {
    var d = e.detail || {}, el = d.el;
    if (!el || el.id === "content") return;
    var form = el.closest ? (el.closest("form") || el) : el;
    if (d.type === "started") {
      // Close a dialog before the server re-renders the page: a morph that
      // drops `open` from a modal dialog would leave the page inert.
      var dlg = form.closest && form.closest("dialog");
      if (dlg && dlg.open && dlg.id !== "sheet") dlg.close();
      form.classList.add("is-busy"); form.setAttribute("aria-busy", "true");
      inflight++;
      if (!barTimer) barTimer = setTimeout(function () { if (inflight > 0) bar(true); }, 150);
    } else if (d.type === "finished" || d.type === "error" || d.type === "retries-failed") {
      form.classList.remove("is-busy"); form.removeAttribute("aria-busy");
      inflight = Math.max(0, inflight - 1);
      if (inflight === 0) { clearTimeout(barTimer); barTimer = null; bar(false); }
    }
  });


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
    if (closer) { var dlg = closer.closest("dialog"); if (dlg && dlg.id === "sheet") closeSheet(); else if (dlg) dlg.close(); }
  });

  // If Datastar never starts (old browser, blocked script), say so instead of
  // leaving buttons that silently do nothing.
  var datastarReady = false;
  document.addEventListener("datastar-ready", function () { datastarReady = true; });
  window.addEventListener("load", function () {
    setTimeout(function () {
      if (datastarReady) return;
      var b = byId("script-banner"); if (b) b.hidden = false;
    }, 4000);
  });

  // Tab badges (e.g. transactions that still need a line) come from a
  // marker in the freshly rendered content.
  function syncBadges() {
    var st = byId("nav-state");
    var n = st ? Number(st.dataset.needsLine || 0) : 0;
    document.querySelectorAll("[data-badge=needs-line]").forEach(function (b) {
      b.textContent = n > 0 ? String(n) : ""; b.hidden = !(n > 0);
    });
  }
  // Quick-add rows expand while focused. Collapse a moment after focus
  // leaves, so the layout doesn't jump under the finger mid-tap.
  document.addEventListener("focusin", function (e) {
    var row = e.target.closest && e.target.closest(".add-line");
    if (row) { clearTimeout(row._keepT); row.classList.add("keep"); }
  });
  document.addEventListener("focusout", function (e) {
    var row = e.target.closest && e.target.closest(".add-line");
    if (!row) return;
    clearTimeout(row._keepT);
    row._keepT = setTimeout(function () { if (!row.contains(document.activeElement)) row.classList.remove("keep"); }, 400);
  });

  // Bank forms share the "Import transactions from" date in the sheet.
  document.addEventListener("submit", function (e) {
    var f = e.target; if (!(f instanceof HTMLFormElement)) return;
    var from = byId("bank-from");
    if (from) f.querySelectorAll("input[data-from]").forEach(function (i) { i.value = from.value; });
  }, true);
  function copyFrom(form) {
    var from = byId("bank-from");
    if (from) form.querySelectorAll("input[data-from]").forEach(function (i) { i.value = from.value; });
  }

  // Plaid Link. The sheet closes first (a modal sheet would sit above
  // Plaid's window); the enroll form moves out of it so the result can
  // still be posted, and the sheet reopens for the next step.
  document.addEventListener("click", function (e) {
    var b = e.target.closest && e.target.closest("[data-plaid-connect]");
    if (!b) return;
    if (!navigator.onLine) { toast("error", t("online_only")); return; }
    var form = byId("plaid-enroll");
    if (!form) return;
    copyFrom(form);
    document.body.appendChild(form);
    closeSheet();
    var start = function () {
      try {
        window.Plaid.create({
          token: b.dataset.token,
          onSuccess: function (publicToken, meta) {
            var set = function (k, v) { var el = form.querySelector('[name="' + k + '"]'); if (el) el.value = v || ""; };
            set("public_token", publicToken);
            set("institution", meta && meta.institution && meta.institution.name);
            openSheet();
            form.requestSubmit();
            setTimeout(function () { if (form.parentNode === document.body) form.remove(); }, 8000);
          },
          onExit: function () { if (form.parentNode === document.body) form.remove(); }
        }).open();
      } catch (_) { toast("error", t("provider_failed")); }
    };
    if (window.Plaid) { start(); return; }
    var s = document.createElement("script");
    s.src = b.dataset.src; s.onload = start;
    s.onerror = function () { toast("error", t("provider_failed")); form.remove(); };
    document.head.appendChild(s);
  });

  var contentQueued = false;
  function onContent() {
    if (contentQueued) return;
    contentQueued = true;
    requestAnimationFrame(function () { contentQueued = false; syncTheme(); syncBadges(); });
  }

  // New transactions default to the account used last on this device.
  function rememberAccount(root) {
    var saved = null; try { saved = localStorage.getItem("pz-last-account"); } catch (_) {}
    if (!saved) return;
    (root || document).querySelectorAll("select[data-remember-account]").forEach(function (s) {
      if (s.dataset.remembered) return;
      s.dataset.remembered = "1";
      if (Array.prototype.some.call(s.options, function (o) { return o.value === saved; })) s.value = saved;
    });
  }
  document.addEventListener("submit", function (e) {
    var s = e.target && e.target.querySelector && e.target.querySelector("select[data-remember-account]");
    if (s) { try { if (s.value) localStorage.setItem("pz-last-account", s.value); else localStorage.removeItem("pz-last-account"); } catch (_) {} }
  }, true);

  document.addEventListener("DOMContentLoaded", function () {
    onContent();
    var sheetEl = byId("sheet");
    if (sheetEl && window.MutationObserver) new MutationObserver(function () { rememberAccount(sheetEl); }).observe(sheetEl, { childList: true, subtree: true });
    var main = byId("content");
    if (main && window.MutationObserver) new MutationObserver(onContent).observe(main, { childList: true, subtree: true, attributes: true, attributeFilter: ["aria-busy", "data-needs-line"] });
    // Drop the one-time sign-in marker from the address bar.
    if (/[?&]signed_in=1/.test(location.search)) {
      var u = new URL(location.href); u.searchParams.delete("signed_in");
      history.replaceState(null, "", u.pathname + (u.search || "") + u.hash);
    }
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
      expense_line_id: text("part_line_0") || text("expense_line_id"), paycheck_id: text("part_paycheck_0") || text("paycheck_id"),
      account_id: text("account_id")
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
    var dlg = form.closest("dialog"); if (dlg && dlg.id === "sheet") closeSheet(); else if (dlg && dlg.open) dlg.close();
    if (op.kind === "create_transaction") {
      var list = byId("tx-list");
      if (!list) {
        list = document.createElement("ul"); list.id = "tx-list"; list.className = "tx-list card";
        var empty = document.querySelector("section[aria-labelledby=tx-list-h] .empty");
        var host = document.querySelector("section[aria-labelledby=tx-list-h]");
        if (empty) empty.replaceWith(list); else if (host) host.appendChild(list); else document.body.appendChild(list);
      }
      var li = document.createElement("li");
      li.className = "tx pending";
      li.innerHTML = '<div class="tx-open"><span class="tx-main"><span class="tx-payee">' + escapeHtml(op.tx.payee || "") + " " + badge +
        '</span><span class="tx-meta">' + escapeHtml(op.tx.date) + '</span></span><span class="tx-amt num">' + escapeHtml(fmt(op.tx.amount)) + "</span></div>";
      list.prepend(li);
      form.reset();
    } else if (op.kind === "update_transaction" || op.kind === "delete_transaction") {
      var row = form.dataset.tx ? document.querySelector('#tx-list li.tx[data-tx="' + form.dataset.tx + '"]') : null;
      if (row) { row.classList.add("pending"); var p = row.querySelector(".tx-payee"); if (p) p.insertAdjacentHTML("beforeend", " " + badge); }
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
    if (navigator.onLine && form instanceof HTMLFormElement && form.dataset.offline && !form.hasAttribute("data-on:submit__prevent")) {
      // Built offline, but the connection is back: reopen the real sheet.
      e.preventDefault(); e.stopImmediatePropagation(); closeSheet(); toast("warning", t("back_online")); return;
    }
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
