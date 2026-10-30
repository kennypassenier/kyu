// kyu: topic-util.js, small pure helpers of the Topic page: selectors, escaping, number and time formats.
// Owns no state and touches no page element beyond what a caller passes in.

/** @param {string} sel @param {ParentNode} [root] @returns {HTMLElement | null} */
export const $ = (sel, root = document) => /** @type {HTMLElement | null} */ (root.querySelector(sel));
/** @param {string} sel @param {ParentNode} [root] @returns {HTMLElement[]} */
export const $$ = (sel, root = document) => /** @type {HTMLElement[]} */ ([...root.querySelectorAll(sel)]);
/** @param {string} tag @param {string} [className] @param {string} [text] */
export function make(tag, className = '', text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
}
/** @param {unknown} value */
export const esc = (value) =>
    String(value).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c] ?? c);
/** The exact number with its thousands grouped. @param {number} n */
export const num = (n) => n.toLocaleString('en-GB');
export const plural = (/** @type {number} */ n, /** @type {string} */ one, /** @type {string} */ many) => `${num(n)} ${n === 1 ? one : many}`;

const ZONE = 'Europe/Brussels';
const MOMENT = new Intl.DateTimeFormat('en-GB', {
    timeZone: ZONE,
    day: '2-digit',
    month: '2-digit',
    year: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
});
/** dd/mm/yyyy HH:mm in Europe/Brussels. @param {number} ms */
export const dmy = (ms) => MOMENT.format(new Date(ms)).replace(', ', ' ');
export const iso = (/** @type {number} */ ms) => new Date(ms).toISOString();

/** 30000 -> "30 s", 120000 -> "2 min", 604800000 -> "7 days". @param {number} ms */
export function span(ms) {
    const s = Math.round(ms / 1000);
    if (s < 60) return `${s} s`;
    if (s < 3600) return `${Math.round(s / 60)} min`;
    if (s < 86400) return `${Math.round(s / 3600)} h`;
    const d = Math.round(s / 86400);
    return d === 1 ? '1 day' : `${d} days`;
}
/** @param {number} bytes */
export function bytesText(bytes) {
    if (bytes < 1000) return `${bytes} B`;
    if (bytes < 1_000_000) return `${(bytes / 1000).toFixed(1).replace('.0', '')} kB`;
    return `${(bytes / 1_000_000).toFixed(1).replace('.0', '')} MB`;
}
