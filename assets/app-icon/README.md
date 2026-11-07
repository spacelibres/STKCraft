# The STKCraft albatross

The project's mark, in the layout the Craft family uses for `assets/app-icon/`:

- `albatross.svg` — the vector master. It is image-traced: the engraving is split across two
  full-feather paths, and the blue tile is a third.
- `albatross.png` — the 512 px RGBA render. `apps/stkcraft/src/branding.rs` embeds it as the window
  icon and draws it beside the controls heading.
- `albatross.ico` — the 16–256 px Windows icon set, generated from the PNG.

Source and licence: [`ATTRIBUTION.md`](../../ATTRIBUTION.md). The mark is the project's own artwork
and is not covered by `MIT OR Apache-2.0`; a fork must replace it.

Regenerate the ICO after the PNG changes, from the repository root:

```sh
python -c "from PIL import Image; Image.open('assets/app-icon/albatross.png').convert('RGBA').save('assets/app-icon/albatross.ico', sizes=[(16,16),(24,24),(32,32),(48,48),(64,64),(128,128),(256,256)])"
```
