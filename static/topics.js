// kyu: topics.js — the Topics page on kp-themes 11.0.0.
//
//   1. the data: the server renders every topic as one JSON block inside the
//      key-figure group (`topics_json`); the automatic refresh fetches this
//      same page and swaps that group, so each refresh brings a fresh block;
//   2. the drawing from that data with the package's own functions
//      (setIdmark, setBarCell, setStackbar, setAgo, setAttention); nothing
//      here draws a component itself;
//   3. the two history sections, which read GET /dashboard/stats/hourly and
//      GET /dashboard/stats/totals and draw them with setHeat and setTrendData.
//
// The components attach through auto.js; this file only fills them and listens.
import { setIdmark, idmarkColours } from '/static/kp/js/idmark.js';
import { setStackbar } from '/static/kp/js/stackbar.js';
import { setBarCell } from '/static/kp/js/barcell.js';
import { setAgo } from '/static/kp/js/freshness.js';
import { setAttention } from '/static/kp/js/attention.js';
import { dataTable, VIEW_EVENT } from '/static/kp/js/datatable.js';
import { tickTo } from '/static/kp/js/tick.js';
import { update } from '/static/kp/js/update.js';
import { setHeat, HEAT_PICK_EVENT } from '/static/kp/js/heat.js';
import { setTrendData } from '/static/kp/js/chart.js';
import { setCardState, CARD_RETRY_EVENT } from '/static/kp/js/card-state.js';
import { toast } from '/static/kp/js/overlays.js';
import { attachAll } from '/static/kp/js/auto.js';

/** @param {string} selector @param {ParentNode} [root] */
const $ = (selector, root = document) => /** @type {HTMLElement | null} */ (root.querySelector(selector));
/** @param {string} selector @param {ParentNode} [root] */
const $$ = (selector, root = document) => /** @type {HTMLElement[]} */ ([...root.querySelectorAll(selector)]);

/**
 * @param {string} tag @param {Record<string, string | boolean | undefined>} [attrs] @param {(Node | string)[]} [kids]
 * @returns {HTMLElement}
 */
function h(tag, attrs = {}, ...kids) {
    const el = document.createElement(tag);
    for (const [name, value] of Object.entries(attrs)) {
        if (value === undefined || value === false) continue;
        el.setAttribute(name, value === true ? '' : value);
    }
    el.append(...kids);
    return el;
}

const number = (/** @type {number} */ n) => n.toLocaleString('en');
const plural = (/** @type {number} */ n, /** @type {string} */ one, /** @type {string} */ many) => `${number(n)} ${n === 1 ? one : many}`;
const HOUR = 3_600_000;
/** A topic counts as quiet after this long, in seconds (the server's status word uses the same 24 h). */
const QUIET_AFTER = 86_400;
const STATUS_BADGE = { 'Dead letters': 'kp-badge--destructive', Backlog: 'kp-badge--warning', Quiet: 'kp-badge--info', Healthy: 'kp-badge--success' };

/**
 * @typedef {object} Topic
 * @property {string} name
 * @property {keyof typeof STATUS_BADGE} status
 * @property {number} messages
 * @property {string[]} subscriptions
 * @property {string[]} flagged
 * @property {number} backlog
 * @property {number} in_flight
 * @property {number} dead
 * @property {number} lapsed
 * @property {number | null} last_published_ms
 * @property {string | null} dead_note
 */

/** The page's data block, taken out of the DOM once read. @returns {{ now_ms: number, topics: Topic[] } | null} */
function takeData() {
    const block = $('[data-tp-data]');
    if (!block) return null;
    block.remove();
    try {
        return JSON.parse(block.textContent ?? '');
    } catch {
        return null;
    }
}

const ui = { pressed: { backlog: false, dead: false }, ready: false, failed: false };
/** @type {{ now_ms: number, topics: Topic[] }} */
let model = { now_ms: Date.now(), topics: [] };

const kpis = $('[data-tp-kpis]');
const band = $('[data-tp-attention]');
const host = $('[data-tp-table-host]');

const totals = () => ({
    topics: model.topics.length,
    subscriptions: model.topics.reduce((n, x) => n + x.subscriptions.length, 0),
    messages: model.topics.reduce((n, x) => n + x.messages, 0),
    backlog: model.topics.reduce((n, x) => n + x.backlog, 0),
    inflight: model.topics.reduce((n, x) => n + x.in_flight, 0),
    dead: model.topics.reduce((n, x) => n + x.dead, 0),
    lapsed: model.topics.reduce((n, x) => n + x.lapsed, 0),
});

/* ------------------------------------------------------------------------------------------ the drawing */

let slots = idmarkColours([]);
/** The marks never collide while the palette has room: the slots are decided from the topics in order. */
const decideSlots = () => {
    slots = idmarkColours(model.topics.map((t) => t.name));
};

/** @param {HTMLElement} el @param {string} name */
function mark(el, name) {
    setIdmark(el, name, { slot: slots.get(name) });
}

/** The key-figure tiles as markup. */
function kpiTiles() {
    const t = totals();
    const calm = t.backlog + t.dead === 0;
    const value = (/** @type {string} */ key, /** @type {number} */ n) => {
        const span = h('span', { 'data-kp-tick': true, 'data-tp-key': key });
        span.textContent = number(n);
        return h('span', { class: 'kp-kpi__value' }, span);
    };
    const trend = (/** @type {string} */ key, /** @type {string} */ text) =>
        h('span', { class: 'kp-kpi__trend', 'data-tp-key': `${key}-words` }, text);
    const hint = h(
        'span',
        { class: 'kp-kpi__hint', 'aria-hidden': 'true' },
        h('span', { 'data-kp-when': 'off' }, 'Filter'),
        h('span', { 'data-kp-when': 'on' }, 'Filtering ×'),
    );
    const toggle = (/** @type {string} */ key, /** @type {string} */ label, /** @type {string} */ tone, /** @type {string} */ words) =>
        h(
            'button',
            {
                type: 'button',
                class: 'kp-kpi kp-kpi--toggle',
                'aria-pressed': String(ui.pressed[/** @type {'backlog' | 'dead'} */ (key)]),
                'data-kp-tone': tone || undefined,
                'data-tp-filter': key,
            },
            h('span', { class: 'kp-kpi__label' }, label),
            hint.cloneNode(true),
            value(key, key === 'backlog' ? t.backlog : t.dead),
            trend(key, words),
        );
    return [
        h(
            'div',
            { class: 'kp-kpi', 'data-tp-tile': 'topics' },
            h('span', { class: 'kp-kpi__label' }, 'Topics'),
            value('topics', t.topics),
            trend('topics', `${plural(t.subscriptions, 'subscription', 'subscriptions')} in all`),
        ),
        h(
            'div',
            { class: 'kp-kpi', 'data-tp-tile': 'messages' },
            h('span', { class: 'kp-kpi__label' }, 'Messages'),
            value('messages', t.messages),
            trend('messages', 'kept across all topics'),
        ),
        toggle('backlog', 'Backlog', calm ? '' : 'warning', calm ? 'nothing is waiting' : `${number(t.inflight)} more in flight`),
        toggle('dead', 'Dead', calm ? '' : 'destructive', t.dead === 0 ? 'nothing needs a decision' : `${number(t.lapsed)} lapsed, not dead yet`),
    ];
}

/** @returns {import('/static/kp/js/attention.js').AttentionItem[]} */
function attentionItems() {
    const show = (/** @type {string} */ name, /** @type {string} */ label) =>
        h('a', { class: 'kp-button kp-button--sm', href: `#topic-${name}`, 'data-tp-show': name }, label);
    /** @type {import('/static/kp/js/attention.js').AttentionItem[]} */
    const items = [];
    for (const t of model.topics) {
        if (t.dead > 0) {
            items.push({
                key: `dead-${t.name}`,
                severity: 'critical',
                title: `${t.name}: ${plural(t.dead, 'dead letter', 'dead letters')}`,
                text: `A subscription gave up on ${t.dead === 1 ? 'a message' : 'these messages'} after its last attempt. Open the topic to requeue or delete.`,
                action: show(t.name, 'Show the topic'),
            });
        }
    }
    for (const t of model.topics) {
        if (t.backlog >= 50) {
            items.push({
                key: `backlog-${t.name}`,
                severity: 'warning',
                title: `${t.name}: ${number(t.backlog)} messages waiting`,
                text: 'No subscription has taken them yet; they are safe and will be delivered in order.',
                action: show(t.name, 'Show the topic'),
            });
        }
    }
    for (const t of model.topics) {
        for (const sub of t.flagged) {
            items.push({
                key: `flagged-${t.name}-${sub}`,
                severity: 'warning',
                title: `The subscription ${sub} is flagged`,
                text: `It has stopped polling ${t.name}; a flagged subscription is worth a look before it is archived.`,
                action: show(t.name, `Show ${t.name}`),
            });
        }
    }
    for (const t of model.topics) {
        if (t.status === 'Quiet' && t.last_published_ms !== null) {
            const hours = Math.round((model.now_ms - t.last_published_ms) / HOUR);
            items.push({
                key: `quiet-${t.name}`,
                severity: 'info',
                title: `${t.name} has been quiet for ${number(hours)} h`,
                text: 'Nothing is wrong with it; the publisher may simply have nothing to say.',
                action: show(t.name, 'Show the topic'),
            });
        }
    }
    return items;
}

/** @param {Topic} t */
function flowSpec(t) {
    const parts = [
        { key: 'backlog', label: 'Waiting', value: t.backlog },
        { key: 'inflight', label: 'In flight', value: t.in_flight },
        { key: 'dead', label: 'Dead', value: t.dead, tone: /** @type {const} */ ('destructive') },
        { key: 'lapsed', label: 'Lapsed', value: t.lapsed, tone: /** @type {const} */ ('warning') },
    ].filter((p) => p.value > 0);
    const sum = parts.reduce((n, p) => n + p.value, 0);
    return {
        parts,
        total: Math.max(sum, 1),
        legend: false,
        format: number,
        restLabel: sum === 0 ? 'Nothing waiting' : 'Rest',
        label: `${t.name}: ${sum === 0 ? 'nothing waiting' : parts.map((p) => `${number(p.value)} ${p.label.toLowerCase()}`).join(', ')}`,
    };
}

/** @param {Topic} t @returns {HTMLTableRowElement} */
function buildRow(t) {
    const cell = (/** @type {string} */ label, /** @type {string} */ cls, /** @type {Node[]} */ kids) =>
        h('td', { 'data-label': label, class: cls || undefined }, ...kids);
    const markEl = h('span', { class: 'kp-idmark kp-idmark--sm', 'data-kp-idmark': t.name, 'aria-hidden': 'true' });
    mark(markEl, t.name);
    const href = `/t/${encodeURIComponent(t.name)}/dashboard`;
    const row = /** @type {HTMLTableRowElement} */ (
        h(
            'tr',
            { 'data-kp-row-key': t.name, 'data-kp-anchor': `topic-${t.name}`, 'data-tp-topic': t.name },
            cell('Topic', 'tp-name', [
                h('span', { class: 'tp-topic' }, markEl, h('a', { class: 'tp-topic__link', href }, t.name)),
            ]),
            cell('Status', '', [h('span', { class: 'kp-badge', 'data-tp-cell': 'status' })]),
            cell('Messages', 'kp-text-end kp-numeric', [h('span', { 'data-kp-tick': true, 'data-tp-cell': 'messages' })]),
            cell('Backlog', '', [h('span', { class: 'kp-bar-cell', 'data-tp-cell': 'backlog' })]),
            cell('Dead', '', [h('span', { class: 'kp-bar-cell tp-hover', tabindex: '0', 'data-tp-cell': 'dead' })]),
            cell('Flow', 'tp-flow', [h('figure', { class: 'kp-stackbar', 'data-kp-stackbar': true, 'data-tp-cell': 'flow' })]),
            cell('Last published', '', [
                h('time', { class: 'kp-ago tp-hover', 'data-kp-ago': true, 'data-kp-ago-verb': 'published', tabindex: '0', 'data-tp-cell': 'last' }),
            ]),
            cell('Actions', 'tp-actions', [
                h(
                    'div',
                    { class: 'kp-row-actions' },
                    h('button', { type: 'button', class: 'kp-button kp-button--sm', 'data-kp-action': 'test', 'data-tp-test': t.name }, 'Send test'),
                    h('a', { class: 'kp-button kp-button--sm kp-button--primary', href, 'data-kp-action': 'open' }, 'Open'),
                ),
            ]),
        )
    );
    paintRow(row, t);
    return row;
}

/** Write one topic into its row. The cells that carry a number tick when they change, the words change the theme's way. @param {HTMLElement} row @param {Topic} t */
function paintRow(row, t, animate = false) {
    const max = {
        backlog: Math.max(0, ...model.topics.map((x) => x.backlog)),
        dead: Math.max(0, ...model.topics.map((x) => x.dead)),
    };
    const cells = (/** @type {string} */ key) => /** @type {HTMLElement} */ (row.querySelector(`[data-tp-cell="${key}"]`));
    const td = (/** @type {string} */ key) => /** @type {HTMLElement} */ (cells(key).closest('td'));
    const badge = cells('status');
    if (badge.textContent !== t.status) {
        badge.className = `kp-badge ${STATUS_BADGE[t.status] ?? ''}`;
        if (animate && badge.textContent) update(badge, t.status);
        else badge.textContent = t.status;
    }
    td('status').setAttribute('data-kp-filter-value', t.status);
    td('status').setAttribute('data-kp-sort-value', t.status);

    const messages = cells('messages');
    if (messages.textContent !== number(t.messages)) {
        if (animate && messages.textContent) tickTo(messages, number(t.messages));
        else messages.textContent = number(t.messages);
    }
    td('messages').setAttribute('data-kp-sort-value', String(t.messages));

    const link = /** @type {HTMLElement} */ (row.querySelector('.tp-topic__link'));
    link.setAttribute(
        'data-kp-hovercard',
        t.subscriptions.length
            ? `${t.name} is read by ${plural(t.subscriptions.length, 'subscription', 'subscriptions')}: ${t.subscriptions.join(', ')}`
            : `${t.name} has no subscriptions yet`,
    );

    setBarCell(cells('backlog'), {
        value: max.backlog ? t.backlog / max.backlog : 0,
        text: number(t.backlog),
        sortValue: t.backlog,
        label: 'of the largest backlog',
    });
    setBarCell(cells('dead'), {
        value: max.dead ? t.dead / max.dead : 0,
        text: number(t.dead),
        sortValue: t.dead,
        label: 'of the most dead letters',
    });
    cells('dead').setAttribute(
        'data-kp-hovercard',
        t.dead > 0 ? `Latest dead letter on ${t.name}, ${t.dead_note ?? 'no detail kept'}` : `No dead letters on ${t.name}`,
    );

    const flow = flowSpec(t);
    setStackbar(cells('flow'), flow);
    td('flow').setAttribute('data-kp-sort-value', String(flow.parts.reduce((n, p) => n + p.value, 0)));

    const last = cells('last');
    last.setAttribute('data-kp-stale-after', String(QUIET_AFTER));
    if (t.last_published_ms === null) {
        last.removeAttribute('data-kp-ago');
        last.textContent = 'never';
        last.removeAttribute('datetime');
    } else {
        last.setAttribute('data-kp-ago', '');
        setAgo(last, t.last_published_ms);
    }
    last.setAttribute('data-kp-hovercard', `Last message on ${t.name}. Hover or focus shows the exact time; quiet after 24 h.`);
    td('last').setAttribute('data-kp-sort-value', String(t.last_published_ms ?? 0));
}

/* ------------------------------------------------------------------------------------------ the page */

const tableEl = () => $('[data-tp-datatable]');
const table = () => {
    const el = tableEl();
    return el ? dataTable(el) : null;
};
const rowsOf = () => $$('[data-tp-topic]');

function fillRows() {
    const body = $('[data-tp-rows]');
    if (!body) return;
    body.replaceChildren(...model.topics.map(buildRow));
}

function drawKpis() {
    kpis?.replaceChildren(...kpiTiles());
    remember();
}

/** The text each counting element shows, by key, so a swapped-in tile can count from what was shown. */
const shown = new Map();
function remember() {
    shown.clear();
    for (const el of $$('[data-tp-key]', kpis ?? document)) shown.set(el.getAttribute('data-tp-key'), el.textContent ?? '');
}

function drawBand() {
    if (band) setAttention(band, attentionItems());
}

function drawHubBar() {
    const bar = $('[data-tp-hubbar]');
    if (!bar) return;
    const t = totals();
    const sum = t.backlog + t.inflight + t.dead + t.lapsed;
    const state = sum === 0 ? 'empty' : 'ready';
    setStackbar(bar, {
        title: 'Where the undelivered messages are, across the hub',
        parts: [
            { key: 'backlog', label: 'Waiting', value: t.backlog },
            { key: 'inflight', label: 'In flight', value: t.inflight },
            { key: 'dead', label: 'Dead', value: t.dead, tone: 'destructive' },
            { key: 'lapsed', label: 'Lapsed', value: t.lapsed, tone: 'warning' },
        ].filter((p) => p.value > 0),
        total: Math.max(sum, 1),
        restLabel: 'Nothing waiting',
        format: number,
        state: /** @type {any} */ (state),
        message: t.topics === 0 ? 'No topics yet.' : 'Nothing is waiting anywhere on the hub.',
        retry: false,
    });
}

function drawMeta() {
    const meta = $('[data-tp-meta]');
    if (!meta) return;
    const time = h('time', { class: 'kp-ago', 'data-kp-ago': true, 'data-kp-ago-verb': 'updated', 'data-kp-stale-after': '180' });
    meta.replaceChildren(h('span', {}, `${plural(model.topics.length, 'topic', 'topics')}\u00a0\u00b7\u00a0`), time);
    setAgo(time, model.now_ms);
}


/** Everything the data changes: the figures, the band, the bar and the rows. @param {boolean} [animate] */
function paintAll(animate = false) {
    drawKpis();
    drawBand();
    drawHubBar();
    drawMeta();
    for (const row of rowsOf()) {
        const t = model.topics.find((x) => x.name === row.getAttribute('data-tp-topic'));
        if (t) paintRow(row, t, animate);
    }
    table()?.refresh();
}

/* ---- the key figures filter the table (Backlog and Dead are range filters on their columns) */

const COLUMN = { backlog: 3, dead: 4 };
let applying = false;
function applyFilters() {
    const dt = table();
    if (!dt) return;
    applying = true;
    for (const key of /** @type {const} */ (['backlog', 'dead'])) dt.filter(COLUMN[key], ui.pressed[key] ? { from: '1' } : null);
    applying = false;
    syncTiles(dt.view());
}

document.addEventListener('kp-kpi-toggle', (event) => {
    const tile = /** @type {HTMLElement} */ (event.target).closest('[data-tp-filter]');
    if (!tile) return;
    ui.pressed[/** @type {'backlog' | 'dead'} */ (tile.getAttribute('data-tp-filter'))] = /** @type {CustomEvent} */ (event).detail.pressed;
    applyFilters();
});

/** The tiles follow the table's filters. @param {import('/static/kp/js/datatable.js').View} view */
function syncTiles(view) {
    const asks = (/** @type {number} */ column) => {
        const f = view.filters?.[column];
        return Boolean(f && !Array.isArray(f) && f.from === '1' && !f.to);
    };
    for (const key of /** @type {const} */ (['backlog', 'dead'])) {
        ui.pressed[key] = asks(COLUMN[key]);
        $(`[data-tp-filter="${key}"]`)?.setAttribute('aria-pressed', String(ui.pressed[key]));
    }
}

// A filter removed from its pill lets go of its tile, and a filter set in the panel lights it.
document.addEventListener(VIEW_EVENT, (event) => {
    if (!(event.target instanceof Element) || !event.target.matches('[data-tp-datatable]')) return;
    const view = /** @type {CustomEvent} */ (event).detail;
    if (!applying) syncTiles(view);
    const empty = $('[data-tp-nomatch]');
    if (empty && view.total > 0 && view.shown === 0) {
        const deadOnly = ui.pressed.dead && !ui.pressed.backlog && Object.keys(view.filters ?? {}).length === 1 && !view.query;
        const allClear = deadOnly && totals().dead === 0;
        empty.setAttribute('data-kp-art', allClear ? 'done' : 'nomatch');
        $('[data-tp-empty-title]')?.replaceChildren(allClear ? 'Nothing is dead' : 'No topic matches');
        $('[data-tp-empty-body]')?.replaceChildren(
            allClear
                ? 'No message has run out of attempts on any topic. Nothing needs a decision.'
                : 'Loosen the search or remove a filter.',
        );
    }
});

/* ---- fix links land on the row; Send test publishes a known message */

document.addEventListener('click', (event) => {
    const target = event.target instanceof Element ? event.target : null;
    if (target?.closest('[data-tp-show]')) {
        // The row must be on the table before the link lands on it.
        const dt = table();
        dt?.clearFilters();
        dt?.query('');
        for (const key of /** @type {const} */ (['backlog', 'dead'])) ui.pressed[key] = false;
    }
    const test = target?.closest('[data-tp-test]');
    if (test) void sendTest(/** @type {HTMLButtonElement} */ (test));
});

/** @param {HTMLButtonElement} button */
async function sendTest(button) {
    const name = button.getAttribute('data-tp-test') ?? '';
    if (button.disabled) return;
    button.disabled = true;
    try {
        const response = await fetch(`/t/${encodeURIComponent(name)}/dashboard/publish`, {
            method: 'POST',
            headers: { 'content-type': 'application/x-www-form-urlencoded', accept: 'application/json' },
            body: `payload=${encodeURIComponent(JSON.stringify({ test: 'hello from the kyu dashboard' }))}`,
        });
        if (!response.ok) {
            let words = `The hub refused it (${response.status}).`;
            try {
                const body = await response.json();
                if (body.error) words = `${body.error}${body.remedy ? ` ${body.remedy}` : ''}`;
            } catch {
                /* the answer was not JSON; the status says enough */
            }
            toast(`Test message not published to ${name}. ${words}`, { sticky: true, live: 'assertive' });
            return;
        }
        toast(`Test message published to ${name}. The figures pick it up at the next refresh.`);
    } catch {
        toast(`Test message not published to ${name}. The hub did not answer.`, { sticky: true, live: 'assertive' });
    } finally {
        button.disabled = false;
    }
}

/* ---- the automatic refresh swaps the key-figure group; the rest follows from the block it carries */

let swapping = false;
if (kpis) {
    new MutationObserver(() => {
        if (swapping || !ui.ready) return;
        const data = takeData();
        if (!data) return;
        swapping = true;
        const before = new Map(shown);
        model = data;
        decideSlots();
        kpis.replaceChildren(...kpiTiles());
        // New tiles start on what the old ones showed and count to the new numbers; the words change the theme's way.
        for (const el of $$('[data-kp-tick][data-tp-key]', kpis)) {
            const old = before.get(el.getAttribute('data-tp-key'));
            const next = el.textContent ?? '';
            if (old !== undefined && old !== next) {
                el.textContent = old;
                tickTo(el, next);
            }
        }
        for (const words of $$('.kp-kpi__trend[data-tp-key]', kpis)) {
            const old = before.get(words.getAttribute('data-tp-key'));
            const next = words.textContent ?? '';
            if (old !== undefined && old !== next) {
                words.textContent = old;
                update(words, next);
            }
        }
        remember();
        drawBand();
        drawHubBar();
        drawMeta();
            syncRows();
        table()?.refresh();
        swapping = false;
    }).observe(kpis, { childList: true });
}

/** Rows for topics that appeared are added, rows for topics that went are removed, the rest are repainted. */
function syncRows() {
    const body = $('[data-tp-rows]');
    if (!body) return;
    const names = new Set(model.topics.map((t) => t.name));
    for (const row of rowsOf()) if (!names.has(row.getAttribute('data-tp-topic') ?? '')) row.remove();
    for (const t of model.topics) {
        const row = rowsOf().find((r) => r.getAttribute('data-tp-topic') === t.name);
        if (row) paintRow(row, t, true);
        else body.append(buildRow(t));
    }
}

/* ------------------------------------------------------------------------------------------ the history sections */

/**
 * Read a JSON route. A route that is not there yet (404) or fails is a failure the component shows in its own words.
 * @param {string} url
 */
async function readJson(url) {
    const response = await fetch(url, { headers: { accept: 'application/json' } });
    if (!response.ok) throw new Error(`${url} answered ${response.status}`);
    return response.json();
}

/* ---- the heat map: topic by hour */

const brussels = new Intl.DateTimeFormat('en-GB', { timeZone: 'Europe/Brussels', hour: '2-digit', hour12: false });
const hourLabel = (/** @type {number} */ ms) => brussels.format(new Date(ms)).replace(/^24$/, '00').padStart(2, '0');

const heat = {
    /** @type {'loading' | 'ready' | 'empty' | 'error'} */
    state: 'loading',
    metric: /** @type {'published' | 'dead'} */ ('published'),
    picked: /** @type {string | null} */ (null),
    /** @type {{ hours: number[], topics: { topic: string, published: number[], dead: number[] }[] } | null} */
    data: null,
};

function heatSpec() {
    const data = heat.data;
    if (!data) return { columns: [], rows: [] };
    const metric = heat.metric;
    const hours = data.hours;
    const peak = Math.max(1, ...data.topics.flatMap((t) => t.published));
    const columns = hours.map((ms, i) => ({ key: new Date(ms).toISOString(), label: i % 3 === 0 || i === hours.length - 1 ? hourLabel(ms) : '' }));
    const rows = data.topics.map((topic) => ({
        key: topic.topic,
        label: topic.topic,
        mark: `var(--chart-${(slots.get(topic.topic) ?? 0) + 1})`,
        cells: hours.map((ms, i) => {
            const n = topic[metric][i] ?? 0;
            const when = `${hourLabel(ms)}:00`;
            if (metric === 'dead')
                return {
                    tone: /** @type {const} */ (n === 0 ? 'ok' : n === 1 ? 'warn' : 'bad'),
                    label: n === 0 ? 'no dead letters' : `${n} ${n === 1 ? 'dead letter' : 'dead letters'}`,
                    aria: `${topic.topic} at ${when}: ${n === 0 ? 'no dead letters' : `${n} dead`}`,
                    tip: `${topic.topic} at ${when}: ${n === 0 ? 'nothing died' : `${n} ${n === 1 ? 'message' : 'messages'} died`}`,
                };
            return {
                tone: /** @type {const} */ (n === 0 ? 'none' : 'level'),
                level: Math.sqrt(n / peak),
                label: `${number(n)} published`,
                aria: `${topic.topic} at ${when}: ${number(n)} published`,
                tip: `${topic.topic} at ${when}: ${number(n)} ${n === 1 ? 'message' : 'messages'} published`,
            };
        }),
    }));
    const hubAt = (/** @type {number} */ i) => data.topics.reduce((n, t) => n + (t[metric][i] ?? 0), 0);
    const hubPeak = Math.max(1, ...hours.map((_, i) => hubAt(i)));
    const summary = {
        key: 'hub',
        label: 'Whole hub',
        cells: hours.map((ms, i) => {
            const total = hubAt(i);
            if (metric === 'dead')
                return {
                    tone: /** @type {const} */ (total === 0 ? 'ok' : total === 1 ? 'warn' : 'bad'),
                    share: total === 0 ? 1 : Math.min(1, total / 3),
                    label: total === 0 ? 'no dead letters' : `${total} dead`,
                    tip: `Whole hub at ${hourLabel(ms)}:00: ${total === 0 ? 'nothing died' : `${total} died`}`,
                };
            return {
                tone: /** @type {const} */ ('level'),
                level: Math.sqrt(total / hubPeak),
                share: total / hubPeak,
                label: `${number(total)} published`,
                tip: `Whole hub at ${hourLabel(ms)}:00: ${number(total)} published`,
            };
        }),
    };
    return {
        columns,
        rows,
        summary,
        every: 3,
        label: `Messages ${metric === 'dead' ? 'that died' : 'published'} per topic per hour, last 24 hours`,
        toggle: true,
        picked: heat.picked,
    };
}

function drawHeat() {
    const grid = $('[data-tp-heat]');
    if (!grid) return;
    setHeat(grid, {
        ...heatSpec(),
        state: /** @type {any} */ (heat.state),
        skeleton: { rows: Math.max(model.topics.length, 3), columns: 24 },
        message: heat.state === 'empty' ? 'No history yet. kyu has not counted a full hour.' : 'The hourly counts could not be read.',
    });
    // The heat map has no Retry of its own: a failed read is the card's own failed state with Try again.
    const card = $('[data-tp-heat-stage]');
    if (card) {
        if (heat.state === 'error') setCardState(card, 'error', { title: 'Could not read the hourly counts', text: 'The hub did not answer.' });
        else setCardState(card, 'ready');
    }
    const legend = $('[data-tp-heat-legend]');
    if (legend) {
        const item = (/** @type {string} */ tone, /** @type {string} */ text, /** @type {string} */ level = '') =>
            h('li', {}, h('span', { class: 'kp-heat__swatch', 'data-kp-tone': tone, style: level ? `--kp-heat-level: ${level}` : undefined }), ` ${text}`);
        legend.replaceChildren(
            ...(heat.metric === 'dead'
                ? [item('ok', 'nothing died'), item('warn', 'one died'), item('bad', 'two or more died')]
                : [item('none', 'nothing published'), item('level', 'a few', '0.2'), item('level', 'some', '0.55'), item('level', 'the most', '1')]),
        );
    }
}

async function loadHeat() {
    heat.state = 'loading';
    drawHeat();
    try {
        const data = await readJson('/dashboard/stats/hourly?hours=24');
        heat.data = data;
        heat.state = data.topics?.length && data.hours?.length ? 'ready' : 'empty';
    } catch {
        heat.data = null;
        heat.state = 'error';
    }
    drawHeat();
}

document.addEventListener(HEAT_PICK_EVENT, (event) => {
    const detail = /** @type {CustomEvent} */ (event).detail;
    const readout = $('[data-tp-heat-readout]');
    if (!readout || !(event.target instanceof Element) || !event.target.matches('[data-tp-heat]')) return;
    heat.picked = detail.picked ? detail.colKey : null;
    const data = heat.data;
    const i = data ? data.hours.findIndex((ms) => new Date(ms).toISOString() === detail.colKey) : -1;
    if (!detail.picked || !data || i < 0) {
        readout.textContent = 'No hour pinned. Press a cell to pin its hour.';
        return;
    }
    const published = data.topics.reduce((n, t) => n + (t.published[i] ?? 0), 0);
    const dead = data.topics.reduce((n, t) => n + (t.dead[i] ?? 0), 0);
    readout.textContent = `${hourLabel(data.hours[i])}:00 pinned: ${number(published)} published across the hub, ${dead === 0 ? 'none died' : `${dead} died`}.`;
});
for (const b of $$('[data-tp-heat-metric]')) {
    b.addEventListener('click', () => {
        heat.metric = /** @type {any} */ (b.getAttribute('data-tp-heat-metric'));
        for (const other of $$('[data-tp-heat-metric]')) other.setAttribute('aria-pressed', String(other === b));
        drawHeat();
    });
}
$('[data-tp-heat-stage]')?.addEventListener(CARD_RETRY_EVENT, () => void loadHeat());

/* ---- the key figures with their last 24 hours */

const SAMPLE_STEP = 300_000;
/** `key` is the field of a totals sample; `bad` is whether a rise is bad news. */
const TRENDS = [
    { key: 'messages', label: 'Messages', note: 'kept', bad: false, tone: '' },
    { key: 'backlog', label: 'Backlog', note: 'waiting', bad: true, tone: 'warning' },
    { key: 'in_flight', label: 'In flight', note: 'being worked', bad: false, tone: '' },
    { key: 'dead', label: 'Dead', note: 'dead letters', bad: true, tone: 'destructive' },
];
const trend = {
    /** @type {'loading' | 'ready' | 'empty' | 'error'} */
    state: 'loading',
    /** @type {{ at: number, messages: number, backlog: number, dead: number, in_flight: number }[]} */
    samples: [],
};

function drawTrends() {
    const strip = $('[data-tp-trends]');
    if (!strip) return;
    // The trend chart has no failed state of its own: a failed read is the card's own failed state with Try again.
    const card = $('[data-tp-trend-card]');
    if (card) {
        if (trend.state === 'error') {
            setCardState(card, 'error', { title: 'Could not read the hourly history', text: 'The hub did not answer. The numbers on the Topics page are still current.' });
            return;
        }
        setCardState(card, 'ready');
    }
    if (strip.children.length === 0) {
        strip.replaceChildren(
            ...TRENDS.map((tr, i) =>
                h(
                    'div',
                    { class: 'kp-kpi kp-kpi--trend', 'data-tp-trend': tr.key, 'data-kp-tone': tr.tone || undefined },
                    h('span', { class: 'kp-kpi__label' }, tr.label, ' ', h('span', { class: 'kp-kpi__label-note' }, tr.note)),
                    h('span', { class: 'kp-kpi__value' }),
                    h('span', { class: 'kp-kpi__trend' }),
                    h('figure', {
                        class: 'kp-kpi__chart',
                        'data-kp-chart': 'spark',
                        'data-kp-spark-head': 'none',
                        'data-kp-spark-axis': 'relative',
                        'aria-label': `${tr.label}, last 24 hours`,
                        style: `--kp-chart-series: var(--chart-${(i % 5) + 1})`,
                    }),
                ),
            ),
        );
        void attachAll(strip);
    }
    const samples = trend.samples;
    for (const tr of TRENDS) {
        const tile = /** @type {HTMLElement} */ ($(`[data-tp-trend="${tr.key}"]`, strip));
        const value = /** @type {HTMLElement} */ ($('.kp-kpi__value', tile));
        const words = /** @type {HTMLElement} */ ($('.kp-kpi__trend', tile));
        const figure = /** @type {HTMLElement} */ ($('.kp-kpi__chart', tile));
        const skeleton = (/** @type {HTMLElement} */ el, /** @type {string} */ width) =>
            el.replaceChildren(Object.assign(h('span', { class: 'kp-skeleton' }), { style: `inline-size: ${width}` }));
        if (trend.state === 'loading') {
            skeleton(value, '4ch');
            skeleton(words, '80%');
            setTrendData(figure, null);
            continue;
        }
        const series = samples.map((s) => /** @type {[number, number]} */ ([s.at, Number(s[/** @type {'messages'} */ (tr.key)] ?? 0)]));
        if (trend.state === 'empty') {
            value.textContent = number(totalsNow(tr.key));
            words.textContent = 'collecting the first 24 hours';
            setTrendData(figure, { points: [], step: SAMPLE_STEP, unit: '', digits: 0 });
            continue;
        }
        const last = series[series.length - 1][1];
        const change = last - series[0][1];
        const peak = Math.max(...series.map((p) => p[1]));
        value.textContent = number(last);
        words.replaceChildren(
            h(
                'span',
                {
                    class: 'kp-kpi__delta',
                    'data-kp-direction': change > 0 ? 'up' : change < 0 ? 'down' : undefined,
                    'data-kp-tone': change === 0 ? undefined : tr.bad === change > 0 ? 'bad' : 'good',
                },
                number(Math.abs(change)),
            ),
            ` since yesterday · peak ${number(peak)}`,
        );
        setTrendData(figure, { points: series, step: SAMPLE_STEP, unit: '', digits: 0 });
    }
}

/** The figure the page itself shows now, for a tile that has no history yet. @param {string} key */
function totalsNow(key) {
    const t = totals();
    return key === 'in_flight' ? t.inflight : key === 'messages' ? t.messages : key === 'backlog' ? t.backlog : t.dead;
}

async function loadTrends() {
    trend.state = 'loading';
    drawTrends();
    try {
        const data = await readJson('/dashboard/stats/totals?hours=24');
        trend.samples = Array.isArray(data.samples) ? data.samples : [];
        trend.state = trend.samples.length >= 2 ? 'ready' : 'empty';
    } catch {
        trend.samples = [];
        trend.state = 'error';
    }
    drawTrends();
}
$('[data-tp-trend-card]')?.addEventListener(CARD_RETRY_EVENT, () => void loadTrends());

/* ------------------------------------------------------------------------------------------ first render */

// All of it is in the markup before auto.js attaches the components.
{
    const data = takeData();
    if (data) model = data;
    decideSlots();
    if ($('[data-tp-stage]')) {
        fillRows();
        paintAll();
    }
    drawHeat();
    drawTrends();
}

const autoReady = new Promise((resolve) => {
    const done = () => document.documentElement.hasAttribute('data-kp-auto-ready');
    if (done()) return resolve(undefined);
    new MutationObserver((_, watch) => {
        if (done()) {
            watch.disconnect();
            resolve(undefined);
        }
    }).observe(document.documentElement, { attributes: true, attributeFilter: ['data-kp-auto-ready'] });
});
void autoReady.then(() => {
    ui.ready = true;
    void loadHeat();
    void loadTrends();
});
