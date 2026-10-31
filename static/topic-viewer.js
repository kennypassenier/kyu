// kyu: topic-viewer.js, the large payload viewer of a recent message (JSON laid out, copy, download).

import { viewerExports, flashExports, codeExports } from '/static/kp/dist/kp-themes.min.js';
import { make, dmy, bytesText } from './topic-util.js';
const { openViewer } = viewerExports;
const { runFlashButton } = flashExports;
const { setCode } = codeExports;

let viewerOpen = /** @type {import('/static/kp/js/viewer.js').ViewerHandle | null} */ (null);

/** The viewer's Copy button: copies the payload and flashes the outcome. @param {import('/static/kp/js/viewer.js').ViewerHandle} viewer @param {string} text */
function copyPayload(viewer, text) {
    const b = viewer.foot.querySelector('[data-kp-viewer-action="copy"]');
    if (!(b instanceof HTMLElement)) return;
    b.setAttribute('data-kp-flash-words', 'Copied,Refused');
    void runFlashButton(b, async () => {
        try {
            await navigator.clipboard.writeText(text);
            return 'Copied';
        } catch {
            return { error: 'Your browser blocked copying.', remedy: 'Select the text and press Ctrl+C.' };
        }
    });
}

/** @param {any} m */
export function openPayload(m, opener = null) {
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
                run: () => copyPayload(viewer, p.text),
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
