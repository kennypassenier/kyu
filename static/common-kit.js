// kyu: common-kit.js, the kit modules the shared parts use, imported once (top-level await) for the others to share.

const KP = '/static/kp/js/';

export const [palette, shortcuts, anchors, keys, tour, overlays, components, heat, graph, segmented, footprint] = await Promise.all([
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
