# df-plugin-prusa — Prusa SLA support for DragonFruit

Complex plugin for [DragonFruit](https://github.com/Open-Resin-Alliance/DragonFruit) that
adds the Original Prusa SL1, the Original Prusa SL1S SPEED and the Prusa Pro SLX, and the
print container all three read: the PrusaSlicer SLA archive saved as `.sl1`, `.sl1s` or
`.slx`.

| Machine | Build volume | Panel | Pixel pitch | File |
| --- | --- | --- | --- | --- |
| Original Prusa SL1 | 120.96 × 68.04 × 150 mm | 2560 × 1440 | 47.25 µm | `.sl1` |
| Original Prusa SL1S SPEED | 128 × 81 × 150 mm | 2560 × 1620 | 50 µm | `.sl1s` |
| Prusa Pro SLX | 220 × 125 × 400 mm | 7680 × 4320 (8K) | ≈ 28.8 µm | `.slx` |

The numbers come from Prusa's own sources: PrusaSlicer's shipped SLA profiles
(`resources/presets/prusa-research-sla/PrusaResearchSLA/preset-sla-printer.yaml`) for the
SL1 and SL1S, and Prusa's published SLX specification (220 × 125 × 400 mm, 8K panel,
≈30 µm) for the SLX. PrusaSlicer does not ship an SLX profile yet, so the SLX entry is the
first one written for it; see [docs/FORMAT.md](docs/FORMAT.md) for exactly what is assumed.

## What it provides

- **Three printer presets** with the real build volume, panel geometry, pixel pitch and
  raster orientation, so the plate, the layer grid and the exported image line up.
- **A native slicer encoder** for the Prusa SLA container, compiled into
  `dragonfruit-slicing-engine` through this repository's plugin registry entry. It takes
  the engine's RLE streaming path: layers are encoded per layer on the workers,
  rotated into the portrait image the SL1/SL1S store (the SLX takes them
  landscape, as the engine rendered them), and never held as full-image PNGs.
- **Per-machine resin pages** with Prusa's stock exposures as defaults (SL1 and SL1S) and
  the tilt speed profile selector the firmware reads from `config.ini`.

## Installing

This repository is expected to be mounted at `plugins/prusa` in DragonFruit (it is a
submodule there, like `plugins/anycubic` and `plugins/ctb`), and `prusa` must be listed in
`src/config/complex-plugin-allowlist.json`. After a checkout:

```bash
git submodule update --init plugins/prusa
npm run generate:plugin-registry
```

## What is in here

| Path | What it is |
| --- | --- |
| `pluginDefinition.ts` | Plugin id, capabilities, format registration, resin pages |
| `pluginManifest.ts` | Printer preset pack served to the profile library |
| `printers/*.json` | The three presets |
| `slicing/sl1FormatDefinition.ts` | The `.sl1` format: versions, modes, extension mapping |
| `slicing/formats.json` | Extension list the registry generator validates |
| `slicing/rust/encoder_impl.rs` | Native encoder: streaming sink, archive writer |
| `slicing/rust/sl1_config.rs` | `config.ini` rendering |
| `slicing/rust/sl1_png.rs` | Portrait rotation by column sweep, PNG assembly |
| `slicing/rust/sl1_thumbnail.rs` | The two preview images |
| `docs/FORMAT.md` | Container details and the raster-orientation derivation |

## Testing

From the DragonFruit checkout:

```bash
npm test -- plugins/prusa        # preset registry + format registration (Node test runner)
cargo test -p dragonfruit-slicing-engine prusa_encoder
```

## License

MIT. See [LICENSE](LICENSE).
