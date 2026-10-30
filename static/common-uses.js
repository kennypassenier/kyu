// kyu: common-uses.js, "who reads what" on the Topics page: the matrix of topics by apps, read from the palette's own options.

import { heat, segmented, footprint } from './common-kit.js';
import { all, one, counted } from './common-context.js';
import { mountGraph } from './common-uses-graph.js';


/**
 * What the palette's options say about the hub: every topic, every subscription and its tone.
 * @param {ParentNode} palette the palette dialog, the page's own or the one in a freshly read page
 */
function collectUses(palette) {
    const topicOptions = all('[data-kyu-topic]:not([data-kyu-sub])', palette);
    const subOptions = all('[data-kyu-sub]', palette);
    const topics = topicOptions.map((o) => ({
        key: o.dataset.kyuTopic ?? '',
        label: o.dataset.kyuTopic ?? '',
        sub: counted(Number(o.dataset.kyuMessages ?? 0), 'message'),
        href: o.getAttribute('href') ?? undefined,
    }));
    const names = [...new Set(subOptions.map((o) => o.dataset.kyuSub ?? ''))].sort();
    const apps = names.map((name) => ({ key: name, label: name }));
    const edges = new Map(
        subOptions.map((o) => [`${o.dataset.kyuTopic}|${o.dataset.kyuSub}`, { tone: o.dataset.kyuTone, label: o.dataset.kyuDetail }]),
    );
    const subs = subOptions.map((o) => ({
        id: `${o.dataset.kyuTopic}/${o.dataset.kyuSub}`,
        topic: o.dataset.kyuTopic ?? '',
        app: o.dataset.kyuSub ?? '',
        kind: o.dataset.kyuTone === 'ok' ? 'active' : o.dataset.kyuTone === 'muted' ? 'archived' : 'flagged',
        waiting: Number(o.dataset.kyuBacklog ?? 0),
        note: o.dataset.kyuDetail ?? '',
    }));
    return { topics, apps, edges, subs };
}

/**
 * Read the Topics page again and take the palette out of it: the same read the automatic refresh does, so the section
 * is in its loading state while it runs and in its failed state, with Retry, when kyu does not answer.
 * @returns {Promise<ReturnType<typeof collectUses>>}
 */
async function readUses() {
    const res = await fetch(location.pathname, { headers: { Accept: 'text/html' }, credentials: 'same-origin' });
    if (!res.ok) throw new Error(`the page answered ${res.status}`);
    const doc = new DOMParser().parseFromString(await res.text(), 'text/html');
    const template = /** @type {HTMLTemplateElement | null} */ (doc.getElementById('kyu-shell'));
    const palette = template?.content.getElementById('kyu-palette');
    if (!palette) throw new Error('the page had no palette');
    return collectUses(palette);
}


/** Put the section below the topics table, or at the end of the page. @param {HTMLElement} section */
function placeSection(section) {
    const target = one('main .kp-datatable, main .kp-table-wrap, main table');
    const host = one('main');
    if (target) (target.closest('.kp-datatable') ?? target).after(section);
    else host?.append(section);
}

/** @typedef {{ model: ReturnType<typeof collectUses> | null, view: 'lines' | 'grid' }} UsesState */

/** @param {HTMLElement} match @param {UsesState} u */
function drawMatrix(match, u) {
    const model = u.model;
    if (!model) return;
    if (model.topics.length === 0 || model.apps.length === 0) {
        heat.setMatrix(match, { view: 'lines', rows: [], columns: [], state: 'empty', message: 'No topic has a subscription yet' });
        return;
    }
    const { edges } = model;
    heat.setMatrix(match, {
        corner: 'Topic ↓ app →',
        rows: model.topics,
        columns: model.apps,
        cell: (row, column) => edges.get(`${row}|${column}`) ?? null,
        countLabel: (n) => counted(n, 'subscription'),
        sourcesLabel: 'Topics',
        usersLabel: 'Apps',
        view: u.view,
    });
}

/** The Lines / Grid switch of the matrix. @param {HTMLElement} section @param {HTMLElement} match @param {UsesState} u */
function wireViewSwitch(section, match, u) {
    segmented.attachSegmented(section);
    section.addEventListener('click', (event) => {
        const item = /** @type {HTMLElement | null} */ ((/** @type {Element} */ (event.target)).closest('[data-kyu-switch="view"] [data-value]'));
        if (!item) return;
        u.view = item.dataset.value === 'grid' ? 'grid' : 'lines';
        drawMatrix(match, u);
    });
    // A view the person left on Grid last time is remembered by the matrix itself.
    if (match.getAttribute('data-kp-view') === 'grid') {
        u.view = 'grid';
        for (const item of all('[data-kyu-switch="view"] [data-value]', section)) item.setAttribute('aria-checked', String(item.dataset.value === 'grid'));
    }
}

/** @param {HTMLElement} section the section from the template; it goes below the topics table */
export function mountUses(section) {
    const match = /** @type {HTMLElement} */ (section.querySelector('#kyu-uses-matrix'));
    placeSection(section);
    /** @type {UsesState} */
    const u = { model: null, view: 'lines' };
    wireViewSwitch(section, match, u);
    const drawGraph = mountGraph(section, () => u.model);
    // The section arrives after auto.js attached the page, so its own matrix is held by hand (js/footprint.js).
    footprint.attachFootprints(section);

    /** Read the hub again: loading, then the matrix and the graph, or the failed plate of each. */
    const read = async () => {
        heat.setMatrix(match, { view: 'lines', rows: [], columns: [], state: 'loading' });
        drawGraph('loading');
        try {
            u.model = await readUses();
        } catch {
            u.model = null;
            heat.setMatrix(match, { view: 'lines', rows: [], columns: [], state: 'error', message: 'Could not read who reads what' });
            drawGraph('error');
            return;
        }
        drawMatrix(match, u);
        drawGraph('ready');
    };
    // The packages' own Retry buttons fire these (heat.js MATRIX_RETRY_EVENT, graph.js GRAPH_RETRY_EVENT).
    match.addEventListener('kp-matrix-retry', () => void read());
    section.querySelector('#kyu-uses-graph')?.addEventListener('kp-graph-retry', () => void read());
    // attachGraphs draws its loading state itself; the first read follows once it has.
    requestAnimationFrame(() => void read());
}
