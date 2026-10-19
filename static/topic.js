// kyu: topic.js, the Topic page (/t/{topic}/dashboard) on kp-themes 11.0.0.
//
// The server renders the page's frame and one JSON block (#kyu-topic-data) with
// everything the regions show; this file draws the regions from it (the
// subscriptions, the dead letters with their problems, the recent messages, the
// commands), answers the actions with the dashboard's JSON routes, and reads
// the page again on each automatic refresh. Every control is a kp-themes
// component; nothing here draws a control of its own.
//
// Order matters once: the rows are built first, then the kit's boot
// (/static/kp/js/auto.js) is imported, so it attaches the datatable, the menus,
// the code blocks and the rest to markup that already exists.

import { setStep } from '/static/kp/js/steplist.js';
import { setCardState } from '/static/kp/js/card-state.js';
import { setProblems } from '/static/kp/js/problems.js';
import { setMenu } from '/static/kp/js/menu-button.js';
import { dataTable } from '/static/kp/js/datatable.js';
import { undoToast } from '/static/kp/js/undo-toast.js';
import { toast } from '/static/kp/js/overlays.js';
import { setCode } from '/static/kp/js/code.js';
import { masterDetail, setPaneState } from '/static/kp/js/master-detail.js';
import { openViewer } from '/static/kp/js/viewer.js';
import { runFlashButton } from '/static/kp/js/flash.js';
import { setSelectionBusy, setSelectionResult } from '/static/kp/js/selection-bar.js';
import { statePlate } from '/static/kp/js/state-plate.js';
import { openBusyCard } from '/static/kp/js/busy-card.js';
import { attachArt } from '/static/kp/js/art.js';
import { setSegmentedCounts } from '/static/kp/js/segmented.js';
import { setAgo } from '/static/kp/js/freshness.js';

/** @param {string} sel @param {ParentNode} [root] @returns {HTMLElement | null} */
const $ = (sel, root = document) => /** @type {HTMLElement | null} */ (root.querySelector(sel));
/** @param {string} sel @param {ParentNode} [root] @returns {HTMLElement[]} */
const $$ = (sel, root = document) => /** @type {HTMLElement[]} */ ([...root.querySelectorAll(sel)]);
/** @param {string} tag @param {string} [className] @param {string} [text] */
function make(tag, className = '', text) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
}
/** @param {unknown} value */
const esc = (value) =>
    String(value).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c] ?? c);
/** The exact number with its thousands grouped. @param {number} n */
const num = (n) => n.toLocaleString('en-GB');
const plural = (/** @type {number} */ n, /** @type {string} */ one, /** @type {string} */ many) => `${num(n)} ${n === 1 ? one : many}`;

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
const dmy = (ms) => MOMENT.format(new Date(ms)).replace(', ', ' ');
const iso = (/** @type {number} */ ms) => new Date(ms).toISOString();

/** 30000 -> "30 s", 120000 -> "2 min", 604800000 -> "7 days". @param {number} ms */
function span(ms) {
    const s = Math.round(ms / 1000);
    if (s < 60) return `${s} s`;
    if (s < 3600) return `${Math.round(s / 60)} min`;
    if (s < 86400) return `${Math.round(s / 3600)} h`;
    const d = Math.round(s / 86400);
    return d === 1 ? '1 day' : `${d} days`;
}
/** @param {number} bytes */
function bytesText(bytes) {
    if (bytes < 1000) return `${bytes} B`;
    if (bytes < 1_000_000) return `${(bytes / 1000).toFixed(1).replace('.0', '')} kB`;
    return `${(bytes / 1_000_000).toFixed(1).replace('.0', '')} MB`;
}

// ---------------------------------------------------------------- the data

const page = /** @type {HTMLElement} */ ($('[data-kt-page]'));
const TOPIC = page.dataset.ktTopic ?? '';
const ORIGIN = location.origin;
const T = `/t/${encodeURIComponent(TOPIC)}/dashboard`;
/** @type {any} */
let DATA = JSON.parse($('#kyu-topic-data')?.textContent ?? '{}');
/** Edits waiting for their undo window: a refresh in between would undo them on screen. */
let pending = 0;

/** @param {string} text */
const withOrigin = (text) => text.split(DATA.base).join(ORIGIN);

/** A JSON call to the dashboard. Never throws: a refusal comes back as `{ ok: false, error, remedy }`.
 * @param {string} path @param {{ form?: Record<string, string>, json?: unknown }} [body] */
async function call(path, body = {}) {
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
            return { ok: false, status: res.status, error: 'This hub does not offer that action yet.', remedy: 'Update kyu, then try again.' };
        return { ok: false, status: res.status, error: `The hub answered ${res.status}.`, remedy: 'Try again in a moment.' };
    } catch {
        return { ok: false, status: 0, error: 'kyu did not answer.', remedy: 'Check that the hub is running, then try again.' };
    }
}
const refusal = (/** @type {{ error: string, remedy: string }} */ r) => ({ error: r.error, remedy: r.remedy });
const sentence = (/** @type {{ error: string, remedy: string }} */ r) => [r.error, r.remedy].filter(Boolean).join(' ');

// ---------------------------------------------------------------- a region that keeps its footprint in every state

/** @param {HTMLElement} host a `[data-kt-stage]` */
function stage(host) {
    const plate = /** @type {HTMLElement} */ ($('[data-kt-plate]', host));
    /** @type {import('/static/kp/js/busy-card.js').BusyCard | null} */
    let card = null;
    const clear = () => {
        card?.remove();
        card = null;
        plate.hidden = true;
        plate.replaceChildren();
        plate.removeAttribute('data-kp-art');
        host.removeAttribute('data-kt-state');
    };
    return {
        ready: clear,
        /** @param {string} words */
        loading(words) {
            clear();
            host.setAttribute('data-kt-state', 'loading');
            card = openBusyCard(host, { words, area: () => [host], arrive: false });
        },
        /** @param {HTMLElement} node @param {string} [art] */
        plate(node, art) {
            clear();
            host.setAttribute('data-kt-state', 'plate');
            if (art) {
                node.setAttribute('data-kp-art', art);
                attachArt(node);
            }
            plate.append(node);
            plate.hidden = false;
        },
    };
}

// ---------------------------------------------------------------- 1 · the header

const header = /** @type {HTMLElement} */ ($('[data-kt-header]'));
function renderHeader() {
    const t = DATA.topic;
    const badge = $('[data-kt-dead-badge]', header);
    if (badge) {
        badge.hidden = !(t.dead > 0);
        badge.textContent = `${num(t.dead)} dead`;
    }
    const facts = $('[data-kt-facts]', header);
    if (facts) facts.textContent = `${num(t.messages)} messages retained · backlog ${num(t.backlog)} · ${plural(t.subscriptions, 'subscription', 'subscriptions')}`;
    const last = $('[data-kt-last]', header);
    if (last) {
        last.hidden = !t.last_published_ms;
        if (t.last_published_ms) setAgo(last, t.last_published_ms);
    }
    const url = $('[data-kt-topic-url]', header);
    if (url) url.textContent = `${ORIGIN}/t/${TOPIC}`;
}

// ---------------------------------------------------------------- 2 · send and receive

const send = /** @type {HTMLElement} */ ($('[data-kt-send]'));
const ttl = Number(page.dataset.ktRevealSeconds ?? 10);
function renderCommands() {
    for (const block of $$('[data-kt-command]', send)) {
        const code = $('code', block);
        if (code) code.textContent = withOrigin(DATA.commands.shown[block.dataset.ktCommand ?? ''] ?? '');
    }
    for (const field of $$('[data-kt-secret]', send)) field.setAttribute('data-kp-secret-ttl', String(ttl));
}
// The token is read from the page's data only when a field is asked to show or copy it.
send.addEventListener('kp-secret-request', (event) => {
    const { respond } = /** @type {CustomEvent} */ (event).detail;
    const field = /** @type {HTMLElement} */ (event.target);
    event.preventDefault();
    respond(withOrigin(DATA.commands.working[field.dataset.ktSecret ?? ''] ?? ''));
});
const journey = /** @type {HTMLElement} */ ($('[data-kt-journey]', send));
const STEPS = ['publish', 'receive', 'ack'];
const startOver = () => STEPS.forEach((id, i) => setStep(journey, id, { state: i === 0 ? 'current' : 'pending', time: '' }));
send.addEventListener('kp-copy', (event) => {
    const step = /** @type {HTMLElement} */ (event.target).closest('[data-kt-step]')?.getAttribute('data-kt-step');
    if (!step) return;
    setStep(journey, step, { state: 'done', time: dmy(Date.now()) });
    const next = STEPS.find((id) => $(`[data-kp-step-id="${id}"]`, journey)?.getAttribute('data-state') !== 'done');
    if (next) setStep(journey, next, { state: 'current' });
});
$('[data-kt-start-over]', send)?.addEventListener('click', startOver);
{
    const words = $('[data-kt-ttl-words]', send);
    if (words) words.textContent = ttl === 0 ? 'until you press Hide' : `for ${ttl} seconds`;
}

// ---------------------------------------------------------------- 3 · nobody has polled yet

const readers = $('[data-kt-readers]');
function renderReaders() {
    if (!readers) return;
    const problems = /** @type {HTMLElement} */ ($('[data-kt-bootstrap]', readers));
    const active = DATA.subscriptions.filter((/** @type {any} */ s) => s.state !== 'archived');
    if (active.length > 0) {
        const first = active[0];
        setCardState(readers, 'empty', {
            title: `A reader has polled ${TOPIC}`,
            text: `${first.name} polled ${first.last_poll_at ?? 'recently'}, so there is nothing to warn about.`,
        });
        return;
    }
    const command = withOrigin(DATA.commands.shown.receive_raw).replace('?as=my-consumer"', '?as=my-consumer&from=beginning"');
    setProblems(problems, [
        {
            key: 'nobody-polled',
            what: `Nobody has polled ${TOPIC} yet`,
            why: 'A subscription only starts existing when it first polls, and it receives only what is published after that.',
            remedy: 'Run the receive command once before you publish the message you want it to see, or add &from=beginning to pick up what the topic still retains.',
            command,
            severity: 'warning',
        },
    ]);
    setCardState(readers, 'ready');
}

// ---------------------------------------------------------------- 4 · the subscriptions table

const subsCard = /** @type {HTMLElement} */ ($('[data-kt-subs]'));
const subsBody = /** @type {HTMLElement} */ ($('[data-kt-subs-rows]', subsCard));
const filterGroup = /** @type {HTMLElement} */ ($('[data-kt-filter]', subsCard));
const subsTableEl = /** @type {HTMLElement} */ ($('[data-kt-subs-table]', subsCard));
subsBody.setAttribute('data-kp-idmark-set', '');
/** @type {Map<string, any>} */
const SUBS = new Map();
/** @type {ReturnType<typeof dataTable>} */
let subsTable = null;
let booted = false;
/** @type {((root: ParentNode) => unknown) | null} */
let attachAll = null;

/** @param {any} s */
function policyHtml(s) {
    const lease = `lease ${span(s.lease_ms)}${s.explicit.lease_ms == null ? ' <i>(default)</i>' : ''}`;
    const tries = `${s.max_attempts} attempts${s.explicit.max_attempts == null ? ' <i>(default)</i>' : ''}`;
    const ttlText = s.ttl_ms ? `, ttl ${span(s.ttl_ms)}` : '';
    return `${lease}, ${tries}${ttlText}`;
}
const badge = (/** @type {any} */ s) =>
    `<span class="kp-badge${s.state === 'active' ? ' kp-badge--success' : s.state === 'flagged' ? ' kp-badge--warning' : ''}">${esc(s.state)}</span>`;

/** @param {any} s */
function subRow(s) {
    const tr = document.createElement('tr');
    const n = esc(s.name);
    const oldest = s.oldest_unacked_ms
        ? `<time datetime="${iso(s.oldest_unacked_ms)}">${dmy(s.oldest_unacked_ms)}</time>`
        : '<span class="kt-quiet">nothing waiting</span>';
    tr.dataset.kpKey = s.name;
    tr.dataset.ktSub = s.name;
    tr.innerHTML = `
        <td data-label="Name"><span class="kt-name"><span class="kp-idmark kp-idmark--sm" data-kp-idmark="${n}" aria-hidden="true"></span><a href="${T}/subs/${encodeURIComponent(s.name)}">${n}</a></span></td>
        <td data-label="State" data-kt-state-cell>${badge(s)}</td>
        <td data-label="Backlog" class="kp-text-end">${num(s.backlog)}</td>
        <td data-label="In flight" class="kp-text-end">${num(s.claimed)}</td>
        <td data-label="Dead" class="kp-text-end">${s.dead > 0 ? `<span class="kp-badge kp-badge--destructive">${num(s.dead)}</span>` : '0'}</td>
        <td data-label="Lapsed" class="kp-text-end">${num(s.lapsed)}</td>
        <td data-label="Oldest unacked" data-kp-sort-value="${s.oldest_unacked_ms ? iso(s.oldest_unacked_ms) : ''}">${oldest}</td>
        <td data-label="Policy" class="kt-policy">${policyHtml(s)}</td>
        <td class="kt-col-actions">
            <div class="kp-row-actions">
                <button type="button" class="kp-button kp-button--sm" data-kp-action="archive" data-kt-archive data-kp-disabled-reason="${n} is already archived">Archive</button>
                <button type="button" class="kp-button kp-button--sm kp-button--destructive" data-kp-action="delete" data-kt-delete data-kp-destructive
                    data-kp-confirm="Delete subscription ${n} and its delivery history?" data-kp-confirm-type="${n}"
                    data-kp-confirm-loses="kt-loses-${n}"
                    data-kp-disabled-reason="Archive ${n} first: only an archived subscription can be deleted">Delete</button>
                <div class="kp-menu-button" data-kp-menu-button data-kp-action="more" data-kt-menu="${n}">
                    <button type="button" class="kp-button kp-button--sm" aria-label="More for ${n}">More ▾</button>
                    <div class="kp-menu kp-menu--rich" role="menu" aria-label="More for ${n}"></div>
                </div>
            </div>
        </td>`;
    paintRow(tr, s);
    return tr;
}

/** What deleting would lose, as the typed confirmation lists it. @param {any} s */
function losesTemplate(s) {
    const id = `kt-loses-${s.name}`;
    let t = /** @type {HTMLTemplateElement | null} */ (document.getElementById(id));
    if (!t) {
        t = document.createElement('template');
        t.id = id;
        document.body.append(t);
    }
    const held = s.backlog + s.claimed + s.dead;
    t.innerHTML = `<ul><li>The delivery history of <b>${esc(s.name)}</b> (${plural(held, 'message', 'messages')} still held)</li><li>Its policy: ${policyHtml(s).replace(/<\/?i>/g, '')}</li></ul>`;
}

/** @param {any} s */
function menuGroups(s) {
    const gone = s.state === 'archived';
    const away = `${s.name} is archived and no longer delivers`;
    return [
        {
            group: 'Look',
            items: [
                {
                    label: s.dead > 0 ? `Dead letters (${num(s.dead)})` : 'Dead letters',
                    hint: 'Messages that gave up after too many tries',
                    disabled: s.dead > 0 ? null : `Nothing died on ${s.name}`,
                    value: 'dead',
                },
                {
                    label: 'Oldest unacked message',
                    hint: 'The message that has waited longest',
                    disabled: s.backlog + s.claimed > 0 ? null : `Nothing is waiting on ${s.name}`,
                    value: 'oldest',
                },
            ],
        },
        {
            group: 'Use',
            items: [
                { label: 'Copy receive command', hint: 'Without the token', disabled: gone ? away : null, value: 'copy-receive' },
                { label: 'Copy acknowledge command', hint: 'Without the token', disabled: gone ? away : null, value: 'copy-ack' },
            ],
        },
    ];
}

/** Bring a row's buttons, badge and menu in step with its subscription. @param {HTMLElement} tr @param {any} s */
function paintRow(tr, s) {
    tr.dataset.state = s.state;
    const cell = $('[data-kt-state-cell]', tr);
    if (cell) cell.innerHTML = badge(s);
    const archived = s.state === 'archived';
    const archive = /** @type {HTMLButtonElement} */ ($('[data-kt-archive]', tr));
    const del = /** @type {HTMLButtonElement} */ ($('[data-kt-delete]', tr));
    archive.disabled = archived;
    del.disabled = !archived;
    losesTemplate(s);
    const wrapper = $('[data-kt-menu]', tr);
    if (wrapper) setMenu(wrapper, menuGroups(s));
}

const sigOf = (/** @type {unknown} */ value) => JSON.stringify(value);

/** Keep a table body in step with a list by key: add, replace what changed, remove what left.
 * @param {HTMLElement} body @param {any[]} items @param {(item: any) => string} keyOf @param {(item: any) => HTMLElement} build */
function reconcile(body, items, keyOf, build) {
    const existing = new Map(
        [...body.children].map((row) => [/** @type {HTMLElement} */ (row).dataset.kpKey ?? '', /** @type {HTMLElement} */ (row)]),
    );
    /** @type {HTMLElement[]} */
    const added = [];
    for (const item of items) {
        const key = keyOf(item);
        const row = existing.get(key);
        const sig = sigOf(item);
        if (!row) {
            const fresh = build(item);
            fresh.dataset.ktSig = sig;
            body.append(fresh);
            added.push(fresh);
        } else if (row.dataset.ktSig !== sig) {
            const fresh = build(item);
            fresh.dataset.ktSig = sig;
            row.replaceWith(fresh);
            added.push(fresh);
        }
        existing.delete(key);
    }
    for (const row of existing.values()) row.remove();
    return added;
}

function updateCounts() {
    const list = [...SUBS.values()];
    const by = (/** @type {string} */ state) => list.filter((s) => s.state === state).length;
    setSegmentedCounts(filterGroup, { all: list.length, active: by('active'), flagged: by('flagged'), archived: by('archived') });
    const total = $('[data-kt-sub-total]', subsCard);
    if (total) total.textContent = plural(list.length, 'subscription', 'subscriptions');
}

/** Subscriptions table and filter, from DATA. */
function renderSubs() {
    SUBS.clear();
    for (const s of DATA.subscriptions) SUBS.set(s.name, { ...s });
    const added = reconcile(subsBody, [...SUBS.values()], (s) => s.name, subRow);
    fixTooltips(added);
    updateCounts();
    if (SUBS.size === 0)
        setCardState(subsCard, 'empty', { title: 'Nothing has polled this topic yet', text: `A subscription shows up here the first time something polls ${TOPIC}.` });
    else setCardState(subsCard, 'ready');
    return added;
}

/** The tooltip wrapper of a button that can be off stands in the row's button box: it carries the button's role. @param {HTMLElement[]} rows */
function fixTooltips(rows, attach = true) {
    if (!booted) return;
    for (const row of rows) {
        if (attach) attachAll?.(row);
        for (const anchor of $$('.kp-row-actions > .kp-tooltip-anchor', row)) {
            const role = anchor.querySelector('[data-kp-action]')?.getAttribute('data-kp-action');
            if (role) anchor.setAttribute('data-kp-action', role);
        }
    }
}

filterGroup.addEventListener('kp-segmented-change', (event) => {
    const { values } = /** @type {CustomEvent} */ (event).detail;
    subsTable?.filter(1, values.length ? values : null);
});

/** @param {any} s @param {string} state */
function setSubState(s, state) {
    s.state = state;
    const tr = subsBody.querySelector(`tr[data-kt-sub="${CSS.escape(s.name)}"]`);
    if (tr instanceof HTMLElement) paintRow(tr, s);
    updateCounts();
    subsTable?.refresh();
}

/** Archive with Undo: the page changes at once, the hub hears it when the window closes. @param {any} s */
function archiveWithUndo(s) {
    const before = s.state;
    setSubState(s, 'archived');
    pending += 1;
    let settled = false;
    const settle = () => {
        if (!settled) pending -= 1;
        settled = true;
    };
    undoToast(`${s.name} archived`, {
        ms: 6000,
        clock: 'ring',
        mode: 'defer',
        max: 3,
        commit: async () => {
            const r = await call(`${T}/subs/${encodeURIComponent(s.name)}/archive`);
            settle();
            if (!r.ok) throw new Error(sentence(/** @type {any} */ (r)));
            void load();
        },
        undo: () => {
            settle();
            setSubState(s, before);
        },
        restore: () => {
            settle();
            setSubState(s, before);
        },
    });
}

/** @param {any} s */
async function deleteSub(s) {
    const r = await call(`${T}/subs/${encodeURIComponent(s.name)}/delete`);
    if (!r.ok) {
        toast(`${s.name} was not deleted: ${sentence(/** @type {any} */ (r))}`, { live: 'assertive', sticky: true });
        return;
    }
    subsBody.querySelector(`tr[data-kt-sub="${CSS.escape(s.name)}"]`)?.remove();
    SUBS.delete(s.name);
    updateCounts();
    subsTable?.refresh();
    if (SUBS.size === 0) setCardState(subsCard, 'empty', { title: 'Nothing has polled this topic yet', text: `A subscription shows up here the first time something polls ${TOPIC}.` });
    toast(`${s.name} deleted`, { ms: 4000 });
    void load();
}

subsCard.addEventListener('click', (event) => {
    const target = event.target instanceof Element ? event.target : null;
    const tr = target?.closest('tr[data-kt-sub]');
    const s = SUBS.get(tr?.getAttribute('data-kt-sub') ?? '');
    if (!target || !tr || !s) return;
    if (target.closest('[data-kt-archive]')) archiveWithUndo(s);
    else if (target.closest('[data-kt-delete]')) void deleteSub(s);
});

/** Copy text; say what happened in a toast. @param {string} text @param {string} said */
async function copyToast(text, said) {
    try {
        await navigator.clipboard.writeText(text);
        toast(said, { ms: 3000 });
    } catch {
        toast('Your browser blocked copying. Select the command on the page and press Ctrl+C.', { ms: 5000, live: 'assertive' });
    }
}
document.addEventListener('kp-menu-select', (event) => {
    const { value } = /** @type {CustomEvent} */ (event).detail;
    const name = /** @type {HTMLElement} */ (event.target).closest('[data-kt-menu]')?.getAttribute('data-kt-menu');
    if (!name || !subsCard.contains(/** @type {Node} */ (event.target))) return;
    const base = `curl -s -H "authorization: Bearer $KYU_TOKEN"`;
    if (value === 'dead') {
        deadTable?.filter(2, [name]);
        document.getElementById('kt-dead-title')?.scrollIntoView({ behavior: 'smooth', block: 'start' });
    } else if (value === 'oldest') location.href = `${T}/subs/${encodeURIComponent(name)}`;
    else if (value === 'copy-receive')
        void copyToast(`${base} -D- -o message.body "${ORIGIN}/t/${TOPIC}/next?as=${name}"`, `Copied the receive command for ${name}, without the token.`);
    else if (value === 'copy-ack')
        void copyToast(`${base} -X POST "${ORIGIN}/t/${TOPIC}/ack/<id>?as=${name}"`, `Copied the acknowledge command for ${name}, without the token.`);
});

// ---------------------------------------------------------------- 5 · dead letters

const deadSection = /** @type {HTMLElement} */ ($('[data-kt-dead]'));
const deadBody = /** @type {HTMLElement} */ ($('[data-kt-dead-rows]', deadSection));
const deadTableEl = /** @type {HTMLElement} */ ($('[data-kt-dead-table]', deadSection));
const selectionBar = /** @type {HTMLElement} */ ($('[data-kt-selection]', deadSection));
const problemsHost = /** @type {HTMLElement} */ ($('[data-kt-stage="problems"]', deadSection));
const problemsView = stage(problemsHost);
const problemsList = /** @type {HTMLElement} */ ($('[data-kt-problems]', problemsHost));
/** @type {ReturnType<typeof dataTable>} */
let deadTable = null;
/** Days dead letters are kept, from the retention route; null while it does not answer. @type {number | null} */
let retentionDays = null;

const deadKey = (/** @type {any} */ d) => `dead-${d.subscription}-${d.id}`;
/** Why a message died, in the words kyu has. @param {any} d */
function whyDied(d) {
    const s = SUBS.get(d.subscription);
    const lease = s ? ` (lease ${span(s.lease_ms)})` : '';
    return `${d.subscription} did not acknowledge it in ${plural(d.attempts, 'attempt', 'attempts')}${lease}`;
}
/** A payload as a table cell. @param {any} p */
function cellFigure(p) {
    const fig = make('figure', 'kp-code kp-code--cell');
    fig.setAttribute('data-kp-code', '');
    fig.setAttribute('data-kp-code-language', 'auto');
    fig.setAttribute('data-kp-code-open', 'inline');
    if (p.binary) fig.setAttribute('data-kp-binary-size', String(p.bytes));
    else if (p.truncated) fig.setAttribute('data-kp-truncated', `${new Blob([p.text]).size}/${p.bytes}`);
    const pre = make('pre');
    pre.append(make('code', '', p.binary ? '' : p.text));
    fig.append(pre);
    return fig;
}

/** @param {any} d */
function deadRow(d) {
    const tr = make('tr');
    const key = deadKey(d);
    tr.dataset.kpKey = key;
    tr.setAttribute('data-kp-row-key', key);
    tr.setAttribute('data-kp-anchor', key);
    tr.setAttribute('data-kp-row-label', `Dead letter ${d.id}`);
    tr.dataset.ktSub = d.subscription;
    tr.dataset.ktId = d.id;
    const when = d.dead_ms ?? d.published_ms;
    tr.innerHTML =
        `<td data-label="Select"><input class="kp-field__check" type="checkbox" data-kp-select-row aria-label="Select dead letter ${esc(d.id)}" /></td>` +
        `<td data-label="Died" data-kp-sort-value="${iso(when)}"><div class="kt-when"><time class="kp-ago" data-kp-ago data-kp-ago-verb="" datetime="${iso(when)}">${d.dead_ms ? esc(d.dead_at) : 'unknown'}</time><small>${d.dead_ms ? dmy(d.dead_ms) : `published ${dmy(d.published_ms)}`}</small></div></td>` +
        `<td data-label="Subscription" data-kp-filter-value="${esc(d.subscription)}"><b>${esc(d.subscription)}</b></td>` +
        `<td data-label="Tries" class="kp-text-end">${num(d.attempts)}</td>` +
        `<td data-label="Why it died">${esc(whyDied(d))}</td>` +
        `<td data-label="Payload"></td>` +
        `<td data-label="Actions"><div class="kp-row-actions"><button type="button" class="kp-button kp-button--sm" data-kp-action="requeue" data-kt-row="requeue" data-kp-flash-words="Requeued,Refused,Failed" data-kp-refusal="toast">Requeue</button><button type="button" class="kp-button kp-button--sm kp-button--destructive" data-kp-action="delete" data-kt-row="delete">Delete</button></div></td>`;
    /** @type {HTMLElement} */ ($('td:nth-child(6)', tr)).append(cellFigure(d.payload));
    return tr;
}

const deadNote = /** @type {HTMLElement} */ ($('[data-kt-dead-note]', deadSection));
function renderDead() {
    const rows = DATA.dead_letters;
    const added = reconcile(deadBody, rows, deadKey, deadRow);
    if (booted) {
        for (const row of added) attachAll?.(row);
        deadTable?.refresh();
    }
    const total = DATA.topic.dead;
    deadNote.textContent = total > rows.length ? `Showing the ${num(rows.length)} newest of ${num(total)} dead letters.` : '';
    renderProblems();
}

/** The dead letters that are on the page, grouped by subscription. */
function deadBySub() {
    /** @type {Map<string, any[]>} */
    const map = new Map();
    for (const d of DATA.dead_letters) map.set(d.subscription, [...(map.get(d.subscription) ?? []), d]);
    return map;
}

/** What bulk answers when the route is not there yet, as a refusal on every key. @param {string[]} keys @param {string} reason */
const refuseAll = (keys, reason) => ({ done: [], refused: keys.map((key) => ({ id: key, reason })) });

/** Run one bulk action over [{subscription, id}], one call per subscription.
 * @param {'requeue' | 'delete'} action @param {{ subscription: string, id: string }[]} items */
async function bulk(action, items) {
    /** @type {Map<string, string[]>} */
    const groups = new Map();
    for (const it of items) groups.set(it.subscription, [...(groups.get(it.subscription) ?? []), it.id]);
    /** @type {{ subscription: string, id: string }[]} */
    const done = [];
    /** @type {{ subscription: string, id: string, reason: string }[]} */
    const refused = [];
    for (const [subscription, ids] of groups) {
        const r = await call(`${T}/bulk/${action}`, { json: { subscription, ids } });
        if (!r.ok) {
            const why = r.status === 404 ? 'bulk actions are not available on this hub yet' : sentence(/** @type {any} */ (r));
            for (const id of ids) refused.push({ subscription, id, reason: why });
            continue;
        }
        const answer = r.json ?? { done: ids, refused: [] };
        for (const id of answer.done ?? []) done.push({ subscription, id });
        for (const x of answer.refused ?? []) refused.push({ subscription, id: x.id, reason: x.reason });
    }
    return { done, refused };
}
void refuseAll;

/** The problems of the dead letters: what died, why, and what now. */
async function renderProblems() {
    const groups = deadBySub();
    const items = [];
    for (const [name, list] of groups) {
        const s = SUBS.get(name);
        const total = s ? Math.max(s.dead, list.length) : list.length;
        const archived = s?.state === 'archived';
        const last = Math.max(...list.map((d) => d.dead_ms ?? 0));
        const button = make('button', 'kp-button', `Requeue ${num(list.length)}`);
        button.type = 'button';
        button.setAttribute('data-kt-fix', name);
        button.setAttribute('data-kp-flash-words', 'Requeued,Refused,Failed');
        button.setAttribute('data-kp-refusal', 'toast');
        items.push({
            key: name,
            severity: archived ? 'info' : s?.state === 'flagged' ? 'critical' : 'warning',
            what: `${plural(total, 'message', 'messages')} died on ${name}`,
            why: `${name} did not acknowledge ${total === 1 ? 'it' : 'them'} in ${s?.max_attempts ?? 'the allowed number of'} attempts.${last ? ` The last one died on ${dmy(last)}.` : ''}`,
            remedy: archived
                ? `${name} is archived and no longer delivers. Delete these dead letters, or bring it back and requeue them:`
                : `Fix whatever reads ${name}, then requeue them.`,
            command: archived
                ? `curl -s -X POST -H "authorization: Bearer $KYU_TOKEN" "${ORIGIN}/api/t/${TOPIC}/subs/${name}/unarchive"`
                : undefined,
            action: button,
        });
    }
    // The oldest dead letter against how long dead letters are kept.
    if (retentionDays && DATA.dead_letters.length) {
        const oldest = DATA.dead_letters.reduce((a, b) => ((a.dead_ms ?? a.published_ms) < (b.dead_ms ?? b.published_ms) ? a : b));
        const at = oldest.dead_ms ?? oldest.published_ms;
        const days = Math.floor((DATA.now_ms - at) / 86_400_000);
        if (days >= 1) {
            const button = make('button', 'kp-button', 'Show it');
            button.type = 'button';
            button.setAttribute('data-kt-oldest', deadKey(oldest));
            items.push({
                key: 'oldest',
                severity: 'info',
                what: `The oldest dead letter is ${plural(days, 'day', 'days')} old`,
                why: `Dead letters are kept for ${plural(retentionDays, 'day', 'days')} and then pruned.`,
                remedy: `Requeue or delete it before ${dmy(at + retentionDays * 86_400_000).slice(0, 10)}.`,
                action: button,
            });
        }
    }
    if (items.length === 0) {
        setProblems(problemsList, []);
        problemsHost.hidden = true;
        problemsView.ready();
        return;
    }
    problemsHost.hidden = false;
    problemsView.ready();
    setProblems(problemsList, items);
}

problemsList.addEventListener('click', async (event) => {
    const target = event.target instanceof Element ? event.target : null;
    const oldest = target?.closest('[data-kt-oldest]');
    if (oldest instanceof HTMLElement) {
        const key = oldest.dataset.ktOldest ?? '';
        if (location.hash === `#${key}`) location.hash = '';
        location.hash = key;
        return;
    }
    const button = target?.closest('[data-kt-fix]');
    if (!(button instanceof HTMLElement)) return;
    const name = button.dataset.ktFix ?? '';
    const list = deadBySub().get(name) ?? [];
    await runFlashButton(button, async () => {
        const { done, refused } = await bulk('requeue', list.map((d) => ({ subscription: name, id: d.id })));
        if (refused.length) return { error: `${plural(refused.length, 'message', 'messages')} not requeued.`, remedy: refused[0].reason };
        void done;
        return 'Requeued';
    });
    void load();
});

// Rows: requeue, delete with Undo.
deadBody.addEventListener('click', async (event) => {
    const button = event.target instanceof Element ? event.target.closest('[data-kt-row]') : null;
    const row = button?.closest('tr');
    if (!(button instanceof HTMLElement) || !row) return;
    const sub = row.dataset.ktSub ?? '';
    const id = row.dataset.ktId ?? '';
    if (button.dataset.ktRow === 'requeue') {
        let ok = false;
        await runFlashButton(button, async () => {
            const r = await call(`${T}/requeue`, { form: { subscription: sub, id } });
            if (!r.ok) return refusal(/** @type {any} */ (r));
            ok = true;
            return 'Requeued';
        });
        if (ok) {
            await new Promise((resolve) => setTimeout(resolve, 500));
            row.remove();
            deadTable?.refresh();
            void load();
        }
        return;
    }
    const next = row.nextElementSibling;
    row.remove();
    deadTable?.refresh();
    pending += 1;
    let settled = false;
    const settle = () => {
        if (!settled) pending -= 1;
        settled = true;
    };
    const putBack = () => {
        deadBody.insertBefore(row, next && next.isConnected ? next : null);
        deadTable?.refresh();
    };
    undoToast(`Dead letter ${id} deleted`, {
        ms: 6000,
        max: 3,
        commit: async () => {
            const r = await call(`${T}/delivery/delete`, { form: { subscription: sub, id } });
            settle();
            if (!r.ok) throw new Error(sentence(/** @type {any} */ (r)));
            void load();
        },
        undo: () => {
            settle();
            putBack();
        },
        restore: () => {
            settle();
            putBack();
        },
    });
});

// The selection bar: requeue or delete the ticked rows.
selectionBar.addEventListener('kp-selection-action', (event) => {
    const detail = /** @type {CustomEvent} */ (event).detail;
    /** @type {string[]} */
    const keys = detail.keys;
    const rows = $$('tr', deadBody).filter((r) => keys.includes(r.getAttribute('data-kp-row-key') ?? ''));
    const items = rows.map((r) => ({ subscription: r.dataset.ktSub ?? '', id: r.dataset.ktId ?? '' }));
    const keyOf = (/** @type {{ subscription: string, id: string }} */ i) => `dead-${i.subscription}-${i.id}`;
    if (detail.action === 'delete') {
        const places = rows.map((r) => r.nextElementSibling);
        for (const r of rows) {
            const box = /** @type {HTMLInputElement | null} */ (r.querySelector('[data-kp-select-row]'));
            if (box) {
                box.checked = false;
                box.dispatchEvent(new Event('change', { bubbles: true }));
            }
        }
        for (const r of rows) r.remove();
        deadTable?.refresh();
        pending += 1;
        let settled = false;
        const settle = () => {
            if (!settled) pending -= 1;
            settled = true;
        };
        const putBack = (/** @type {HTMLElement[]} */ list) => {
            for (const r of list) {
                const i = rows.indexOf(r);
                const place = places[i];
                deadBody.insertBefore(r, place && place.isConnected ? place : null);
            }
            deadTable?.refresh();
        };
        /** @type {HTMLElement[]} */
        let restoreOnly = rows;
        undoToast(`${plural(keys.length, 'dead letter', 'dead letters')} deleted`, {
            ms: detail.undoMs ?? 6000,
            max: 3,
            commit: async () => {
                const { refused } = await bulk('delete', items);
                settle();
                if (refused.length) {
                    const gone = new Set(refused.map((x) => keyOf(x)));
                    restoreOnly = rows.filter((r) => gone.has(r.getAttribute('data-kp-row-key') ?? ''));
                    throw new Error(`${plural(refused.length, 'dead letter', 'dead letters')} could not be deleted: ${refused[0].reason}`);
                }
                void load();
            },
            undo: () => {
                settle();
                putBack(rows);
            },
            restore: () => putBack(restoreOnly),
        });
        return;
    }
    setSelectionBusy(selectionBar, `Requeueing ${num(keys.length)}…`);
    void bulk('requeue', items).then(({ done, refused }) => {
        setSelectionBusy(selectionBar, '');
        setSelectionResult(selectionBar, {
            done: done.length,
            refused: refused.map((x) => ({ key: keyOf(x), reason: x.reason })),
        });
        const gone = new Set(done.map((x) => keyOf(x)));
        for (const r of rows) if (gone.has(r.getAttribute('data-kp-row-key') ?? '')) r.remove();
        deadTable?.refresh();
        void load();
    });
});

// ---------------------------------------------------------------- 6 · recent messages

const recent = /** @type {HTMLElement} */ ($('[data-kt-recent]'));
const recentStage = stage(/** @type {HTMLElement} */ ($('[data-kt-stage="recent"]', recent)));
const mdRoot = /** @type {HTMLElement} */ ($('[data-kt-md]', recent));
const items = /** @type {HTMLElement} */ ($('[data-kt-items]', recent));
/** @type {Map<string, any>} */
const MESSAGES = new Map();
let viewerOpen = /** @type {import('/static/kp/js/viewer.js').ViewerHandle | null} */ (null);

/** @param {any} m */
const snippet = (m) => (m.payload.binary ? `binary, ${bytesText(m.payload.bytes)}` : m.payload.text === '' ? 'empty payload' : m.payload.text);

/** @param {any} m */
function messageItem(m) {
    const li = make('li');
    li.setAttribute('role', 'option');
    li.setAttribute('data-kp-pane-key', m.id);
    li.dataset.kpKey = m.id;
    const wrap = make('span', 'kt-msg');
    const ago = make('time', 'kp-ago');
    ago.setAttribute('data-kp-ago', '');
    ago.setAttribute('data-kp-ago-verb', '');
    ago.setAttribute('datetime', iso(m.published_ms));
    ago.textContent = m.published_at;
    const small = make('small');
    small.append(ago);
    wrap.append(make('b', '', m.id), small, make('code', '', snippet(m)));
    li.append(wrap);
    return li;
}

/** @param {any} m */
function openPayload(m, opener = null) {
    viewerOpen?.close();
    const p = m.payload;
    const name = `message-${m.id}.${p.binary ? 'bin' : 'json'}`;
    const url = URL.createObjectURL(new Blob([p.binary ? new Uint8Array(0) : p.text], { type: p.binary ? 'application/octet-stream' : 'application/json' }));
    const viewer = openViewer({
        title: `Message ${m.id}`,
        desc: p.binary ? 'A binary payload: kyu cannot show it as text.' : 'Shown as JSON, laid out. Raw is one press away.',
        facts: `${p.binary ? 'Binary' : 'JSON'} · ${bytesText(p.bytes)} · ${dmy(m.published_ms)}`,
        size: 'wide',
        opener,
        actions: [
            {
                kind: 'copy',
                label: 'Copy payload',
                run: () => {
                    const b = viewer.foot.querySelector('[data-kp-viewer-action="copy"]');
                    if (!(b instanceof HTMLElement)) return;
                    b.setAttribute('data-kp-flash-words', 'Copied,Refused');
                    void runFlashButton(b, async () => {
                        try {
                            await navigator.clipboard.writeText(p.text);
                            return 'Copied';
                        } catch {
                            return { error: 'Your browser blocked copying.', remedy: 'Select the text and press Ctrl+C.' };
                        }
                    });
                },
            },
            ...(p.binary ? [] : [{ kind: 'download', href: url, name }]),
        ],
        onClose: () => {
            URL.revokeObjectURL(url);
            if (viewerOpen === viewer) viewerOpen = null;
        },
    });
    viewerOpen = viewer;
    if (p.binary) {
        viewer.setState('unsupported', { message: `This payload is binary (${bytesText(p.bytes)}); it cannot be shown as text.` });
        return;
    }
    const fig = make('figure', 'kp-code');
    fig.setAttribute('data-kp-code', '');
    viewer.setContent(fig);
    setCode(fig, { text: p.text, title: 'Payload', language: 'JSON', lines: true, maxLines: 0, state: p.text === '' ? 'empty' : 'ready' });
}

const detailBody = /** @type {HTMLElement} */ ($('[data-kt-md-body]', recent));
/** @param {string} id */
let filledId = '';
function fillDetail(id) {
    const m = MESSAGES.get(id) ?? [...MESSAGES.values()][0];
    if (!m || filledId === m.id) return;
    filledId = m.id;
    const heading = $('[data-kp-pane-heading]', mdRoot);
    if (heading) heading.textContent = `Message ${m.id}`;
    detailBody.replaceChildren();
    const facts = make('dl', 'kt-facts-list');
    const rows = [
        ['Published', dmy(m.published_ms)],
        ['Content type', m.content_type ?? 'not given'],
        ['Size', bytesText(m.payload.bytes)],
        ...(m.due_ms ? [['Due', dmy(m.due_ms)]] : []),
    ];
    for (const [k, v] of rows) facts.append(make('dt', '', k), make('dd', '', v));
    const fig = make('figure', 'kp-code');
    fig.setAttribute('data-kp-code', '');
    const actions = make('div', 'kt-detail-actions');
    const open = make('button', 'kp-button', 'Open large');
    open.type = 'button';
    open.addEventListener('click', () => openPayload(m, open));
    const replay = make('button', 'kp-button kp-button--primary', 'Replay this message');
    replay.type = 'button';
    replay.setAttribute('data-kp-flash-button', '');
    replay.setAttribute('data-kp-flash-words', 'Replayed,Refused,Failed');
    replay.setAttribute('data-kp-flash-word', 'Replayed');
    replay.setAttribute('data-kp-refusal', 'toast');
    replay.addEventListener('click', () =>
        runFlashButton(replay, async () => {
            const r = await call(`${T}/messages/${encodeURIComponent(m.id)}/replay`);
            if (!r.ok) {
                return r.status === 404 || r.status === 405
                    ? { error: 'Replay is not available on this hub yet.', remedy: 'Update kyu, or publish the payload again from the box below.' }
                    : refusal(/** @type {any} */ (r));
            }
            void load();
            return { word: 'Replayed', line: `Published again as ${r.json?.id ?? 'a new message'}. Every active subscription will receive it.` };
        }),
    );
    actions.append(open, replay);
    detailBody.append(facts, fig, actions);
    setCode(fig, {
        text: m.payload.text,
        title: 'Payload',
        language: 'JSON',
        lines: true,
        maxLines: 12,
        size: bytesText(m.payload.bytes),
        state: m.payload.binary ? 'binary' : m.payload.text === '' ? 'empty' : 'ready',
    });
    if (booted) attachAll?.(actions);
}

let selectedMessage = '';
function renderRecent() {
    const list = DATA.messages;
    MESSAGES.clear();
    for (const m of list) MESSAGES.set(m.id, m);
    filledId = '';
    const count = $('[data-kt-recent-count]', recent);
    if (count) count.textContent = list.length ? `The newest ${num(list.length)} of ${num(DATA.topic.messages)} messages retained on ${TOPIC}.` : '';
    if (list.length === 0) {
        recentStage.plate(statePlate(document, { kind: 'empty', title: 'No messages yet', text: `Messages published to ${TOPIC} show here.` }), 'none');
        return;
    }
    recentStage.ready();
    const known = new Set([...items.children].map((li) => /** @type {HTMLElement} */ (li).dataset.kpKey));
    const fresh = list.filter((/** @type {any} */ m) => !known.has(m.id));
    const wanted = new Set(list.map((/** @type {any} */ m) => m.id));
    for (const li of [...items.children]) if (!wanted.has(/** @type {HTMLElement} */ (li).dataset.kpKey)) li.remove();
    const first = known.size === 0;
    // Newest first: new arrivals go to the head.
    const lis = fresh.map(messageItem);
    items.prepend(...lis);
    if (!first) for (const li of lis) li.setAttribute('data-new', '');
    setTimeout(() => lis.forEach((li) => li.removeAttribute('data-new')), 2500);
    if (booted) {
        for (const li of lis) attachAll?.(li);
        const md = masterDetail(mdRoot);
        if (md && md.key() === null) md.select(list[0].id);
        else if (md) fillDetail(String(md.key()));
    }
}

mdRoot.addEventListener('kp-pane-select', (event) => {
    const key = /** @type {CustomEvent} */ (event).detail?.key;
    if (key == null) return;
    selectedMessage = String(key);
    fillDetail(selectedMessage);
    setPaneState(mdRoot, 'ready');
});
mdRoot.addEventListener('kp-pane-open', (event) => {
    const key = /** @type {CustomEvent} */ (event).detail?.key;
    if (key == null) return;
    fillDetail(String(key));
    setPaneState(mdRoot, 'ready');
});
mdRoot.addEventListener('kp-pane-retry', () => void load());

// ---------------------------------------------------------------- 7 · publish a test message

const form = /** @type {HTMLFormElement} */ (document.getElementById('kt-publish-form'));
const area = /** @type {HTMLTextAreaElement} */ (document.getElementById('kt-publish-body'));
const sendButton = /** @type {HTMLElement} */ (document.getElementById('kt-publish-send'));
form.addEventListener('submit', (event) => event.preventDefault());
form.addEventListener('kp-form-valid', () => {
    void runFlashButton(sendButton, async () => {
        const r = await call(`${T}/publish`, { form: { payload: area.value } });
        if (!r.ok) return refusal(/** @type {any} */ (r));
        void load();
        return { word: 'Published', line: `Published to ${TOPIC}.` };
    });
});

// ---------------------------------------------------------------- reading the page again

/** Read the retention of dead letters once; a hub without the route answers 404 and the problem is left out. */
async function readRetention() {
    try {
        const res = await fetch(`${T}/retention`, { headers: { Accept: 'application/json' }, credentials: 'same-origin' });
        if (!res.ok) return;
        const json = await res.json();
        if (Number.isFinite(json.dead_letter_days) && json.dead_letter_days > 0) retentionDays = json.dead_letter_days;
    } catch {
        // No retention: the "oldest dead letter" problem is left out.
    }
}

/** Fetch this page again and take its data block. */
async function load() {
    if (pending > 0) return;
    const res = await fetch(location.pathname + location.search, { headers: { Accept: 'text/html' }, credentials: 'same-origin' });
    if (!res.ok) throw new Error(`the page answered ${res.status}`);
    const doc = new DOMParser().parseFromString(await res.text(), 'text/html');
    const block = doc.getElementById('kyu-topic-data');
    if (!block) throw new Error('the page had no data');
    if (pending > 0) return;
    DATA = JSON.parse(block.textContent ?? '{}');
    apply();
}

function apply() {
    renderHeader();
    renderReaders();
    renderSubs();
    subsTable?.refresh();
    renderDead();
    renderRecent();
}

// ---------------------------------------------------------------- first paint, then the kit's boot

renderHeader();
renderCommands();
renderReaders();
SUBS.clear();
for (const s of DATA.subscriptions) SUBS.set(s.name, { ...s });
renderSubs();
renderDead();
renderRecent();
updateCounts();

const root = document.documentElement;
const autoReady = new Promise((resolve) => {
    if (root.hasAttribute('data-kp-auto-ready')) return resolve(undefined);
    new MutationObserver((_, watch) => {
        if (!root.hasAttribute('data-kp-auto-ready')) return;
        watch.disconnect();
        resolve(undefined);
    }).observe(root, { attributes: true, attributeFilter: ['data-kp-auto-ready'] });
});
const auto = await import('/static/kp/js/auto.js');
attachAll = (/** @type {ParentNode} */ r) => auto.attachAll(r);
await autoReady;
booted = true;

subsTable = dataTable(subsTableEl);
deadTable = dataTable(deadTableEl);
fixTooltips($$('tr', subsBody), false);
updateCounts();

{
    const md = masterDetail(mdRoot);
    if (md && DATA.messages.length) {
        md.select(DATA.messages[0].id);
        fillDetail(DATA.messages[0].id);
        setPaneState(mdRoot, 'ready');
    }
}

await readRetention();
renderProblems();

const refreshEl = $('[data-kt-refresh]');
refreshEl?.addEventListener('kp-refresh', (event) => {
    const { done } = /** @type {CustomEvent} */ (event).detail;
    load().then(
        () => done(true),
        () => done(false),
    );
});

// A hash that names a dead letter lands on its row (the anchors module marks it).
void startOver;
