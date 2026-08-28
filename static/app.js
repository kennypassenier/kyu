// mailbox dashboard — reveal and copy (W2).
//
// Everything else on this dashboard is server-rendered HTML and plain
// forms. This file exists for exactly two controls, which is why htmx was
// dropped rather than actually shipped (T4 amendment).
//
// The one thing worth knowing before editing: navigator.clipboard is
// undefined here. It is gated behind a secure context, and the hub is plain
// HTTP on a LAN, so the deprecated execCommand path is not laziness — it is
// the only one that works. It is fed from a textarea placed off-screen
// rather than hidden, because a display:none element cannot be selected.

(function () {
  "use strict";

  function copyText(text) {
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
      flash(button, copyText(text) ? "Copied" : "Press Ctrl+C");
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
