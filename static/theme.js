// kyu — the theme picker (kp-themes v0.1.1).
//
// A behaviour-only port of @kp-soft/themes' React ThemeSwitcher: kyu's
// dashboard is server-rendered HTML with no build step, so the hook and the
// JSX component cannot be used. What MUST match, and does, is the contract
// a visitor can feel:
//
//   - the choice is stored in localStorage under the key "theme"
//   - it is applied as data-theme on <html>, plus the class "dark"
//   - the default is "formal"
//
// Deliberately no theme list here. The seven themes live once, in Rust
// (dashboard.rs THEMES), and are rendered into the markup this script reads:
// every option carries data-theme and data-dark. A copy of that list in
// JavaScript would be a second source of truth, and the version that goes
// stale is always the one nobody is looking at.

(function () {
    'use strict';

    var STORAGE_KEY = 'theme';
    var DEFAULT_THEME = 'formal';

    /** Every theme the server rendered, with whether it is a dark one. */
    function options() {
        return Array.prototype.slice.call(
            document.querySelectorAll('[data-theme-picker] [data-theme]')
        );
    }

    function isDark(name) {
        var found = options().filter(function (option) {
            return option.dataset.theme === name;
        })[0];
        return found ? found.dataset.dark === 'true' : false;
    }

    function known(name) {
        return options().some(function (option) {
            return option.dataset.theme === name;
        });
    }

    function apply(name) {
        var root = document.documentElement;
        root.dataset.theme = name;
        root.classList.toggle('dark', isDark(name));
        // Bootstrap styles its own components off this attribute; without it
        // the tokens go dark while the components stay light.
        root.setAttribute('data-bs-theme', isDark(name) ? 'dark' : 'light');
        options().forEach(function (option) {
            option.setAttribute('aria-selected', String(option.dataset.theme === name));
        });
    }

    function stored() {
        try {
            return localStorage.getItem(STORAGE_KEY);
        } catch (error) {
            // Private mode or blocked storage: the DOM still carries a theme.
            return null;
        }
    }

    function store(name) {
        try {
            localStorage.setItem(STORAGE_KEY, name);
        } catch (error) {
            /* nothing to do; the choice simply will not outlive the tab */
        }
    }

    function ready() {
        var picker = document.querySelector('[data-theme-picker]');
        if (!picker) return;

        var trigger = picker.querySelector('[data-theme-trigger]');
        var list = picker.querySelector('[data-theme-list]');
        if (!trigger || !list) return;

        // The <head> script already applied the stored theme so the page
        // never flashes; this re-applies it now that the options exist, which
        // is what marks the active one and settles data-bs-theme.
        var initial = stored();
        apply(known(initial) ? initial : DEFAULT_THEME);

        function close() {
            list.hidden = true;
            trigger.setAttribute('aria-expanded', 'false');
        }

        function open() {
            list.hidden = false;
            trigger.setAttribute('aria-expanded', 'true');
        }

        trigger.addEventListener('click', function () {
            if (list.hidden) open();
            else close();
        });

        function choose(option) {
            var name = option.dataset.theme;
            apply(name);
            store(name);
            close();
            trigger.focus();
        }

        options().forEach(function (option) {
            option.addEventListener('click', function () {
                choose(option);
            });
            option.addEventListener('keydown', function (event) {
                if (event.key === 'Enter' || event.key === ' ') {
                    event.preventDefault();
                    choose(option);
                }
            });
        });

        document.addEventListener('mousedown', function (event) {
            if (!list.hidden && !picker.contains(event.target)) close();
        });
        document.addEventListener('keydown', function (event) {
            if (event.key === 'Escape' && !list.hidden) {
                close();
                trigger.focus();
            }
        });
    }

    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', ready);
    } else {
        ready();
    }
})();
