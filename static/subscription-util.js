// kyu: subscription-util.js, small helpers of the Subscription page: selectors, the boot hook, JSON reads and duration words.

import { freshnessExports } from '/static/kp/dist/kp-themes.min.js';
const { agoMoment } = freshnessExports;

/** @param {string} sel @param {ParentNode} [root] @returns {HTMLElement | null} */
export const $ = (sel, root = document) => /** @type {HTMLElement | null} */ (root.querySelector(sel));
/** @param {string} sel @param {ParentNode} [root] @returns {HTMLElement[]} */
export const $$ = (sel, root = document) => /** @type {HTMLElement[]} */ ([...root.querySelectorAll(sel)]);

/** Run once the package's own boot (js/auto.js) has attached everything the page carries. @param {() => void} run */
export function onReady(run) {
    const root = document.documentElement;
    if (root.hasAttribute('data-kp-auto-ready')) return run();
    new MutationObserver((_, obs) => {
        if (!root.hasAttribute('data-kp-auto-ready')) return;
        obs.disconnect();
        run();
    }).observe(root, { attributes: true });
}

/** @param {string} url @param {RequestInit} [init] */
export async function getJson(url, init) {
    const response = await fetch(url, { credentials: 'same-origin', headers: { Accept: 'application/json', ...(init?.headers ?? {}) }, ...init });
    if (!response.ok) {
        /** @type {any} */
        let body = null;
        try {
            body = await response.json();
        } catch {
            /* an answer that is not JSON has no remedy to show */
        }
        throw new Error(body?.error ?? `kyu answered ${response.status}`);
    }
    return response.json();
}

/** A duration in words. @param {number} seconds */
export function words(seconds) {
    const s = Math.round(seconds);
    if (s < 60) return `${s} s`;
    if (s < 3600) return `${Math.floor(s / 60)} min`;
    if (s < 86_400) return `${Math.floor(s / 3600)} h`;
    const d = Math.floor(s / 86_400);
    return `${d} ${d === 1 ? 'day' : 'days'}`;
}

/** dd/mm/yyyy HH:mm:ss in Europe/Brussels, for the lane names. @param {number} ms */
export const withSeconds = (ms) => `${agoMoment(ms)}:${String(Math.floor(ms / 1000) % 60).padStart(2, '0')}`;
