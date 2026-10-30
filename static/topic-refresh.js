// kyu: topic-refresh.js, reading the page again: fetch the data block, redraw every section, and the failed/loading states.

import { setPaneState } from '/static/kp/js/master-detail.js';
import { S, T } from './topic-state.js';
import { renderHeader } from './topic-header.js';
import { renderReaders } from './topic-readers.js';
import { renderSubs } from './topic-subs.js';
import { renderDead } from './topic-dead.js';
import { renderRecent } from './topic-recent.js';
import { mdRoot, setDetailCode, detailFig } from './topic-detail.js';

/** Read the retention of dead letters once; a hub without the route answers 404 and the problem is left out. */
export async function readRetention() {
    try {
        const res = await fetch(`${T}/retention`, { headers: { Accept: 'application/json' }, credentials: 'same-origin' });
        if (!res.ok) return;
        const json = await res.json();
        if (Number.isFinite(json.dead_letter_days) && json.dead_letter_days > 0) S.retentionDays = json.dead_letter_days;
    } catch (error) {
        // No retention: the "oldest dead letter" problem is left out.
        console.debug('kyu: the dead-letter retention could not be read', error);
    }
}

/** Fetch this page again and take its data block. */
export async function load() {
    if (S.pending > 0) return;
    const res = await fetch(location.pathname + location.search, { headers: { Accept: 'text/html' }, credentials: 'same-origin' });
    if (!res.ok) throw new Error(`the page answered ${res.status}`);
    const doc = new DOMParser().parseFromString(await res.text(), 'text/html');
    const block = doc.getElementById('kyu-topic-data');
    if (!block) throw new Error('the page had no data');
    if (S.pending > 0) return;
    S.data = JSON.parse(block.textContent ?? '{}');
    apply();
}

/** Resolve once the payload block shows its failed plate (it draws after its loading card has closed), or after 1 s. */
function codePlate() {
    const fig = detailFig();
    if (!fig || fig.getAttribute('data-kp-state') === 'error') return Promise.resolve();
    return new Promise((resolve) => {
        const watch = new MutationObserver(() => {
            if (fig.getAttribute('data-kp-state') !== 'error') return;
            watch.disconnect();
            resolve(undefined);
        });
        watch.observe(fig, { attributes: true, attributeFilter: ['data-kp-state'] });
        setTimeout(() => { watch.disconnect(); resolve(undefined); }, 1000);
    });
}

let readFailed = false;
const READ_FAILED = 'kyu did not answer, so this page may be out of date.';
/**
 * Read the page again with the components' own states: `visible` lays the busy card over the tables and the
 * detail pane while it reads (a Retry); a failure turns every table and the detail pane into their failed state
 * with Try again. The automatic refresh does neither: it keeps the stale data and the refresh control says so.
 * @param {boolean} [visible]
 */
export async function reload(visible = false) {
    const tables = [S.subsTable, S.deadTable];
    if (visible) {
        for (const t of tables) t?.state('loading');
        setPaneState(mdRoot, 'loading');
        setDetailCode('loading');
    }
    try {
        await load();
    } catch {
        readFailed = true;
        for (const t of tables) t?.fail(READ_FAILED);
        // The payload block draws its own failed plate first and the pane's plate yields to it (kp-themes 11.2.0).
        // The block closes its loading card before it draws, so wait for its plate; where the block is not on screen
        // (a phone, nothing opened yet) the pane's plate says it.
        setPaneState(mdRoot, 'ready');
        setDetailCode('error', READ_FAILED);
        await codePlate();
        setPaneState(mdRoot, 'error', { title: 'Could not read the messages', text: READ_FAILED });
        return;
    }
    if (visible || readFailed) {
        readFailed = false;
        for (const t of tables) t?.state('ready');
        setPaneState(mdRoot, 'ready');
        setDetailCode('ready');
    }
}
/** Redraw every section from the data block just read. */
function apply() {
    renderHeader();
    renderReaders();
    renderSubs();
    S.subsTable?.refresh();
    renderDead();
    renderRecent();
}
