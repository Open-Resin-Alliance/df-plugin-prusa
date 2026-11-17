# The Prusa SLA container

`.sl1`, `.sl1s` and `.slx` are one format: the ZIP archive PrusaSlicer's
`store_sl1` writes (`Slic3r/Biz/ResultExport/SLA/SL1.cpp` in PrusaSlicer). The
machine is named inside `config.ini`; the extension only says which machine the
file was prepared for.

```
archive
├── config.ini                         key = value, sorted, LF endings
├── <jobDir>00000.png                  layer 0, 8-bit grayscale, portrait
├── <jobDir>00001.png                  layer 1, …
├── …                                  one per layer, 5-digit zero-based index
├── thumbnail/thumbnail400x400.png     RGBA preview
└── thumbnail/thumbnail800x480.png     RGBA preview
```

Every entry is deflated inside the ZIP, layer images included, which is what
PrusaSlicer's own writer does. A layer the run writer produced is a fixed-Huffman
deflate stream made almost entirely of repeated distance-1 matches, so the second
pass costs a few milliseconds and shrinks one 8K layer from 216 KB to 2.9 KB.

The rotation into the portrait image is a sweep over the source columns, and the
column's value vector is kept as a run list over the source rows: it can only
change where a source span starts or ends, so the sweep patches the run list at
those boundaries instead of re-reading every pixel. At 8K a binary layer's runs
take about a millisecond to transpose and two to write, which is the same order
as the engine's other run-based formats.

PrusaSlicer additionally writes `config.json` (its tilt/exposure profile) and
`prusaslicer.ini` / `prusaslicer.json` (its full configuration dump). The firmware
does not need them, and UVtools has written printer-accepted files without any of
the four, which is why this encoder writes only `config.ini` and the images:
`config.json` describes motion curves that this plugin does not model, and
inventing them would change how the vat tilts.

## Layer images

A layer PNG is exactly one display frame at the machine's raster resolution, 8-bit
grayscale, non-interlaced, where `255` solidly cures and `0` does not, and the
values between carry anti-aliasing. PrusaSlicer writes them portrait:

| Machine | File dimensions | Pixel pitch |
| --- | --- | --- |
| SL1 | 1440 × 2560 | 47.25 µm |
| SL1S | 1620 × 2560 | 50 µm |
| SLX | 4320 × 7680 | ≈ 28.8 µm |

DragonFruit's engine rasterises the plate landscape, in the printer profile's
`resolutionX × resolutionY`, with the plate's front edge at the bottom of the
image. PrusaSlicer's rasterizer rotates the plate a quarter turn and applies the
mirroring its printer profile declares (`Trafo` in
`src/libslic3r/src/libslic3r/SLA/RasterBase.hpp`):

- the SL1 profile declares no mirror, so the portrait image is a quarter turn
  counter-clockwise of DragonFruit's landscape frame with `mirrorY` **off**;
- the SL1S profile declares `display_mirror_x = 1`, which cancels the implicit X
  mirror the portrait rotation adds, i.e. one further flip of the landscape frame:
  `mirrorY` **on**;
- the SLX has no shipped PrusaSlicer profile. It is treated like the SL1S — the
  machine it succeeds — until Prusa publishes one.

That is why `mirrorY` differs between the presets: it is the rotation convention,
not a hardware difference the user can see. The encoder always rotates a quarter
turn counter-clockwise; `mirrorX` stays off everywhere.

## `config.ini`

The keys PrusaSlicer and UVtools both write, and the ones this encoder writes:

| Key | Value |
| --- | --- |
| `action` | `print` |
| `expTime` | normal layer exposure, seconds |
| `expTimeFirst` | initial (first-layer) exposure, seconds |
| `expUserProfile` | `0` fast, `1` slow, `2` custom tilt profile |
| `fileCreationTimestamp` | `YYYY-MM-DD at HH:MM:SS UTC` |
| `hollow` | `0` (hollowing is baked into the mesh) |
| `jobDir` | archive name; also the layer-entry prefix |
| `layerHeight` | layer step, mm |
| `numFade` | layers fading from `expTimeFirst` to `expTime`, capped at the firmware's 20 |
| `numFast` | layers in the firmware's fast tilt profile; every layer here, since the tilt is not modelled |
| `numSlow` | `0` |
| `printTime` | `0`, unknown; the firmware runs its own motion profile |
| `printerModel` | `SL1`, `SL1S` or `SLX` |
| `printerVariant` | `default` |
| `prusaSlicerVersion` | writer identity, `DragonFruit <version>` |
| `usedMaterial` | cured volume, ml, summed from each layer's measured solid area |
| `materialName` | the material profile's name, when it has one |

`jobDir` and the layer names must agree, so the archive is named after the output
file's stem (sanitised to ASCII letters, digits, `-` and `_`; `print` when there is
no stem, as on the in-memory encode path).

## What this plugin does not do

- **Import.** A sliced archive carries raster layers, not geometry, so there is
  nothing to turn back into a scene. DragonFruit imports `.sl1` files by extracting
  them in a viewer, not as a scene format.
- **Shell thumbnails** (`outputFileTypes.json`). The native thumbnail providers read
  a chunk-table grammar; a ZIP's preview is behind a central directory they cannot
  describe yet, so `.sl1` files keep the generic icon.
- **Tilt/exposure profiles** (`config.json`). See above: not modelled, not invented.

## Sources

- PrusaSlicer, `src/slic3r-shared/src/Slic3r/Biz/ResultExport/SLA/SL1.cpp` — the
  writer this archive matches, including the layer naming and `config.ini` keys.
- PrusaSlicer, `resources/presets/prusa-research-sla/PrusaResearchSLA/preset-sla-printer.yaml`
  — SL1/SL1S display geometry, mirroring and build height.
- PrusaSlicer, `src/libslic3r/src/libslic3r/SLA/RasterBase.hpp` — the raster
  orientation and mirror defaults the derivation above follows.
- UVtools, `UVtools.Core/FileFormats/SL1File.cs` — an independent writer/reader of
  the same container, useful for confirming which keys the firmware tolerates.
- Prusa's SLX product page — 220 × 125 × 400 mm, 8K monochrome LCD, ≈30 µm XY.
