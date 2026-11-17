import assert from 'node:assert/strict';
import test from 'node:test';

import {
  addPrinterProfileFromPreset,
  getMaterialProfilesForPrinter,
  getProfileStoreSnapshot,
} from '@/features/profiles/profileStore';
import { assembleSliceJob } from '@/features/slicing/sliceJobAssembly';
import { resolveOutputFileExtension } from '@/features/slicing/formats/registry';

/**
 * The preset and the encoder only meet through `assembleSliceJob`: if the format
 * registration or the resin page is wrong, the job comes out at another format,
 * another size, or with no `prusa.*` keys for the encoder to read. This pins the
 * job the app would hand the engine for an SL1S.
 */
test('an SL1S preset assembles the job the Prusa encoder reads', () => {
  const printerId = addPrinterProfileFromPreset('prusa-sl1s-speed');
  const material = getMaterialProfilesForPrinter(printerId)[0];
  assert.ok(material, 'the preset should come with a material profile');

  // The store resolves the preset's archived build volume from its panel
  // geometry, which is what the slicer must see; a raw preset would be wrong.
  const printer = getProfileStoreSnapshot().printerProfiles.find((entry) => entry.id === printerId);
  assert.ok(printer, 'the preset should be in the store');
  assert.deepEqual(printer.buildVolumeMm, { width: 128, depth: 81, height: 150 });

  const job = assembleSliceJob({
    printerProfile: printer,
    materialProfile: material,
    scene: { totalLayers: 2, tallestObjectHeightMm: 0.1, models: [] },
  });

  assert.equal(job.outputFormat, '.sl1');
  assert.equal(job.sourceWidthPx, 2560);
  assert.equal(job.sourceHeightPx, 1620);
  assert.equal(job.mirrorY, true);
  assert.equal(job.formatVersion, 'sl1s');
  assert.equal(job.layerDataKind, 'raw-mask');
  assert.equal(resolveOutputFileExtension(job.outputFormat, job.formatVersion), 'sl1s');

  const metadata = JSON.parse(job.metadataJson) as {
    material?: Record<string, unknown>;
    prusa?: Record<string, unknown>;
    printer?: Record<string, unknown>;
  };
  assert.equal(metadata.printer?.formatVersion, 'sl1s');
  assert.equal(metadata.material?.layerHeightMm, 0.05);
  assert.equal(metadata.material?.normalExposureSec, 2.2);
  assert.equal(metadata.material?.bottomExposureSec, 6);
  assert.equal(metadata.material?.bottomLayerCount, 10);
  // The encoder reads the tilt profile from `prusa.*`, which only exists because
  // the resin page declares it.
  assert.equal(metadata.prusa?.expUserProfile, '0');
});
