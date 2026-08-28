// mailbox dashboard — reveal and copy (W2).
//
// Everything else on this dashboard is server-rendered HTML and plain
// forms. This file exists for exactly two controls, which is why htmx was
// dropped rather than actually shipped (T4 amendment).
//
// Copying needs both paths, and which one runs depends on the URL you use:
//
// - navigator.clipboard exists only in a "secure context". https qualifies,
//   and so does http://localhost — so it IS available when you open the hub
//   on the machine it runs on, and is NOT when you open it at
//   http://mailbox.lan:8080 from your laptop. Both are normal ways to use
//   this dashboard, so both paths have to be here.
// - The execCommand fallback works on plain http, but only inside a real
//   user gesture. That is fine for a button someone clicks, and it is why an
//   automated click in a console reports failure while a real one succeeds.
//
// The scratch textarea is positioned off-screen rather than hidden, because
// a display:none element cannot be selected.

(function () {
  "use strict";

  function copyLegacy(text) {
    var scratch = document.createElement("textarea");
    scratch.value = text;
    scratch.setAttribute("readonly", "");
    scratch.style.position = "fixed";
    scratch.style.left = "-9999px";
    document.body.appendChild(scratch);
    scratch.select();
    var copied = false;
    try {
      copied = document.execCommand("copy");
    } catch (error) {
      copied = false;
    }
    document.body.removeChild(scratch);
    return copied;
  }

  // Reports success through a callback rather than a return value, because
  // the modern path is asynchronous and the legacy one is not.
  function copyText(text, done) {
    if (navigator.clipboard && navigator.clipboard.writeText) {
      navigator.clipboard.writeText(text).then(
        function () {
          done(true);
        },
        function () {
          // Permission refused or no gesture: the old way may still work.
          done(copyLegacy(text));
        }
      );
      return;
    }
    done(copyLegacy(text));
  }

  function flash(button, message) {
    var original = button.dataset.label || button.textContent;
    button.dataset.label = original;
    button.textContent = message;
    window.setTimeout(function () {
      button.textContent = button.dataset.label;
    }, 1500);
  }

  document.addEventListener("click", function (event) {
    var button = event.target.closest("[data-copy-target]");
    if (button) {
      var source = document.getElementById(button.dataset.copyTarget);
      // The secret lives in a data attribute, never in the visible text, so
      // copying works while the value is still masked on screen.
      var text = source ? source.dataset.secret || source.textContent : "";
      copyText(text, function (copied) {
        // The honest failure message. Telling someone to press Ctrl+C when
        // the value on screen is masked would have them copying bullets.
        flash(button, copied ? "Copied" : "Copy failed — use Reveal");
      });
      return;
    }

    var toggle = event.target.closest("[data-reveal-target]");
    if (!toggle) {
      return;
    }
    var target = document.getElementById(toggle.dataset.revealTarget);
    if (!target) {
      return;
    }

    var seconds = parseInt(toggle.dataset.revealSeconds, 10) || 10;
    var masked = target.dataset.masked;
    var secret = target.dataset.secret;

    if (target.dataset.revealed === "true") {
      hide(target, toggle, masked);
      return;
    }

    target.textContent = secret;
    target.dataset.revealed = "true";
    toggle.textContent = "Hide";
    // Re-arm on every reveal, so a second click never leaves a stale timer
    // hiding a freshly revealed value.
    window.clearTimeout(target.revealTimer);
    target.revealTimer = window.setTimeout(function () {
      hide(target, toggle, masked);
    }, seconds * 1000);
  });

  function hide(target, toggle, masked) {
    window.clearTimeout(target.revealTimer);
    target.textContent = masked;
    target.dataset.revealed = "false";
    toggle.textContent = "Reveal";
  }
})();
