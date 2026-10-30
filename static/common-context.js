// kyu: common-context.js, where the page is (topic page, Topics page, which topic) and the small DOM helpers the shared parts use.

export const TOUR_NAME = 'kyu-topic-page';
export const PATH = location.pathname.replace(/\/+$/, '');
export const TOPIC_MATCH = /^\/t\/([^/]+)\/dashboard(?:\/|$)/.exec(PATH);
export const CONTEXT_TOPIC = TOPIC_MATCH ? decodeURIComponent(TOPIC_MATCH[1]) : '';
export const ON_TOPIC_PAGE = /^\/t\/[^/]+\/dashboard$/.test(PATH);
export const ON_TOPICS_PAGE = PATH === '/topics';

/** @param {string} selector @param {ParentNode} [root] @returns {HTMLElement[]} */
export const all = (selector, root = document) => /** @type {HTMLElement[]} */ ([...root.querySelectorAll(selector)]);
/** @param {string} selector @param {ParentNode} [root] @returns {HTMLElement | null} */
export const one = (selector, root = document) => /** @type {HTMLElement | null} */ (root.querySelector(selector));

/** Plural of a count: "1 dead letter", "3 dead letters". */
export const counted = (n, noun) => `${n} ${noun}${n === 1 ? '' : 's'}`;

export const main = one('main');
