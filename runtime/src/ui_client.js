// Flux frontend — universal client runtime (PR-5a, server-driven, stateless).
//
// Falsafa (FRONTEND-PROD-ARCHITECTURE): client THIN. Interaktiv island'da event
// (data-fx-on) bo'lganda, client island state'ini server'ga POST qiladi, server
// o'sha island'ni qayta render qiladi (Rust interp) va yangi HTML qaytaradi,
// client island DOM'ini almashtiradi. STATELESS: server RAM'da state saqlamaydi
// — client har event'da state yuboradi. WS yo'q (PR-7 real-time uchun qo'shadi).
//
// Bu fayl runtime'ga include_str! bilan kiritiladi va /_fx/client.js da beriladi.
// Faqat island bor sahifalarga yuklanadi (window.__fx mavjud bo'lsa).
(function () {
  "use strict";
  if (!window.__fx) return;

  // Island element'ining joriy state'ini yig'adi (stateless — server shu state
  // bilan re-render qiladi). Ikki manba:
  //  1) data-fx-state (island ildizidagi JSON) — DOM input bo'lmagan reaktiv
  //     state (masalan `count`). Server oxirgi event javobida yozadi (PR-6).
  //  2) data-fx-bind input qiymatlari — two-way input'lar (jonli, DOM'dan).
  // Input qiymatlari data-fx-state ustiga yoziladi (input = eng yangi haqiqat).
  function islandState(islandEl) {
    var state = {};
    var raw = islandEl.getAttribute("data-fx-state");
    if (raw) {
      try {
        var parsed = JSON.parse(raw);
        for (var k in parsed) {
          if (Object.prototype.hasOwnProperty.call(parsed, k)) state[k] = parsed[k];
        }
      } catch (e) {
        console.error("flux: data-fx-state parse xato", e);
      }
    }
    var bound = islandEl.querySelectorAll("[data-fx-bind]");
    for (var i = 0; i < bound.length; i++) {
      var el = bound[i];
      var name = el.getAttribute("data-fx-bind");
      // input/select/textarea -> value; checkbox -> checked.
      if (el.type === "checkbox") state[name] = el.checked;
      else state[name] = el.value;
    }
    return state;
  }

  // Eng yaqin o'rab turuvchi island element'ini topadi.
  function closestIsland(el) {
    while (el && el !== document.body) {
      if (el.hasAttribute && el.hasAttribute("data-fx-island")) return el;
      el = el.parentElement;
    }
    return null;
  }

  // Server'ga event yuboradi va island DOM'ini yangilaydi.
  function dispatch(islandEl, event, handler) {
    var id = islandEl.getAttribute("data-fx-island");
    var payload = {
      island: id,
      event: event,
      handler: handler,
      state: islandState(islandEl),
    };
    fetch("/_fx/event", {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(payload),
    })
      .then(function (r) {
        return r.text();
      })
      .then(function (html) {
        // Server yangi island HTML qaytaradi -> o'rnini almashtiramiz.
        // outerHTML almashtirgach yangi element listenerlarni avtomat oladi
        // (event delegation document darajasida — pastga qarang).
        if (html) islandEl.outerHTML = html;
      })
      .catch(function (e) {
        console.error("flux: event xato", e);
      });
  }

  // Event delegation: bitta global listener (Qwikloader naqshi). data-fx-on
  // bo'lgan element click qilinsa, eng yaqin island'ni topib server'ga yuboradi.
  document.addEventListener("click", function (ev) {
    var el = ev.target;
    while (el && el !== document.body) {
      if (el.hasAttribute && el.hasAttribute("data-fx-on")) {
        var marker = el.getAttribute("data-fx-on"); // "event:handler"
        var parts = marker.split(":");
        var evName = parts[0] || "click";
        var handler = parts[1] || "_";
        if (evName === "click") {
          var island = closestIsland(el);
          if (island) {
            ev.preventDefault();
            dispatch(island, "click", handler);
          }
        }
        return;
      }
      el = el.parentElement;
    }
  });

  // bind: input o'zgarganda — hozir (PR-5a) faqat lokal qiymat saqlanadi
  // (islandState keyingi event'da yuboradi). Jonli filtr/qidiruv server-driven:
  // input event island'ni qayta render qiladi (debounce keyingi optimizatsiya).
  document.addEventListener("input", function (ev) {
    var el = ev.target;
    if (el.hasAttribute && el.hasAttribute("data-fx-bind")) {
      var island = closestIsland(el);
      if (island) dispatch(island, "input", el.getAttribute("data-fx-bind"));
    }
  });
})();
