// kyu: topic.js, the Topic page (/t/{topic}/dashboard) on kp-themes 11.0.0. The entry: it imports
// the page's modules (topic-*.js, one per region), paints once, boots the kit, then starts reading.
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

import { dataTable } from '/static/kp/js/datatable.js';
import { masterDetail, setPaneState } from '/static/kp/js/master-detail.js';
import { $, $$ } from './topic-util.js';
import { S, SUBS } from './topic-state.js';
import { renderHeader } from './topic-header.js';
import { renderCommands, startOver } from './topic-send.js';
import { renderReaders } from './topic-readers.js';
import { renderSubs, updateCounts, fixTooltips, subsBody, subsTableEl } from './topic-subs.js';
import './topic-subs-menu.js';
import { renderDead, deadTableEl } from './topic-dead.js';
import { renderProblems } from './topic-problems.js';
import './topic-dead-actions.js';
import { renderRecent } from './topic-recent.js';
import { mdRoot, fillDetail } from './topic-detail.js';
import './topic-publish.js';
import { load, reload, readRetention } from './topic-refresh.js';

for (const el of [subsTableEl, deadTableEl]) el.addEventListener('kp-datatable-retry', () => void reload(true));

// ---------------------------------------------------------------- first paint, then the kit's boot

renderHeader();
renderCommands();
renderReaders();
SUBS.clear();
for (const s of S.data.subscriptions) SUBS.set(s.name, { ...s });
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
S.attachAll = (/** @type {ParentNode} */ r) => auto.attachAll(r);
await autoReady;
S.booted = true;

S.subsTable = dataTable(subsTableEl);
S.deadTable = dataTable(deadTableEl);
S.subsTable?.density('comfortable');
fixTooltips($$('tr', subsBody), false);
updateCounts();

{
    const md = masterDetail(mdRoot);
    if (md && S.data.messages.length) {
        md.select(S.data.messages[0].id);
        fillDetail(S.data.messages[0].id);
        setPaneState(mdRoot, 'ready');
    }
}

// The first read is a real one, with the components' own loading and failed states: the page's server-drawn rows
// stand behind the busy card until kyu has answered.
void reload(true);
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
