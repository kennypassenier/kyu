// kyu: topics-util.js, small helpers of the Topics page: selectors, the element builder, number formats and the JSON reader.

/** @param {string} selector @param {ParentNode} [root] */
export const $ = (selector, root = document) => /** @type {HTMLElement | null} */ (root.querySelector(selector));
/** @param {string} selector @param {ParentNode} [root] */
export const $$ = (selector, root = document) => /** @type {HTMLElement[]} */ ([...root.querySelectorAll(selector)]);

/**
 * @param {string} tag @param {Record<string, string | boolean | undefined>} [attrs] @param {(Node | string)[]} [kids]
 * @returns {HTMLElement}
 */
export function h(tag, attrs = {}, ...kids) {
    const el = document.createElement(tag);
    for (const [name, value] of Object.entries(attrs)) {
        if (value === undefined || value === false) continue;
        el.setAttribute(name, value === true ? '' : value);
    }
    el.append(...kids);
    return el;
}

export const number = (/** @type {number} */ n) => n.toLocaleString('en');
export const plural = (/** @type {number} */ n, /** @type {string} */ one, /** @type {string} */ many) => `${number(n)} ${n === 1 ? one : many}`;

/**
 * Read a JSON route. A route that is not there yet (404) or fails is a failure the component shows in its own words.
 * @param {string} url
 */
export async function readJson(url) {
    const response = await fetch(url, { headers: { accept: 'application/json' } });
    if (!response.ok) throw new Error(`${url} answered ${response.status}`);
    return response.json();
}
