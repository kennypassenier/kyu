// kyu: the parts that cross every page (kp-themes 11.0.0 adoption, demo kyu-11-overview).
//
// Loaded by templates/_kp.html on every page, before /static/kp/js/auto.js. The server wrote
// the palette's options into a <template id="kyu-shell"> (every topic, every subscription,
// Publish, Prune, Apps); this file moves the dialogs into the body and gives them their
// behaviour:
//
//   - the command palette (Ctrl+K), with Publish and Prune run here;
//   - the keys: g t, g a, /, p, the ? sheet and the hint line under the page;
//   - Help in the bar, its drawer, and the first-visit tour of the topic page;
//   - link anchors on headings and on rows;
//   - "who reads what" on the Topics page, from the palette's own options.
//
// Every control is a kp-themes component; kyu only decides what they say and do.

const KP = '/static/kp/js/';

const [palette, shortcuts, anchors, keys, tour, overlays, components, heat, graph, segmented, footprint] = await Promise.all([
    import(`${KP}palette.js`),
    import(`${KP}shortcuts.js`),
    import(`${KP}anchors.js`),
    import(`${KP}keys.js`),
    import(`${KP}tour.js`),
    import(`${KP}overlays.js`),
    import(`${KP}components.js`),
    import(`${KP}heat.js`),
    import(`${KP}graph.js`),
    import(`${KP}segmented.js`),
    import(`${KP}footprint.js`),
]);

const TOUR_NAME = 'kyu-topic-page';
const PATH = location.pathname.replace(/\/+$/, '');
const TOPIC_MATCH = /^\/t\/([^/]+)\/dashboard(?:\/|$)/.exec(PATH);
const CONTEXT_TOPIC = TOPIC_MATCH ? decodeURIComponent(TOPIC_MATCH[1]) : '';
const ON_TOPIC_PAGE = /^\/t\/[^/]+\/dashboard$/.test(PATH);
const ON_TOPICS_PAGE = PATH === '/topics';

/** @param {string} selector @param {ParentNode} [root] @returns {HTMLElement[]} */
const all = (selector, root = document) => /** @type {HTMLElement[]} */ ([...root.querySelectorAll(selector)]);
/** @param {string} selector @param {ParentNode} [root] @returns {HTMLElement | null} */
const one = (selector, root = document) => /** @type {HTMLElement | null} */ (root.querySelector(selector));

/** Plural of a count: "1 dead letter", "3 dead letters". */
const counted = (n, noun) => `${n} ${noun}${n === 1 ? '' : 's'}`;

// ── the server's markup, moved into the body ────────────────────────────────────────────────────

const shell = /** @type {HTMLTemplateElement | null} */ (document.getElementById('kyu-shell'));
if (shell) {
    const fragment = /** @type {DocumentFragment} */ (shell.content.cloneNode(true));
    const uses = fragment.getElementById('kyu-uses');
    uses?.remove();
    document.body.append(fragment);
    if (uses && ON_TOPICS_PAGE) mountUses(uses);
}

const dialog = /** @type {HTMLDialogElement | null} */ (document.getElementById('kyu-palette'));
const sheet = /** @type {HTMLDialogElement | null} */ (document.getElementById('kyu-sheet'));
const help = /** @type {HTMLDialogElement | null} */ (document.getElementById('kyu-help'));
const prune = /** @type {HTMLButtonElement | null} */ (document.getElementById('kyu-prune'));

// ── the command palette ─────────────────────────────────────────────────────────────────────────

/** The publish box of the topic page, wherever the page builder put it. */
const publishField = () =>
    one('form[action$="/dashboard/publish"] [name="payload"]') ?? one('[data-kyu-publish]') ?? null;

if (dialog) {
    if (CONTEXT_TOPIC) dialog.setAttribute('data-kp-context', CONTEXT_TOPIC);

    const publish = one('[data-value="publish"]', dialog);
    const publishWords = one('.kp-palette__description', publish ?? dialog);
    if (publish && publishWords) {
        publishWords.textContent = ON_TOPIC_PAGE ? (publish.dataset.hintTopic ?? '') : (publish.dataset.hintElsewhere ?? '');
    }

    const dead = Number(one('[data-value="prune"]', dialog)?.dataset.dead ?? 0);
    const pruneWords = one('[data-value="prune"] .kp-palette__description', dialog);
    if (pruneWords) {
        pruneWords.textContent = dead > 0
            ? `Delete the ${counted(dead, 'dead letter')} on this hub, after you type prune`
            : 'Nothing is dead, so there is nothing to prune';
    }

    palette.attachPalettes(document.body);
    if (sheet) sheet.setAttribute('data-kp-primary', '');

    dialog.addEventListener('kp-palette-run', (event) => {
        const value = String(/** @type {CustomEvent} */ (event).detail?.value ?? '');
        // After the palette has closed and handed the focus back to where it came from.
        const after = (act) => {
            if (!dialog.open) return act();
            dialog.addEventListener('close', () => setTimeout(act, 0), { once: true });
        };
        if (value === 'publish') after(() => (publishField() ? goPublish() : location.assign('/topics')));
        else if (value === 'prune') after(() => askPrune(dead));
    });
}

function goPublish() {
    const field = publishField();
    if (!field) return false;
    field.scrollIntoView({ block: 'center' });
    field.focus();
    return true;
}

function askPrune(dead) {
    if (dead === 0) {
        overlays.toast('Nothing is dead on this hub.');
        return;
    }
    prune?.click();
}

if (prune) {
    components.attachConfirmations(document.body);
    prune.addEventListener('click', async () => {
        try {
            const response = await fetch('/dashboard/dead-letters/prune', { method: 'POST', headers: { accept: 'application/json' } });
            if (!response.ok) throw new Error(String(response.status));
            const { pruned } = await response.json();
            overlays.toast(`Pruned ${counted(Number(pruned), 'dead letter')}.`);
            setTimeout(() => location.reload(), 900);
        } catch {
            overlays.toast('The dead letters could not be pruned. Nothing was changed.', { live: 'assertive', close: true });
        }
    });
}

// ── the keys ────────────────────────────────────────────────────────────────────────────────────

shortcuts.configureShortcuts({ unknownHint: true });
shortcuts.registerShortcuts('page', [
    { keys: 'g t', what: 'Go to topics', group: 'Anywhere', run: () => location.assign('/topics') },
    { keys: 'g a', what: 'Go to apps', group: 'Anywhere', run: () => location.assign('/clients') },
    { keys: '/', what: 'Search topics and apps', group: 'Anywhere', run: () => palette.palette(dialog)?.open(), when: () => !!dialog },
    {
        keys: 'p',
        what: 'Publish a test message',
        group: 'Anywhere',
        run: () => goPublish(),
        when: () => ON_TOPIC_PAGE && !!publishField(),
        reason: 'Open a topic first',
    },
]);
if (sheet) shortcuts.attachShortcuts(document.body);

// The hint line, under the page.
const main = one('main');
if (main) {
    const line = document.createElement('div');
    line.className = 'kp-keys kyu-keys';
    keys.setKeys(line, [
        [['g', 't'], 'topics'],
        [['g', 'a'], 'apps'],
        ['/', 'search'],
        ...(ON_TOPIC_PAGE ? [['p', 'publish']] : []),
        ['?', 'all keys'],
        [['Mod', 'K'], 'palette'],
    ]);
    main.append(line);
}

// ── Help, its drawer and the tour ───────────────────────────────────────────────────────────────

/** @type {HTMLButtonElement | null} */
let helpButton = null;
const menu = one('.kp-nav .kp-theme-menu');
if (help && menu) {
    helpButton = document.createElement('button');
    helpButton.type = 'button';
    helpButton.className = 'kp-icon-button kyu-help-button';
    helpButton.setAttribute('aria-label', 'Help');
    helpButton.setAttribute('title', 'Help');
    helpButton.textContent = '?';
    menu.before(helpButton);
    helpButton.addEventListener('click', () => help.showModal());
    for (const close of all('[data-kyu-help-close]', help)) close.addEventListener('click', () => help.close());

    const list = one('[data-kyu-help-keys]', help);
    if (list) {
        const rows = [['Mod K', 'The command palette.'], ['?', 'Every key.'], ...shortcuts.listShortcuts().map((s) => [s.keys, `${s.what}.`])];
        for (const [combo, what] of rows) {
            const term = document.createElement('dt');
            combo.split(' ').forEach((part, index) => {
                if (index > 0) term.append(' ');
                const cap = document.createElement('kbd');
                cap.className = 'kp-kbd';
                cap.textContent = part.toLowerCase() === 'mod' ? (/mac|iphone|ipad/i.test(navigator.platform) ? '⌘' : 'Ctrl') : part;
                term.append(cap);
            });
            const def = document.createElement('dd');
            def.textContent = what;
            list.append(term, def);
        }
    }

    // On a page that is not a topic, the tour is a link to one: the first topic the palette lists.
    one('[data-kyu-help-tour]', help)?.addEventListener('click', () => {
        help.close();
        if (ON_TOPIC_PAGE) return runTour(0);
        const first = one('[data-kyu-topic][href]:not([data-kyu-sub])', dialog ?? document);
        if (first) location.assign(`${first.getAttribute('href')}?tour`);
    });
    if (!ON_TOPIC_PAGE && !one('[data-kyu-topic]:not([data-kyu-sub])', dialog ?? document)) one('[data-kyu-help-tour]', help)?.remove();
}

/**
 * The part of the topic page a step talks about: a marked element, else the content that follows the heading.
 * @param {string} mark @param {RegExp} heading
 */
function section(mark, heading) {
    return () => {
        const marked = one(`[data-kyu-tour="${mark}"]`);
        if (marked) return marked;
        const title = all('main h2').find((h) => heading.test(h.textContent ?? ''));
        if (!title) return null;
        let next = title.nextElementSibling;
        while (next && next.matches('p, .text-secondary, .explain')) next = next.nextElementSibling;
        return /** @type {HTMLElement | null} */ (next ?? title);
    };
}

const STEPS = [
    { target: () => one('[data-kyu-tour="snippets"]') ?? one('#snippet-publish'), title: 'Your commands', text: 'Four commands, one for each step: publish, receive, acknowledge, check. The secret is masked; Reveal shows it for ten seconds and Copy never shows it.' },
    { target: section('subs', /^Subscriptions/i), title: 'Subscriptions', text: 'Each app that reads this topic has its own place in line. Flagged means a message died or a lease ran out.' },
    { target: section('dead', /^Dead letters/i), title: 'Dead letters', text: 'A message nobody could deliver. Requeue puts it back in line; Delete removes it after you type the name.' },
    { target: () => publishField()?.closest('form') ?? publishField(), title: 'Publish a test message', text: 'Type JSON: it is checked as you type, with the line and column of a mistake. Ctrl+Enter publishes.' },
    { target: () => helpButton, title: 'Help', text: 'This tour again, and every key. Adding ?tour to the address starts it too.' },
];

function runTour(start) {
    return tour.startTour(STEPS, { start, remember: TOUR_NAME, returnFocus: helpButton });
}

if (ON_TOPIC_PAGE) {
    const at = tour.shouldStartTour({
        search: location.search,
        remembered: tour.tourRemembered(TOUR_NAME),
        automated: navigator.webdriver === true,
    });
    // The page builder's sections are in the DOM by now; give a late render one frame.
    if (at !== null) requestAnimationFrame(() => runTour(at));
}

// ── anchors: headings and rows ──────────────────────────────────────────────────────────────────

/** The id a row is linked by, from what it holds: topic, subscription, dead letter or message. */
function rowId(row) {
    const own = row.getAttribute('data-kp-anchor');
    if (own) return own;
    const link = one('a[href]', row)?.getAttribute('href') ?? '';
    const sub = /\/dashboard\/subs\/([^/?#]+)/.exec(link);
    if (sub) return `sub-${anchors.anchorSlug(decodeURIComponent(sub[1]))}`;
    const topic = /^\/t\/([^/]+)\/dashboard\/?$/.exec(link);
    if (topic) return `topic-${anchors.anchorSlug(decodeURIComponent(topic[1]))}`;
    const code = one('td code, th code', row)?.textContent?.trim();
    if (code) {
        const table = row.closest('table');
        const dead = !!table && /dead/i.test(table.closest('section, div')?.previousElementSibling?.textContent ?? '') ;
        return `${dead ? 'dead' : 'message'}-${anchors.anchorSlug(code)}`;
    }
    return '';
}

/** Give every row of the page's tables its anchor, and let the tables carry the marks. */
function anchorRows() {
    for (const table of all('main table')) {
        let any = false;
        for (const row of all('tbody > tr', table)) {
            if (row.hasAttribute('data-kp-anchor') && row.getAttribute('data-kp-anchor')) { any = true; continue; }
            const id = rowId(row);
            if (!id) continue;
            row.setAttribute('data-kp-anchor', id);
            const label = (one('a, code', row)?.textContent ?? '').trim();
            if (label) row.setAttribute('data-kp-anchor-label', label);
            any = true;
        }
        if (any && !table.hasAttribute('data-kp-anchors')) table.setAttribute('data-kp-anchors', 'rows');
        if (any) anchors.attachAnchors(table.parentElement ?? table);
    }
}

if (main) {
    main.setAttribute('data-kp-anchors', 'headings');
    anchorRows();
    anchors.attachAnchors(main);
    // Tables a page fills or re-draws later (a data table's rows) get their anchors when they appear.
    let pending = 0;
    new MutationObserver((records) => {
        if (!records.some((r) => [...r.addedNodes].some((n) => n instanceof HTMLElement && (n.matches('tr') || n.querySelector('tr'))))) return;
        clearTimeout(pending);
        pending = window.setTimeout(anchorRows, 50);
    }).observe(main, { childList: true, subtree: true });
}

// ── who reads what (Topics page) ────────────────────────────────────────────────────────────────

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

/** @param {HTMLElement} section the section from the template; it goes below the topics table */
function mountUses(section) {
    const match = /** @type {HTMLElement} */ (section.querySelector('#kyu-uses-matrix'));
    const target = one('main .kp-datatable, main .kp-table-wrap, main table');
    const host = one('main');
    if (target) (target.closest('.kp-datatable') ?? target).after(section);
    else host?.append(section);

    /** @type {ReturnType<typeof collectUses> | null} */
    let model = null;
    let view = 'lines';
    const drawMatrix = () => {
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
            view,
        });
    };
    segmented.attachSegmented(section);
    section.addEventListener('click', (event) => {
        const item = /** @type {HTMLElement | null} */ ((/** @type {Element} */ (event.target)).closest('[data-kyu-switch="view"] [data-value]'));
        if (!item) return;
        view = item.dataset.value === 'grid' ? 'grid' : 'lines';
        drawMatrix();
    });
    // A view the person left on Grid last time is remembered by the matrix itself.
    if (match.getAttribute('data-kp-view') === 'grid') {
        view = 'grid';
        for (const item of all('[data-kyu-switch="view"] [data-value]', section)) item.setAttribute('aria-checked', String(item.dataset.value === 'grid'));
    }
    const drawGraph = mountGraph(section, () => model);
    // The section arrives after auto.js attached the page, so its own matrix is held by hand (js/footprint.js).
    footprint.attachFootprints(section);

    /** Read the hub again: loading, then the matrix and the graph, or the failed plate of each. */
    const read = async () => {
        heat.setMatrix(match, { view: 'lines', rows: [], columns: [], state: 'loading' });
        drawGraph('loading');
        try {
            model = await readUses();
        } catch {
            model = null;
            heat.setMatrix(match, { view: 'lines', rows: [], columns: [], state: 'error', message: 'Could not read who reads what' });
            drawGraph('error');
            return;
        }
        drawMatrix();
        drawGraph('ready');
    };
    // The packages' own Retry buttons fire these (heat.js MATRIX_RETRY_EVENT, graph.js GRAPH_RETRY_EVENT).
    match.addEventListener('kp-matrix-retry', () => void read());
    section.querySelector('#kyu-uses-graph')?.addEventListener('kp-graph-retry', () => void read());
    // attachGraphs draws its loading state itself; the first read follows once it has.
    requestAnimationFrame(() => void read());
}

/**
 * The hub-and-ring picture of the same subscriptions: kyu in the middle, a node per subscription, the link
 * kind = the subscription's state. Node size is the package's `weight`: unset (all the same) or the backlog.
 * @param {HTMLElement} section @param {() => ReturnType<typeof collectUses> | null} current the subscriptions last read
 * @returns {(state: 'loading' | 'ready' | 'error') => void} draw the picture in that state
 */
function mountGraph(section, current) {
    const el = /** @type {HTMLElement} */ (section.querySelector('#kyu-uses-graph'));
    const out = /** @type {HTMLElement} */ (section.querySelector('#kyu-uses-readout'));
    const KINDS = [
        { kind: 'active', label: 'Active', hint: 'The app is reading and acknowledging', style: 'solid', colour: 'var(--chart-1)' },
        { kind: 'flagged', label: 'Flagged', hint: 'A message died or a lease ran out on this subscription', style: 'dash', colour: 'var(--destructive)' },
        { kind: 'archived', label: 'Archived', hint: 'No longer reading; kept for its backlog', style: 'dot', colour: 'var(--muted-foreground)' },
    ];
    let bySize = false;
    graph.attachGraphs(section);
    /** @param {'loading' | 'ready' | 'error'} state */
    const draw = (state) => {
        if (state === 'loading') {
            graph.setGraphState(el, 'loading', 'Reading the subscriptions from kyu.');
            return;
        }
        if (state === 'error') {
            graph.setGraphState(el, 'error', 'Could not read who reads what', { text: 'kyu did not answer.' });
            return;
        }
        const subs = current()?.subs ?? [];
        if (subs.length === 0) {
            graph.setGraphState(el, 'empty', 'Nothing to draw yet: no topic has a subscription.');
            return;
        }
        const most = Math.max(1, ...subs.map((s) => s.waiting));
        graph.setGraphData(el, {
            hub: 'kyu',
            kinds: KINDS,
            nodes: [
                { id: 'kyu', label: 'kyu', description: 'the message hub', weight: bySize ? 1 : null },
                ...subs.map((s) => ({
                    id: s.id,
                    label: `${s.topic} / ${s.app}`,
                    description: `${counted(s.waiting, 'message')} waiting, ${s.note}`,
                    weight: bySize ? Math.max(0.12, s.waiting / most) : null,
                    flag: null,
                })),
            ],
            edges: subs.map((s) => ({ from: 'kyu', to: s.id, kind: s.kind, detail: `${counted(s.waiting, 'message')} waiting, ${s.note}` })),
        });
    };
    section.addEventListener('click', (event) => {
        const item = /** @type {HTMLElement | null} */ ((/** @type {Element} */ (event.target)).closest('[data-kyu-switch="size"] [data-value]'));
        if (!item) return;
        bySize = item.dataset.value === 'backlog';
        draw('ready');
    });
    el.addEventListener(graph.GRAPH_CHANGE_EVENT, (event) => {
        const { selected, hiddenKinds } = /** @type {CustomEvent} */ (event).detail;
        const name = (k) => KINDS.find((x) => x.kind === k)?.label ?? k;
        out.textContent =
            (selected.length ? `Picked: ${selected.join(', ')}.` : 'Nothing picked.') +
            (hiddenKinds.length ? ` Hidden: ${hiddenKinds.map(name).join(', ')}.` : ' Every kind of link is shown.');
    });
    return draw;
}
