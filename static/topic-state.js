// kyu: topic-state.js, the Topic page's shared data: the page element, the JSON block, the
// mutable state the modules share (one object, because an imported binding cannot be assigned
// from another module), and the JSON call to the dashboard.

import { $ } from './topic-util.js';

export const page = /** @type {HTMLElement} */ ($('[data-kt-page]'));
export const TOPIC = page.dataset.ktTopic ?? '';
export const ORIGIN = location.origin;
export const T = `/t/${encodeURIComponent(TOPIC)}/dashboard`;

/** Everything the modules share and reassign. */
export const S = {
    /** The page's JSON block, replaced on each read. @type {any} */
    data: JSON.parse($('#kyu-topic-data')?.textContent ?? '{}'),
    /** Edits waiting for their undo window: a refresh in between would undo them on screen. */
    pending: 0,
    /** True once the kit's boot has run, so rows built later are attached by hand. */
    booted: false,
    /** The kit's attachAll, set at boot. @type {((root: ParentNode) => unknown) | null} */
    attachAll: null,
    /** @type {ReturnType<typeof import('/static/kp/js/datatable.js').dataTable>} */
    subsTable: null,
    /** @type {ReturnType<typeof import('/static/kp/js/datatable.js').dataTable>} */
    deadTable: null,
    /** Days dead letters are kept, from the retention route; null while it does not answer. @type {number | null} */
    retentionDays: null,
};
/** The subscriptions on the page by name. @type {Map<string, any>} */
export const SUBS = new Map();
/** The recent messages on the page by id. @type {Map<string, any>} */
export const MESSAGES = new Map();

/**
 * Count one edit as waiting for its undo window; the returned function releases it, once.
 * @returns {() => void}
 */
export function holdPending() {
    S.pending += 1;
    let settled = false;
    return () => {
        if (!settled) S.pending -= 1;
        settled = true;
    };
}

/** @param {string} text */
export const withOrigin = (text) => text.split(S.data.base).join(ORIGIN);

/** A JSON call to the dashboard. Never throws: a refusal comes back as `{ ok: false, error, remedy }`.
 * @param {string} path @param {{ form?: Record<string, string>, json?: unknown }} [body] */
export async function call(path, body = {}) {
    const headers = /** @type {Record<string, string>} */ ({ Accept: 'application/json' });
    let payload;
    if (body.json !== undefined) {
        headers['Content-Type'] = 'application/json';
        payload = JSON.stringify(body.json);
    } else if (body.form) {
        headers['Content-Type'] = 'application/x-www-form-urlencoded';
        payload = new URLSearchParams(body.form).toString();
    }
    try {
        const res = await fetch(path, { method: 'POST', headers, body: payload, credentials: 'same-origin' });
        /** @type {any} */
        let json = null;
        try {
            json = await res.json();
        } catch {
            json = null;
        }
        if (res.ok) return { ok: true, json };
        if (json && json.error) return { ok: false, status: res.status, error: String(json.error), remedy: String(json.remedy ?? '') };
        if (res.status === 404 || res.status === 405)
            return { ok: false, missing: true, status: res.status, error: 'This hub does not offer that action yet.', remedy: 'Update kyu, then try again.' };
        return { ok: false, status: res.status, error: `The hub answered ${res.status}.`, remedy: 'Try again in a moment.' };
    } catch {
        return { ok: false, status: 0, error: 'kyu did not answer.', remedy: 'Check that the hub is running, then try again.' };
    }
}
export const refusal = (/** @type {{ error: string, remedy: string }} */ r) => ({ error: r.error, remedy: r.remedy });
export const sentence = (/** @type {{ error: string, remedy: string }} */ r) => [r.error, r.remedy].filter(Boolean).join(' ');
