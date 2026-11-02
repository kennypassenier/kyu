// kyu: topic-reconcile.js, keeps a table body in step with a list by key (add, update or replace what changed, remove what left).

const sigOf = (/** @type {unknown} */ value) => JSON.stringify(value);

/** Keep a table body in step with a list by key: add, replace what changed, remove what left.
 * A `patch` updates a changed row in place, so its focus, open menu and open confirmation survive; without one the row is rebuilt.
 * @param {HTMLElement} body @param {any[]} items @param {(item: any) => string} keyOf @param {(item: any) => HTMLElement} build
 * @param {(row: HTMLElement, item: any) => void} [patch] */
export function reconcile(body, items, keyOf, build, patch) {
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
        } else if (row.dataset.ktSig !== sig && patch) {
            patch(row, item);
            row.dataset.ktSig = sig;
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
