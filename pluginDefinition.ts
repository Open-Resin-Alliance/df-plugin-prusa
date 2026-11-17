import type {
  ComplexPluginDefinition,
  MaterialSettingsSource,
  PluginLocalMaterialSettingsAdapterContract,
} from '@/features/plugins/complexPluginContracts';
import { resolveDifferentialMaterialSettings } from '@/features/plugins/resolveDifferentialSettings';
import { PRUSA_PLUGIN_MANIFEST } from './pluginManifest';
import { PRUSA_SL1_FORMAT_DEFINITION } from './slicing/sl1FormatDefinition';
import sl1MaterialSettings from './materialSettings/settings_sl1.json';
import sl1sMaterialSettings from './materialSettings/settings_sl1s.json';
import slxMaterialSettings from './materialSettings/settings_slx.json';

/**
 * One output format, three machines.
 *
 * The SL1, SL1S and SLX all read the same container; what differs is the
 * `printerModel` the file names and the display geometry the preset carries. The
 * format's `settingsModes` mirror that split so each machine's resin page starts
 * from its own stock exposures rather than one machine's defaults applied to all
 * three. The SLX mode inherits the SL1S page until Prusa publishes SLX exposure
 * data - a wrong starting exposure is worse than an inherited one that the user
 * adjusts.
 */
const PRUSA_MODE_SOURCES: Record<string, MaterialSettingsSource> = {
  sl1: sl1MaterialSettings as MaterialSettingsSource,
  sl1s: sl1sMaterialSettings as MaterialSettingsSource,
  slx: slxMaterialSettings as MaterialSettingsSource,
};

function modeAdapter(mode: string): PluginLocalMaterialSettingsAdapterContract {
  return {
    outputFormat: PRUSA_SL1_FORMAT_DEFINITION.outputFormat,
    ...resolveDifferentialMaterialSettings(PRUSA_MODE_SOURCES[mode], PRUSA_MODE_SOURCES),
  };
}

const PRUSA_SL1_SETTINGS = modeAdapter('sl1');
const PRUSA_SL1S_SETTINGS = modeAdapter('sl1s');
const PRUSA_SLX_SETTINGS = modeAdapter('slx');

export const PRUSA_COMPLEX_PLUGIN_DEFINITION: ComplexPluginDefinition = {
  id: 'prusa',
  manifest: PRUSA_PLUGIN_MANIFEST,
  capabilities: {
    networkOperations: false,
    uploadWithProgress: false,
    slicerEncoder: true,
    tauriRuntimePlugin: false,
  },
  slicingFormatsByOutput: {
    [PRUSA_SL1_FORMAT_DEFINITION.outputFormat]: PRUSA_SL1_FORMAT_DEFINITION,
  },
  localMaterialSettingsByOutput: {
    [PRUSA_SL1_FORMAT_DEFINITION.outputFormat]: PRUSA_SL1_SETTINGS,
  },
  localMaterialSettingsByOutputAndMode: {
    [PRUSA_SL1_FORMAT_DEFINITION.outputFormat]: {
      sl1: PRUSA_SL1_SETTINGS,
      sl1s: PRUSA_SL1S_SETTINGS,
      slx: PRUSA_SLX_SETTINGS,
    },
  },
};

export default PRUSA_COMPLEX_PLUGIN_DEFINITION;
