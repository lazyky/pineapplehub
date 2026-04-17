# Assets

Static files embedded into the WASM binary at compile time via `include_bytes!`.

| File | Purpose | Source |
|------|---------|--------|
| `favicon.png` | App icon shown in title bar and browser tab | Custom design |
| `github-mark.png` | GitHub logo button (RGBA, transparent bg, 64×64) | [GitHub Logos](https://github.com/logos) |
| `material-symbols.ttf` | Icon font subset (Material Symbols Outlined) | See `src/icons.rs` for regeneration steps |
| `NotoSansSC-Regular.ttf` | CJK text rendering (Simplified Chinese) | [Google Fonts](https://fonts.google.com/noto/specimen/Noto+Sans+SC) |
| `_headers` | HTTP headers for Trunk dev server (COOP/COEP for SharedArrayBuffer) | Manual |

## Notes

- **`material-symbols.ttf`** is a pyftsubset-generated subset (~25 KB) of the
  full Material Symbols Outlined variable font (~10 MB). Regenerate with:
  ```sh
  # 1. Download full font
  curl -sL -o /tmp/MaterialSymbolsOutlined.ttf \
    "https://github.com/google/material-design-icons/raw/master/variablefont/MaterialSymbolsOutlined%5BFILL%2CGRAD%2Copsz%2Cwght%5D.ttf"

  # 2. Look up new codepoints (use fontTools, NOT the legacy .codepoints file)
  python3 -c "
  from fontTools.ttLib import TTFont
  cmap = TTFont('/tmp/MaterialSymbolsOutlined.ttf').getBestCmap()
  for cp, name in sorted(cmap.items()):
      if name == 'YOUR_ICON_NAME':
          print(f'{name}: U+{cp:04X}')
  "

  # 3. Generate subset (update --unicodes when adding icons)
  #    Current glyphs (39): close, edit, select_all, undo, download, bar_chart,
  #    history, folder, chevron_left, chevron_right, query_stats, cancel,
  #    more_vert, unfold_more, arrow_upward, arrow_downward, arrow_back, sync,
  #    star, check_circle, delete, description, help, hourglass_empty, info,
  #    search, hourglass_top, percent, comment, cleaning_services,
  #    mark_chat_read, monitoring, sticky_note_2, warning, error, palette,
  #    photo_camera, screen_rotation_alt, add_box
  pyftsubset /tmp/MaterialSymbolsOutlined.ttf \
    --unicodes="U+E000,U+E002,U+E0B9,U+E14C,U+E146,U+E150,U+E162,U+E166,U+E171,U+E26B,U+E28E,U+E2C7,U+E408,U+E409,U+E412,U+E40A,U+E4FC,U+E5C4,U+E5C9,U+E5D4,U+E5D7,U+E5D8,U+E5DB,U+E627,U+E838,U+E86C,U+E872,U+E873,U+E887,U+E88B,U+E88E,U+E8B6,U+EA5B,U+EB58,U+EBEE,U+F0FF,U+F18B,U+F190,U+F1FC" \
    --output-file=assets/material-symbols.ttf \
    --layout-features="" --no-hinting --desubroutinize
  ```
  Then add the corresponding `ICON_*` constant in `src/icons.rs`.

  > ⚠ **WARNING**: Material Symbols Outlined and the legacy Material Icons font
  > share glyph *names* but use *different codepoints*. Always look up codepoints
  > from the actual `.ttf` variable font using `fontTools`, never from the legacy
  > `.codepoints` file or Material Icons documentation.

- **`NotoSansSC-Regular.ttf`** is a pyftsubset-generated subset (~7 MB) of the
  full Noto Sans SC variable font (~25 MB). It covers ASCII, CJK punctuation,
  and CJK Unified Ideographs (U+4E00–9FFF). Regenerate with:
  ```sh
  pyftsubset NotoSansSC-full.ttf \
    --output-file=assets/NotoSansSC-Regular.ttf \
    --unicodes="U+0000-007F,U+00B2-00B3,U+2000-206F,U+3000-303F,U+4E00-9FFF,U+FF01-FF5E" \
    --no-hinting --desubroutinize \
    --drop-tables=GPOS,GSUB,GDEF,STAT,fvar,gvar,avar,cvar,HVAR,MVAR
  ```
  Variable-font tables (fvar, gvar, etc.) must be dropped, or the WASM binary
  may crash with a blank page due to cosmic-text parsing failures.

- **`github-mark.png`** uses a transparent background so it blends with any
  theme. The original GitHub mark has a white background; transparency was
  applied via Pillow (`PIL`).
