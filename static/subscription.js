// kyu: subscription.js, the Subscription page on kp-themes 11.0.0. The entry: it names kyu's words on the
// uptime strip and starts the page's modules (subscription-*.js, one per region) once the kit has booted.
//
// The server renders the header, the policy and the backlog table (so the page
// reads without a script); this file brings the rest to life: the marks and
// freshness line, the gauges, bulk delete with Undo, the attempts timeline and
// request inspector for one message, and the 90-day strip. The data of the
// page is the JSON block `#ky-page-data`; the rest is fetched from the
// dashboard routes under `/t/{topic}/dashboard/subs/{name}`.
import { stringsExports } from '/static/kp/dist/kp-themes.min.js';
import { onReady } from './subscription-util.js';
import { P } from './subscription-state.js';
import { initHeader } from './subscription-header.js';
import { drawGauges } from './subscription-gauges.js';
import { initBacklog } from './subscription-backlog.js';
import { wireAttempts, holdAttempts, startAttempts } from './subscription-attempts.js';
import { initUptime } from './subscription-uptime.js';
import './subscription-page.js';
const { setStrings } = stringsExports;


// kyu's own words on the uptime strip: a day is judged by what was delivered, not by what was up.
const STATE_WORDS = /** @type {Record<string, string>} */ ({ up: 'all delivered', degraded: 'some dead', down: 'mostly dead' });
setStrings({
    uptimeIncidents: (n) => `${n} dead`,
    uptimeSummary: (percent, n) => `${percent}% delivered, ${n} dead`,
    uptimeStateWord: (state) => STATE_WORDS[state] ?? 'no messages',
    uptimeCell: (label, state, percent, n) =>
        [label, STATE_WORDS[state] ?? 'no messages', percent === null ? null : `${percent} delivered`, n === null ? null : `${n} dead`]
            .filter(Boolean)
            .join(' · '),
    uptimeInfoFigures: (percent, n) =>
        [percent === null ? null : `${percent}% delivered`, n === null ? null : `${n} dead`].filter(Boolean).join(' · '),
});

onReady(() => {
    initHeader();
    P.pageState = 'loading';
    drawGauges();
    initBacklog();
    wireAttempts();
    holdAttempts();
    void initUptime();
    // The first read is a real one with the components' own states: the server-drawn page stands behind the busy
    // cards until kyu has answered; the attempts wait for it, since the backlog it reads says which message to show.
    void startAttempts();
});
