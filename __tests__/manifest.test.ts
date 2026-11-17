import assert from 'node:assert/strict';
import test from 'node:test';

import { PRUSA_PLUGIN_MANIFEST } from '../pluginManifest';
import PRUSA_COMPLEX_PLUGIN_DEFINITION from '../pluginDefinition';
import { getRuntimePrinterPresets, getProfileLocalMaterialSettingsAdapter } from '@/features/plugins/pluginRegistry';
import type { PrinterPreset } from '@/features/profiles/profileStore';
import {
  resolveOutputFileExtension,
  resolveSlicingFormatDefinitionByOutput,
} from '@/features/slicing/formats/registry';

/**
 * These presets are only usable if the registry's sanitizer keeps the geometry:
 * a dropped build volume or resolution silently produces a printer that
 * mis-scales every model, and a missing format version exports a `.sl1` for a
 * machine that wants a `.slx`.
 */
const EXPECTED = [
  {
    presetId: 'prusa-sl1',
    name: 'Original Prusa SL1',
    buildVolumeMm: { width: 120.96, depth: 68.04, height: 150 },
    pixelSize: { x: 47.25, y: 47.25 },
    resolution: { x: 2560, y: 1440 },
    formatVersion: 'sl1',
    settingsMode: 'sl1',
    mirrorY: false,
    extension: '.sl1',
    asset: 'prusa-sl1.png',
  },
  {
    presetId: 'prusa-sl1s-speed',
    name: 'Original Prusa SL1S SPEED',
    buildVolumeMm: { width: 128, depth: 81, height: 150 },
    pixelSize: { x: 50, y: 50 },
    resolution: { x: 2560, y: 1620 },
    formatVersion: 'sl1s',
    settingsMode: 'sl1s',
    mirrorY: true,
    extension: '.sl1s',
    asset: 'prusa-sl1s.png',
  },
  {
    presetId: 'prusa-pro-slx',
    name: 'Prusa Pro SLX',
    buildVolumeMm: { width: 220, depth: 125, height: 400 },
    pixelSize: { x: 28.65, y: 28.94 },
    resolution: { x: 7680, y: 4320 },
    formatVersion: 'slx',
    settingsMode: 'slx',
    // The SLX's layers are the plate as-is (PrusaSlicer's SLX profile is
    // landscape), unlike the portrait SL1/SL1S.
    mirrorY: false,
    extension: '.slx',
    asset: 'prusa-slx.png',
  },
];

test('the plugin exposes one preset per machine with its real geometry', () => {
  const presets = getRuntimePrinterPresets([]);
  for (const expected of EXPECTED) {
    const preset = presets.find((entry: PrinterPreset) => entry.presetId === expected.presetId);
    assert.ok(preset, `${expected.presetId} missing from the runtime presets`);
    assert.equal(preset.name, expected.name);
    assert.deepEqual(preset.buildVolumeMm, expected.buildVolumeMm);
    assert.deepEqual(preset.pixelSize, expected.pixelSize);
    assert.equal(preset.display.resolutionX, expected.resolution.x);
    assert.equal(preset.display.resolutionY, expected.resolution.y);
    assert.equal(preset.display.outputFormat, '.sl1');
    assert.equal(preset.display.formatVersion, expected.formatVersion);
    assert.equal(preset.display.settingsMode, expected.settingsMode);
    assert.equal(preset.display.mirrorY, expected.mirrorY);
    assert.equal(preset.display.mirrorX, false);
    // The runtime registry keys plugin assets by the plugin version, so a replaced
    // image is a new URL and a webview cache cannot pin the old one.
    assert.equal(
      preset.imageAssetPath,
      `/api/profile-assets/plugins/prusa/printers/assets/${expected.asset}?v=${PRUSA_PLUGIN_MANIFEST.version}`,
    );

    // Pitch x pixels must match the advertised volume; a mismatch means one was
    // changed without the other, which mis-scales every print.
    const widthMm = preset.display.resolutionX * (expected.pixelSize.x / 1000);
    const depthMm = preset.display.resolutionY * (expected.pixelSize.y / 1000);
    assert.ok(Math.abs(widthMm - expected.buildVolumeMm.width) < 0.05, `${expected.presetId} width ${widthMm}mm`);
    assert.ok(Math.abs(depthMm - expected.buildVolumeMm.depth) < 0.05, `${expected.presetId} depth ${depthMm}mm`);
  }
});

test('each machine exports the extension PrusaSlicer uses for it', () => {
  const presets = getRuntimePrinterPresets([]);
  for (const expected of EXPECTED) {
    const preset = presets.find((entry: PrinterPreset) => entry.presetId === expected.presetId);
    assert.ok(preset);
    assert.equal(
      `.${
        resolveOutputFileExtension(preset.display.outputFormat, preset.display.formatVersion)
      }`,
      expected.extension,
    );
  }
});

test('the format is a plugin-owned streaming format with per-machine modes', () => {
  const format = resolveSlicingFormatDefinitionByOutput('.sl1');
  assert.ok(format, '.sl1 format not registered');
  assert.equal(format.ownership, 'plugin');
  assert.equal(format.pluginId, 'prusa');
  assert.equal(format.layerDataKind, 'raw-mask');
  assert.equal(format.fileExtensionFromVersion, true);
  assert.deepEqual(
    format.formatVersions?.map((version: { value: string }) => version.value),
    ['sl1', 'sl1s', 'slx'],
  );
  assert.deepEqual(
    format.settingsModes?.map((mode: { value: string }) => mode.value),
    ['sl1', 'sl1s', 'slx'],
  );
});

test('each machine has its own resin defaults', () => {
  const fieldValue = (mode: string, key: string) =>
    getProfileLocalMaterialSettingsAdapter('.sl1', mode)?.fields.find((field: { key: string }) => field.key === key)
      ?.defaultValue;

  assert.equal(fieldValue('sl1', 'normalExposureSec'), 12);
  assert.equal(fieldValue('sl1', 'bottomExposureSec'), 45);
  assert.equal(fieldValue('sl1s', 'normalExposureSec'), 2.2);
  assert.equal(fieldValue('sl1s', 'bottomExposureSec'), 6);
  // SLX has no published exposure data yet, so it inherits the SL1S page.
  assert.equal(fieldValue('slx', 'normalExposureSec'), 2.2);
});

test('the manifest declares the encoder and its presets', () => {
  assert.equal(PRUSA_PLUGIN_MANIFEST.id, 'prusa-builtin');
  assert.equal(PRUSA_COMPLEX_PLUGIN_DEFINITION.id, 'prusa');
  assert.equal(PRUSA_COMPLEX_PLUGIN_DEFINITION.capabilities?.slicerEncoder, true);
  assert.deepEqual(
    PRUSA_PLUGIN_MANIFEST.printerPresets.map((preset) => preset.presetId),
    EXPECTED.map((expected) => expected.presetId),
  );
});
