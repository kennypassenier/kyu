// kyu: topics.js, the Topics page on kp-themes 11.0.0. The entry: it imports the page's modules
// (topics-*.js, one per concern), renders once and starts the history sections.
//
//   1. the data: the server renders every topic as one JSON block inside the
//      key-figure group (`topics_json`); the automatic refresh fetches this
//      same page and swaps that group, so each refresh brings a fresh block;
//   2. the drawing from that data with the package's own functions
//      (setIdmark, setBarCell, setStackbar, setAgo, setAttention); nothing
//      here draws a component itself;
//   3. the two history sections, which read GET /dashboard/stats/hourly and
//      GET /dashboard/stats/totals and draw them with setHeat and setTrendData.

import { $ } from './topics-util.js';
import { M, ui, takeData, decideSlots } from './topics-state.js';
import './topics-swap.js';
import './topics-filters.js';
import { paintAll } from './topics-paint.js';
import { fillRows } from './topics-table.js';
import { drawHeat, loadHeat } from './topics-heat.js';
import { drawTrends, loadTrends } from './topics-trends.js';

// All of it is in the markup before auto.js attaches the components.
{
    const data = takeData();
    if (data) M.model = data;
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
