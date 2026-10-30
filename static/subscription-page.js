// kyu: subscription-page.js, reading the Subscription page again: the policy, the gauges and the backlog show one state together.

import { agoMoment } from '/static/kp/js/freshness.js';
import { dataTable } from '/static/kp/js/datatable.js';
import { setCardState, CARD_RETRY_EVENT } from '/static/kp/js/card-state.js';
import { setCode } from '/static/kp/js/code.js';
import { $, $$ } from './subscription-util.js';
import { items, P } from './subscription-state.js';
import { drawGauges, drawCounts } from './subscription-gauges.js';
import { swapRows } from './subscription-backlog.js';
import { initUptime } from './subscription-uptime.js';

const policyCard = () => /** @type {HTMLElement | null} */ ($('#ky-policy'));
const backlogTable = () => {
    const el = $('#ky-backlog-table');
    return el ? dataTable(el) : null;
};
const PAGE_FAILED = 'kyu did not answer, so this page may be out of date.';

/** The policy card, the gauges and the backlog table are one read of the page: they show its state together. @param {'loading' | 'ready' | 'error'} state */
function showPageState(state) {
    P.pageState = state;
    const card = policyCard();
    if (card) {
        if (state === 'error') setCardState(card, 'error', { title: 'Could not read the policy', text: 'kyu did not answer.' });
        else setCardState(card, state);
    }
    const table = backlogTable();
    // The payload cells wait with the rows (a cell code view is skeleton while it loads).
    if (state !== 'ready') for (const fig of $$('#ky-backlog-table [data-kp-code]')) setCode(fig, { cell: true, state, message: state === 'error' ? PAGE_FAILED : undefined });
    if (state === 'loading') {
        table?.state('loading');
    } else if (state === 'error') table?.fail(PAGE_FAILED);
    else table?.state('ready');
    drawGauges();
}

/** Read this page again: its data block, the policy and the backlog rows. @returns {Promise<boolean>} whether kyu answered */
export async function readPage() {
    showPageState('loading');
    try {
        const response = await fetch(location.pathname + location.search, { credentials: 'same-origin', headers: { Accept: 'text/html' } });
        if (!response.ok) throw new Error(`the page answered ${response.status}`);
        const doc = new DOMParser().parseFromString(await response.text(), 'text/html');
        const block = doc.getElementById('ky-page-data');
        if (!block) throw new Error('the page had no data');
        const fresh = JSON.parse(block.textContent ?? '{}');
        items.clear();
        for (const item of fresh.items ?? []) items.set(item.id, item);
        const policy = $('#ky-policy .kp-kv', doc);
        const here = $('#ky-policy .kp-kv');
        if (policy && here) {
            here.replaceWith(document.importNode(policy, true));
            for (const el of $$('#ky-policy [data-ky-moment]')) el.textContent = agoMoment(Number(el.dataset.kyMoment));
        }
        swapRows(doc);
        drawCounts();
    } catch {
        showPageState('error');
        return false;
    }
    showPageState('ready');
    return true;
}
for (const id of ['ky-bullets-age', 'ky-bullets-attempts']) document.getElementById(id)?.addEventListener('kp-bullets-retry', () => void readPage());
policyCard()?.addEventListener(CARD_RETRY_EVENT, () => void readPage());
$('#ky-backlog-table')?.addEventListener('kp-datatable-retry', () => void readPage());
$('#ky-drain')?.addEventListener('kp-progressbar-retry', () => void (P.pageState === 'error' ? readPage() : initUptime()));
