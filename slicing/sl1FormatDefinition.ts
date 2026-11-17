import type { SlicingFormatDefinition } from '@/features/slicing/formats/types';

/**
 * The Prusa SLA container (`.sl1`), which SL1, SL1S and SLX firmware all read.
 *
 * The three machines differ only in the name they expect in `config.ini`
 * (`printerModel`) and in the extension PrusaSlicer gives the file, so the format
 * declares one canonical output and takes the machine from the selected
 * `formatVersion`, which `fileExtensionFromVersion` also turns into the file
 * extension: `sl1` → `.sl1`, `sl1s` → `.sl1s`, `slx` → `.slx`.
 *
 * `raw-mask` is what puts the engine on its streaming RLE path: layers arrive as
 * runs and are PNG-encoded per layer on the workers, never as full engine-side
 * PNG layers. The encoder rotates each raster to the portrait orientation the
 * printer stores (see `slicing/rust/encoder_impl.rs`).
 */
export const PRUSA_SL1_FORMAT_DEFINITION: SlicingFormatDefinition = {
  id: 'prusa.sl1.v1',
  outputFormat: '.sl1',
  displayName: 'Prusa SLA',
  ownership: 'plugin',
  layerDataKind: 'raw-mask',
  pluginId: 'prusa',
  fileExtensionFromVersion: true,
  formatVersions: [
    { value: 'sl1', label: 'Original Prusa SL1 (.sl1)', isDefault: true },
    { value: 'sl1s', label: 'Original Prusa SL1S SPEED (.sl1s)' },
    { value: 'slx', label: 'Prusa Pro SLX (.slx)' },
  ],
  settingsModes: [
    { value: 'sl1', label: 'Original Prusa SL1', isDefault: true },
    { value: 'sl1s', label: 'Original Prusa SL1S SPEED' },
    { value: 'slx', label: 'Prusa Pro SLX' },
  ],
  rustModulePath: 'formats::sl1',
  wasmExportName: 'encode_sl1_container',
  notes:
    'ZIP archive of 8-bit grayscale layer PNGs plus config.ini; the same container PrusaSlicer writes for the SL1, SL1S and SLX.',
};
