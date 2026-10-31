// kyu: common-kit.js, the kit modules the shared parts use, read from the one kp-themes bundle for the others to share.

export {
    paletteExports as palette,
    shortcutsExports as shortcuts,
    anchorsExports as anchors,
    keysExports as keys,
    tourExports as tour,
    overlaysExports as overlays,
    componentsExports as components,
    heatExports as heat,
    graphExports as graph,
    segmentedExports as segmented,
    footprintExports as footprint,
} from '/static/kp/dist/kp-themes.min.js';
