import sl1Printers from './printers/prusa-sl1.json';
import sl1sPrinters from './printers/prusa-sl1s-speed.json';
import slxPrinters from './printers/prusa-pro-slx.json';
import type { PrinterPreset } from '../../src/features/profiles/profileStore';

/**
 * The preset pack for the Prusa SLA family.
 *
 * Build volumes, display sizes and mirroring come from PrusaSlicer's own shipped
 * SLA profiles (`resources/presets/prusa-research-sla/`), which are the numbers the
 * printers are calibrated against: the SL1's 120.96 × 68.04 mm over 2560 × 1440
 * at 47.25 µm, the SL1S's 128 × 81 mm over 2560 × 1620 at 50 µm, and the SLX's
 * published 220 × 125 × 400 mm over its 8K panel.
 *
 * `mirrorY` is the raster orientation, not a hardware difference the user can see:
 * PrusaSlicer's SL1 profile stores the raster with the plate's front edge at the top
 * of the image, the SL1S/SLX profiles with it at the bottom. The encoder then
 * rotates every layer into the portrait image the printers store. See
 * `docs/FORMAT.md`.
 */
export const PRUSA_PLUGIN_MANIFEST = {
  schemaVersion: 1,
  id: 'prusa-builtin',
  name: 'Prusa Research Plugin',
  version: '0.1.0',
  description: 'Original Prusa SL1, SL1S SPEED and Prusa Pro SLX profiles and the Prusa SLA print container.',
  author: 'Open Resin Alliance',
  homepage: 'https://github.com/Open-Resin-Alliance/df-plugin-prusa',
  printerPresets: [
    ...sl1Printers,
    ...sl1sPrinters,
    ...slxPrinters,
  ] as PrinterPreset[],
  materialTemplates: [],
};
